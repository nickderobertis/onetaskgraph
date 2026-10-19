//! The asset journeys' HTTP boundary: real sockets, whole bodies and GitHub's rolling limiter.
use onetaskgraph_e2e_support::clock::SimulatedClock;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Upload,
    Read,
    Repository,
}

/// What kind of request one the board answers is, by the classes the timing spike measured
/// GitHub's latency in (`board-copy-speed-2026-10-18-spike-copy-timing-report`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A GraphQL read of one item, or of anything else no other class names.
    ItemRead,
    /// A page of the board's own items, [`onetaskgraph_github_projects::graphql::BOARD`].
    BoardPage,
    /// A page of the board-scoped issue search,
    /// [`onetaskgraph_github_projects::graphql::SEARCH_ISSUES`].
    SearchPage,
    /// A GraphQL read whose aliases name several items by id.
    BatchedRead,
    /// A mutation document holding one mutation field.
    Mutation,
    /// A mutation document holding several aliased mutation fields.
    AliasedMutations,
    /// `POST /user-attachments/assets`.
    Upload,
    /// `GET /repos/{owner}/{repo}`, the repository id an upload names.
    Repository,
    /// The authenticated read of an uploaded attachment that verifies it.
    Verification,
}

impl Kind {
    /// The spike report's mean latency for this kind of request, in milliseconds: item read
    /// 554, board page 6,948 and search page 6,791 (both measured on the 1,986-item `plans`
    /// board), mutation 1,082, a request of up to eleven aliased mutations 3,700, upload 927,
    /// its repository read 526, its verification 1,140, and — by the table's note — a read
    /// batching several items 4,551.
    pub const fn measured_millis(self) -> u64 {
        match self {
            Self::ItemRead => 554,
            Self::BoardPage => 6_948,
            Self::SearchPage => 6_791,
            Self::BatchedRead => 4_551,
            Self::Mutation => 1_082,
            Self::AliasedMutations => 3_700,
            Self::Upload => 927,
            Self::Repository => 526,
            Self::Verification => 1_140,
        }
    }

    /// Which kind the request with request line path `path` and body `bytes` is.
    pub fn of(path: &str, bytes: &[u8]) -> Self {
        use onetaskgraph_github_projects::graphql;
        if path.starts_with("/user-attachments/assets?") {
            return Self::Upload;
        }
        if path.starts_with("/user-attachments/assets/") {
            return Self::Verification;
        }
        if path.starts_with("/repos/") {
            return Self::Repository;
        }
        let request: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
        let query = request["query"].as_str().unwrap_or_default();
        if query.trim_start().starts_with("mutation") {
            return if top_level_fields(query) > 1 {
                Self::AliasedMutations
            } else {
                Self::Mutation
            };
        }
        if query == graphql::BOARD {
            return Self::BoardPage;
        }
        if query == graphql::SEARCH_ISSUES {
            return Self::SearchPage;
        }
        let named: std::collections::BTreeSet<&str> = request["variables"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, _)| {
                name.strip_prefix("id").is_some_and(|slot| {
                    !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit())
                })
            })
            .filter_map(|(_, id)| id.as_str())
            .filter(|id| !id.is_empty())
            .collect();
        if named.len() > 1 {
            Self::BatchedRead
        } else {
            Self::ItemRead
        }
    }
}

/// How many fields the operation's own selection set holds — the aliases of a batched
/// mutation — read by brace depth, so a nested selection is never counted.
fn top_level_fields(document: &str) -> usize {
    let Some(open) = document.find('{') else {
        return 0;
    };
    let (mut depth, mut parens, mut fields, mut in_name, mut directive) =
        (1usize, 0usize, 0usize, false, false);
    for character in document[open + 1..].chars() {
        match character {
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '{' if parens == 0 => depth += 1,
            '}' if parens == 0 => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        // A field starts where a name begins at the operation's own depth; an alias and its
        // field (`a1:createIssue`) are one, because the colon never ends a name run there, and
        // a directive's name (`@include`) is none.
        let naming = depth == 1 && parens == 0 && (character.is_alphanumeric() || character == '_');
        if naming && !in_name && !directive {
            fields += 1;
        }
        if character == '@' {
            directive = true;
        } else if !naming && !(in_name && character == ':') {
            directive = false;
        }
        in_name = naming || (in_name && depth == 1 && parens == 0 && character == ':');
    }
    fields
}

#[derive(Clone)]
pub(super) struct Refusal {
    pub stage: Stage,
    pub status: u16,
    pub headers: String,
    pub body: String,
    pub remaining: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub client: usize,
    pub path: String,
    pub headers: String,
    pub bytes: Vec<u8>,
    pub at: Duration,
    pub status: u16,
    pub creating: bool,
    pub kind: Kind,
    /// The virtual time the board released its answer, after holding it for its latency.
    pub answered: Duration,
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    transferred: BTreeMap<String, String>,
    assets: BTreeMap<String, Vec<u8>>,
    refusal: Option<Refusal>,
    disconnect: Option<Stage>,
    truncate: Option<Stage>,
    rolling: VecDeque<Duration>,
    enforce_limit: bool,
    /// Whether each answer is held for the spike's measured latency of its [`Kind`] rather than
    /// 200 ms, or 800 ms for an upload.
    measured: bool,
}

pub(super) struct Board {
    pub endpoint: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl Board {
    pub fn new(
        upstream: &str,
        clock: &SimulatedClock,
        client: usize,
        shared: Option<&Self>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}/graphql");
        let state = shared.map_or_else(
            || Arc::new(Mutex::new(State::default())),
            |board| board.state.clone(),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let held = state.clone();
        let upstream = upstream
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let clock = clock.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let mut stream = stream.unwrap();
                let request = clock.request(client);
                let (headers, bytes, raw) = read_request(&mut stream);
                let first = headers.lines().next().unwrap();
                let path = first.split_whitespace().nth(1).unwrap().to_owned();
                let upload = first.starts_with("POST /user-attachments/assets?");
                let attachment_read = first.starts_with("GET /user-attachments/assets/");
                let graphql = path == "/graphql";
                let creating = upload
                    || (graphql
                        && serde_json::from_slice::<Value>(&bytes).unwrap()["query"]
                            .as_str()
                            .unwrap()
                            .trim_start()
                            .starts_with("mutation"));
                let at = clock.now();
                let mut answer = None;
                let mut disconnect = false;
                let mut truncate = false;
                let mut status = 200;
                let mut extra_headers = String::new();
                let kind = Kind::of(&path, &bytes);
                let measured;
                {
                    let mut state = held.lock().unwrap();
                    measured = state.measured;
                    let stage = if upload {
                        Some(Stage::Upload)
                    } else if attachment_read {
                        Some(Stage::Read)
                    } else if path.starts_with("/repos/") {
                        Some(Stage::Repository)
                    } else {
                        None
                    };
                    if state.disconnect.is_some() && state.disconnect == stage {
                        disconnect = true;
                        state.disconnect = None;
                    }

                    if state.truncate.is_some() && state.truncate == stage {
                        truncate = true;
                        state.truncate = None;
                    }

                    while state.rolling.front().is_some_and(|previous| {
                        at.saturating_sub(*previous) >= Duration::from_secs(60)
                    }) {
                        state.rolling.pop_front();
                    }
                    if creating
                        && state.enforce_limit
                        && state.rolling.len() as u64
                            >= onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE
                    {
                        status = 403;
                        let wait = (state.rolling[0] + Duration::from_secs(60))
                            .saturating_sub(at)
                            .as_secs()
                            + 1;
                        extra_headers = format!("Retry-After: {wait}\r\n");
                        answer = Some(
                            br#"{"message":"You have exceeded a secondary rate limit"}"#.to_vec(),
                        );
                    } else {
                        if creating {
                            state.rolling.push_back(at);
                        }
                        if let Some(refusal) = &mut state.refusal
                            && refusal.remaining > 0
                            && ((upload && refusal.stage == Stage::Upload)
                                || (attachment_read && refusal.stage == Stage::Read)
                                || (path.starts_with("/repos/")
                                    && refusal.stage == Stage::Repository))
                        {
                            refusal.remaining -= 1;
                            status = refusal.status;
                            extra_headers = refusal.headers.clone();
                            answer = Some(refusal.body.as_bytes().to_vec());
                        }
                    }
                    if !disconnect && answer.is_none() && upload {
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer test-token")
                        );
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("content-type: image/png")
                        );
                        let id = state.assets.len() + 1;
                        let key = format!("/user-attachments/assets/{id}");
                        state.assets.insert(key.clone(), bytes.clone());
                        status = 201;
                        answer = Some(
                            serde_json::to_vec(&json!({"url":format!("http://{address}{key}")}))
                                .unwrap(),
                        );
                    } else if !disconnect && answer.is_none() && attachment_read {
                        let authenticated = headers
                            .to_ascii_lowercase()
                            .contains("authorization: bearer test-token");
                        answer = Some(if authenticated {
                            state.assets.get(&path).cloned().unwrap_or_else(|| {
                                status = 404;
                                Vec::new()
                            })
                        } else {
                            status = 404;
                            Vec::new()
                        });
                    } else if !disconnect && answer.is_none() && path.starts_with("/repos/") {
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer test-token")
                        );
                        answer = Some(if path == "/repos/fixture/transferred" {
                            br#"{"id":654321}"#.to_vec()
                        } else {
                            br#"{"id":123456}"#.to_vec()
                        });
                    }
                    if !disconnect {
                        state.calls.push(Call {
                            client,
                            path: path.clone(),
                            headers: headers.clone(),
                            bytes: bytes.clone(),
                            at,
                            status,
                            creating,
                            kind,
                            answered: at,
                        });
                    }
                }
                if disconnect {
                    continue;
                }
                let forwarded = if answer.is_none() {
                    assert!(graphql, "unexpected endpoint {path}");
                    let mut connection = TcpStream::connect(&upstream).unwrap();
                    connection.write_all(&raw).unwrap();
                    let mut response = Vec::new();
                    connection.read_to_end(&mut response).unwrap();
                    let transfers = held.lock().unwrap().transferred.clone();
                    if !transfers.is_empty() {
                        let body = response
                            .windows(4)
                            .position(|part| part == b"\r\n\r\n")
                            .unwrap()
                            + 4;
                        let mut value: Value = serde_json::from_slice(&response[body..]).unwrap();
                        transfer_repositories(&mut value, &transfers);
                        let body = serde_json::to_vec(&value).unwrap();
                        response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        response.extend(body);
                    }
                    Some(response)
                } else {
                    None
                };
                runtime.block_on(request.delay(Duration::from_millis(if measured {
                    kind.measured_millis()
                } else if upload {
                    800
                } else {
                    200
                })));
                let answered = clock.now();
                if let Some(call) = held.lock().unwrap().calls.last_mut() {
                    call.answered = answered;
                }
                if let Some(forwarded) = forwarded {
                    stream.write_all(&forwarded).unwrap();
                } else {
                    let answer = answer.unwrap();
                    write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n", answer.len() + usize::from(truncate)).unwrap();
                    stream.write_all(&answer).unwrap();
                }
                drop(request);
            }
        });
        Self {
            endpoint,
            state,
            stop,
        }
    }
    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }
    /// The bytes this host holds at `path`, as an authenticated read of it would answer.
    pub fn attachment(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().assets.get(path).cloned()
    }
    pub fn transfer_issue(&self, id: &str, repository: &str) {
        self.state
            .lock()
            .unwrap()
            .transferred
            .insert(id.into(), repository.into());
    }
    pub fn refuse(&self, refusal: Refusal) {
        self.state.lock().unwrap().refusal = Some(refusal);
    }
    pub fn disconnect_next(&self, stage: Stage) {
        self.state.lock().unwrap().disconnect = Some(stage);
    }
    pub fn truncate_next(&self, stage: Stage) {
        self.state.lock().unwrap().truncate = Some(stage);
    }
    pub fn enforce_limit(&self) {
        self.state.lock().unwrap().enforce_limit = true;
    }
    /// Hold every answer for the spike's measured latency of its kind,
    /// [`Kind::measured_millis`], instead of 200 ms, or 800 ms for an upload.
    pub fn measure_latency(&self) {
        self.state.lock().unwrap().measured = true;
    }
}
fn transfer_repositories(value: &mut Value, transfers: &BTreeMap<String, String>) {
    match value {
        Value::Object(object) => {
            if let Some(repository) = object
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| transfers.get(id))
                && object.contains_key("repository")
            {
                object.insert("repository".into(), json!({"nameWithOwner": repository}));
            }
            for value in object.values_mut() {
                transfer_repositories(value, transfers);
            }
        }
        Value::Array(values) => {
            for value in values {
                transfer_repositories(value, transfers);
            }
        }
        _ => {}
    }
}
impl Drop for Board {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let address = self
            .endpoint
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let _ = TcpStream::connect(address);
    }
}
fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>, Vec<u8>) {
    let mut raw = Vec::new();
    let mut chunk = [0; 8192];
    let end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before headers");
        raw.extend_from_slice(&chunk[..count]);
        if let Some(end) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8(raw[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(|number| {
                    number
                        .parse::<usize>()
                        .expect("Content-Length must be an unsigned integer")
                })
        })
        .unwrap_or(0);
    while raw.len() < end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before all declared bytes");
        raw.extend_from_slice(&chunk[..count]);
    }
    (headers, raw[end..end + length].to_vec(), raw)
}
