//! The asset journeys' HTTP boundary: real sockets, whole bodies and GitHub's rolling limiter.
use onetaskgraph_e2e_support::clock::SimulatedClock;
use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_until};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
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
    /// A GraphQL read whose aliases name several items by id, or ask about several origins.
    BatchedRead,
    /// A mutation document sending one mutation field.
    Mutation,
    /// A mutation document sending several aliased mutation fields.
    AliasedMutations,
    /// `POST /user-attachments/assets`.
    Upload,
    /// `GET /repos/{owner}/{repo}`, the repository id an upload names.
    Repository,
    /// The authenticated read of an uploaded attachment that verifies it.
    Verification,
}

impl Kind {
    /// The spike report's mean latency for this kind of request, in milliseconds — its
    /// "Latency per request kind" table, the board and search pages as measured on the
    /// 1,986-item `plans` board, and a batched read charged as the table's note says.
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
            return if top_level_fields(query, &request["variables"]) > 1 {
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
        // The read of where GitHub placed the issues a copy created names several ids, but
        // selects each one's board item and status alone: 0.11's own trace of it on the real
        // board (`docs/measurements/board-copy-speed-2026-10-18/after/tb-first-span-20261024T095836`)
        // answered in 212 to 385 ms, an item read's time rather than a batched detail read's.
        if query == graphql::CREATED_BOARD_ITEMS {
            return Self::ItemRead;
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
        // A read asking about several origins at once is a batch as much as one naming several
        // ids is: it is priced as the spike's batched read.
        let origins = request["variables"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, asked)| {
                name.strip_prefix("ask").is_some_and(|slot| {
                    !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit())
                }) && **asked == Value::Bool(true)
            })
            .count();
        if named.len() > 1 || origins > 1 {
            Self::BatchedRead
        } else {
            Self::ItemRead
        }
    }
}

/// How many fields the operation's own selection set sends — the aliases of a batched
/// mutation — read by brace depth, so a nested selection is never counted, and leaving out each
/// field an `@include(if:$name)` whose variable is false, or a `@skip(if:$name)` whose variable
/// is true, keeps from being sent.
fn top_level_fields(document: &str, variables: &Value) -> usize {
    let sent = |name: &str| variables.get(name).and_then(Value::as_bool);
    let mut excluded = 0;
    for (directive, keeps) in [("@include(if:$", true), ("@skip(if:$", false)] {
        for (at, _) in document.match_indices(directive) {
            let rest = &document[at + directive.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // Only a directive on a field of the operation's own selection set leaves one out.
            let depth = document[..at].matches('{').count() - document[..at].matches('}').count();
            if depth == 1 && sent(&name).is_some_and(|value| value != keeps) {
                excluded += 1;
            }
        }
    }
    selected_fields(document).saturating_sub(excluded)
}

/// Every field of the operation's own selection set, sent or not.
fn selected_fields(document: &str) -> usize {
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

/// A refusal of the GraphQL requests whose document contains `matching`, answered `remaining`
/// times with `status`, `headers` and `body` and never forwarded to the board behind.
#[derive(Clone)]
pub(super) struct GraphqlRefusal {
    pub matching: &'static str,
    pub status: u16,
    pub headers: String,
    pub body: String,
    pub remaining: usize,
}

/// The content creations one request is charged against GitHub's secondary windows: each
/// mutation field a GraphQL request sends, and one for an upload.
pub(super) fn creations(path: &str, bytes: &[u8]) -> u64 {
    if path.starts_with("/user-attachments/assets?") {
        return 1;
    }
    let request: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let query = request["query"].as_str().unwrap_or_default();
    if query.trim_start().starts_with("mutation") {
        top_level_fields(query, &request["variables"]) as u64
    } else {
        0
    }
}

/// The secondary points one request is charged: a GraphQL request with a mutation five, one
/// without one, and a REST request none.
pub(super) fn secondary_points(path: &str, bytes: &[u8]) -> u64 {
    if path != "/graphql" {
        return 0;
    }
    let request: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    if request["query"]
        .as_str()
        .unwrap_or_default()
        .trim_start()
        .starts_with("mutation")
    {
        onetaskgraph_github_projects::SECONDARY_POINTS_PER_MUTATION
    } else {
        onetaskgraph_github_projects::SECONDARY_POINTS_PER_QUERY
    }
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
    /// The wall-clock time the request arrived, since the Unix epoch: what orders it against
    /// what happened outside the board — a boundary command the binary ran — which the
    /// simulated clock, standing still while the binary works, cannot.
    pub wall: Duration,
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    transferred: BTreeMap<String, String>,
    assets: BTreeMap<String, Vec<u8>>,
    refusal: Option<Refusal>,
    disconnect: Option<Stage>,
    truncate: Option<Stage>,
    /// Each content creation the board counted, at the time its request arrived.
    creations: VecDeque<Duration>,
    /// Each GraphQL request's secondary points, at the time it arrived.
    points: VecDeque<(Duration, u64)>,
    /// Whether GitHub's three secondary windows are enforced: a request that would take one
    /// past its limit is refused as GitHub refuses it, with a `Retry-After`.
    enforce_limit: bool,
    /// How long every answer is held, when a journey says so.
    latency: Option<Duration>,
    graphql_refusal: Option<GraphqlRefusal>,
    /// Whether each answer is held for the spike's measured latency of its [`Kind`] rather than
    /// 200 ms, or 800 ms for an upload.
    measured: bool,
    /// Told the virtual time each answer was released at, once it is.
    answered: Option<Arc<dyn Fn(Duration) + Send + Sync>>,
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
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        // The binary keeps its connection to this proxy open, so the proxy keeps one to the
        // board behind it too, rather than a connection per request on that side.
        let mut upstream = Upstream::new(upstream.parse().expect("the board's own address"));
        serve_until(listener, Arc::clone(&stopping), move |sent, stream| {
            if stopping.load(Ordering::SeqCst) {
                return After::Close;
            }
            let request = clock.request(client);
            let (headers, bytes) = (sent.head().to_owned(), sent.body().to_vec());
            let raw = sent.bytes();
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

                let minute = Duration::from_secs(60);
                let hour = Duration::from_secs(3_600);
                while state
                    .creations
                    .front()
                    .is_some_and(|previous| at.saturating_sub(*previous) >= hour)
                {
                    state.creations.pop_front();
                }
                while state
                    .points
                    .front()
                    .is_some_and(|(previous, _)| at.saturating_sub(*previous) >= minute)
                {
                    state.points.pop_front();
                }
                let charged = creations(&path, &bytes);
                let priced = secondary_points(&path, &bytes);
                let in_minute = state
                    .creations
                    .iter()
                    .filter(|previous| at.saturating_sub(**previous) < minute)
                    .count() as u64;
                let in_hour = state.creations.len() as u64;
                let points: u64 = state.points.iter().map(|(_, points)| points).sum();
                let over = charged > 0
                    && (in_minute + charged
                        > onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE
                        || in_hour + charged
                            > onetaskgraph_github_projects::CONTENT_CREATION_PER_HOUR)
                    || priced > 0
                        && points + priced
                            > onetaskgraph_github_projects::SECONDARY_POINTS_PER_MINUTE;
                let refused_graphql = graphql
                    && state.graphql_refusal.as_ref().is_some_and(|refusal| {
                        refusal.remaining > 0
                            && String::from_utf8_lossy(&bytes).contains(refusal.matching)
                    });
                if over && state.enforce_limit {
                    status = 403;
                    extra_headers = "Retry-After: 60\r\n".to_owned();
                    answer =
                        Some(br#"{"message":"You have exceeded a secondary rate limit"}"#.to_vec());
                } else if refused_graphql {
                    let refusal = state.graphql_refusal.as_mut().expect("armed");
                    refusal.remaining -= 1;
                    status = refusal.status;
                    extra_headers = refusal.headers.clone();
                    answer = Some(refusal.body.as_bytes().to_vec());
                } else {
                    for _ in 0..charged {
                        state.creations.push_back(at);
                    }
                    if priced > 0 {
                        state.points.push_back((at, priced));
                    }
                    if let Some(refusal) = &mut state.refusal
                        && refusal.remaining > 0
                        && ((upload && refusal.stage == Stage::Upload)
                            || (attachment_read && refusal.stage == Stage::Read)
                            || (path.starts_with("/repos/") && refusal.stage == Stage::Repository))
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
                        wall: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default(),
                    });
                }
            }
            if disconnect {
                return After::Close;
            }
            let forwarded = if answer.is_none() {
                assert!(graphql, "unexpected endpoint {path}");
                let mut response = upstream.forward(&raw);
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
                    response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                    response.extend(body);
                }
                Some(response)
            } else {
                None
            };
            let latency = held.lock().unwrap().latency;
            runtime.block_on(request.delay(latency.unwrap_or(Duration::from_millis(
                if measured {
                    kind.measured_millis()
                } else if upload {
                    800
                } else {
                    200
                },
            ))));
            let answered = clock.now();
            let told = {
                let mut state = held.lock().unwrap();
                if let Some(call) = state.calls.last_mut() {
                    call.answered = answered;
                }
                state.answered.clone()
            };
            if let Some(told) = told {
                told(answered);
            }
            // A board that closed its connection unanswered is relayed as just that.
            let after = if forwarded.as_ref().is_some_and(Vec::is_empty) || truncate {
                After::Close
            } else {
                After::KeepOpen
            };
            if let Some(forwarded) = forwarded {
                stream.write_all(&forwarded).unwrap();
            } else {
                let answer = answer.unwrap();
                // A body cut short is followed by the connection closing, as it is on a
                // real host, so the client reads it as cut short rather than waiting.
                let closing = if truncate {
                    "Connection: close\r\n"
                } else {
                    ""
                };
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\n{closing}{extra_headers}\r\n", answer.len() + usize::from(truncate)).unwrap();
                stream.write_all(&answer).unwrap();
            }
            drop(request);
            after
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
    /// Enforce GitHub's secondary limits over their windows, as GitHub does at a request's
    /// arrival: at most 80 content creations in any minute and 500 in any hour, and 2,000
    /// secondary points in any minute.
    pub fn enforce_limit(&self) {
        self.state.lock().unwrap().enforce_limit = true;
    }
    /// Hold every answer for `latency`, whatever its kind.
    pub fn hold_answers_for(&self, latency: Duration) {
        self.state.lock().unwrap().latency = Some(latency);
    }
    /// Refuse the GraphQL requests `refusal` matches, as many times as it says.
    pub fn refuse_graphql(&self, refusal: GraphqlRefusal) {
        self.state.lock().unwrap().graphql_refusal = Some(refusal);
    }
    /// Tell `answered` the virtual time each answer is released at, once it is: what a board
    /// placing created issues some time after their create was answered keeps that time by.
    pub fn on_answered(&self, answered: impl Fn(Duration) + Send + Sync + 'static) {
        self.state.lock().unwrap().answered = Some(Arc::new(answered));
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
