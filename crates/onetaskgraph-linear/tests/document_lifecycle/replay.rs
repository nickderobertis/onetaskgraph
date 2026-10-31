//! Synthetic HTTP replay of the recorded Linear wire shapes, never a source double.
use super::document_lifecycle::journal::Journal;
#[path = "replay_schema.rs"]
pub(crate) mod schema;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
pub struct State {
    pub documents: BTreeMap<String, Value>,
    pub projects: BTreeMap<String, Value>,
    pub issues: BTreeMap<String, Value>,
    /// Labels by name: their id, and how many lookups of that name answer no match before
    /// the label filter holds it — Linear's read-after-write lag; `u32::MAX` never holds it.
    pub labels: BTreeMap<String, (String, u32)>,
    /// How many times each label name has been looked up.
    pub label_lookups: BTreeMap<String, u32>,
    pub requests: Vec<Value>,
    pub uncertain: bool,
    pub pause_next_create: bool,
    pub wrong_team: bool,
    pub direct_team_override: Option<Value>,
    pub omit_direct_team: bool,
    pub unverifiable_team: bool,
    pub corrupt_document_create_ack: bool,
    pub freeze_document_run_metadata: bool,
    pub ambiguous: bool,
    /// Discovery-only page size and deliberate pagination fault, applied over stored rows.
    pub discovery_pagination: Option<&'static str>,
    pub schema_errors: Vec<String>,
    /// Every request as `(Authorization header, body)`, so concurrent sessions sending
    /// different keys can each be told apart in one log.
    pub keyed: Vec<(String, Value)>,
    /// Linear's team document limit, which counts trashed documents as well as live ones:
    /// a `documentCreate` once `documents_ever` has reached it is refused, as Linear refuses it.
    pub document_limit: Option<usize>,
    /// Every document the team has ever held, deleted ones included, seeded with the
    /// documents a real team already holds before a test begins.
    pub documents_ever: usize,
    /// Ids Linear still holds in its trash: a create naming one is refused like a live one.
    pub trashed: std::collections::BTreeSet<String>,
    /// Linear's hourly request quota for one key, which every session of every run shares: a
    /// request past it is refused `RATELIMITED`, as Linear refuses it, and counted here.
    pub request_limit: Option<usize>,
    /// Linear's hourly complexity quota for one key, which every session shares too: a request
    /// whose score — as Linear scores it — would carry the hour past it is refused the same way.
    pub complexity_limit: Option<f64>,
    pub complexity_spent: f64,
    /// Store the next `issueCreate` and then answer it with a gateway error, as a create
    /// whose acknowledgement was lost on the way back.
    pub lose_next_issue_create_ack: bool,
    /// Refuse every `issueDelete` while set.
    pub refuse_issue_deletes: bool,
    /// How the pool's lease listing is misanswered: `"no-page-info"`, `"repeated-cursor"`,
    /// `"untitled"`, or `"unreadable-created-at"`.
    pub lease_listing_fault: Option<&'static str>,
    /// Take a create under an id already held as an overwrite, as a Linear whose ids were not
    /// exclusive would.
    pub accept_duplicate_ids: bool,
    /// Refuse every `issueUpdate` while set.
    pub refuse_issue_updates: bool,
    pub rate_limited: usize,
    serial: usize,
    rewrites: BTreeMap<String, String>,
}
pub struct Replay {
    pub endpoint: String,
    pub state: Arc<Mutex<State>>,
    pub journal: Journal,
    pub paused: Arc<AtomicBool>,
    pub release: Arc<AtomicBool>,
}
fn page(nodes: Vec<Value>) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"hasPreviousPage":false,"endCursor":null,"startCursor":null}})
}
fn matches(row: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return true;
    };
    filter.iter().all(|(field, cmp)| {
        if field == "and" {
            return cmp.as_array().unwrap().iter().all(|f| matches(row, f));
        }
        if field == "or" {
            return cmp.as_array().unwrap().iter().any(|f| matches(row, f));
        }
        if field == "team" {
            return true;
        }
        if field == "comments" {
            return row["comments"].as_array().is_some_and(|comments| {
                comments
                    .iter()
                    .any(|comment| matches(comment, &cmp["some"]))
            });
        }
        let value = &row[field];
        if let Some(cmp) = cmp.as_object() {
            cmp.iter().all(|(op, wanted)| match op.as_str() {
                "eq" => value == wanted,
                "neq" => value != wanted,
                "eqIgnoreCase" => value
                    .as_str()
                    .zip(wanted.as_str())
                    .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)),
                "contains" => value
                    .as_str()
                    .zip(wanted.as_str())
                    .is_some_and(|(a, b)| a.contains(b)),
                "containsIgnoreCase" => value
                    .as_str()
                    .zip(wanted.as_str())
                    .is_some_and(|(a, b)| a.to_lowercase().contains(&b.to_lowercase())),
                "startsWith" => value
                    .as_str()
                    .zip(wanted.as_str())
                    .is_some_and(|(a, b)| a.starts_with(b)),
                "in" => wanted.as_array().is_some_and(|v| v.contains(value)),
                "null" => value.is_null() == wanted.as_bool().unwrap_or(false),
                "some" => value
                    .get("nodes")
                    .and_then(Value::as_array)
                    .is_some_and(|nodes| nodes.iter().any(|n| matches(n, wanted))),
                "gte" => value
                    .as_str()
                    .zip(wanted.as_str())
                    .is_some_and(|(a, b)| a >= b),
                _ => matches(value, &json!({op:wanted})),
            })
        } else {
            true
        }
    })
}
impl Replay {
    pub fn start(journal: Journal, rewrites: BTreeMap<String, String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream = format!("http://{}/graphql", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            rewrites,
            ..State::default()
        }));
        let paused = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let serving_paused = paused.clone();
        let serving_release = release.clone();
        let serving = state.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let (key, request) = read(&mut stream);
                let mut state = serving.lock().unwrap();
                state.keyed.push((key, request.clone()));
                let scored = schema::complexity::score(
                    request["query"].as_str().unwrap_or_default(),
                    &request["variables"],
                )
                .map_or(0.0, |points| {
                    points + schema::complexity::LINEAR_ABOVE_MODEL
                });
                state.complexity_spent += scored;
                let over_complexity = state
                    .complexity_limit
                    .is_some_and(|limit| state.complexity_spent > limit);
                if over_complexity
                    || state
                        .request_limit
                        .is_some_and(|limit| state.keyed.len() > limit)
                {
                    state.rate_limited += 1;
                    drop(state);
                    let body = json!({"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}).to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    continue;
                }
                // Linear scores a document before it reads a row and refuses one over its
                // cap with HTTP 400, so this replay does too, rather than answering it.
                // A request with no query string fails to parse and is refused the same way.
                if let Some(refused) = schema::complexity::refusal(
                    request["query"].as_str().unwrap_or_default(),
                    &request["variables"],
                ) {
                    state.schema_errors.push(refused.clone());
                    state.requests.push(request);
                    drop(state);
                    let body = json!({"errors":[{"message":refused}]}).to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    continue;
                }
                let mut body = respond(&mut state, &request);
                if body.get("errors").is_none()
                    && body.get("__http_status").is_none()
                    && body
                        .pointer("/data/documentCreate/document")
                        .is_none_or(|v| !v.is_null())
                    && let Err(error) = schema::validate(request["query"].as_str().unwrap(), &body)
                {
                    state.schema_errors.push(error.clone());
                    body = json!({"errors":[{"message":error}]});
                }
                // The pool's lease listing, misanswered after it validated.
                let lease_listing = request["query"].as_str().unwrap().contains("issues(")
                    && request
                        .pointer("/variables/filter/title/startsWith")
                        .and_then(Value::as_str)
                        .is_some_and(|prefix| prefix.contains("lease-v1"));
                if let (Some(fault), true, Some(connection)) = (
                    state.lease_listing_fault,
                    lease_listing,
                    body.pointer_mut("/data/issues"),
                ) {
                    match fault {
                        "no-page-info" => {
                            connection.as_object_mut().unwrap().remove("pageInfo");
                        }
                        "untitled" => {
                            for node in connection["nodes"].as_array_mut().unwrap() {
                                node.as_object_mut().unwrap().remove("title");
                            }
                        }
                        "repeated-cursor" => {
                            let after = request["variables"]["after"].as_str().unwrap_or("0");
                            connection["pageInfo"] = json!({"hasNextPage":true,"endCursor":after});
                        }
                        _ => {
                            for node in connection["nodes"].as_array_mut().unwrap() {
                                node["createdAt"] = json!("yesterday");
                            }
                        }
                    }
                }
                // Negative wire journeys corrupt an otherwise schema-validated response.
                // Missing pagination must reach discovery's guard, rather than becoming a
                // schema validator error returned by the fixture server itself.
                if state.discovery_pagination == Some("missing-pagination")
                    && request["query"].as_str().unwrap().contains("documents(")
                    && let Some(connection) = body
                        .pointer_mut("/data/documents")
                        .and_then(Value::as_object_mut)
                {
                    connection.remove("pageInfo");
                }
                if request["query"].as_str().unwrap().contains("documents(")
                    && let Some(nodes) = body
                        .pointer_mut("/data/documents/nodes")
                        .and_then(Value::as_array_mut)
                {
                    for node in nodes {
                        if state.omit_direct_team {
                            node.as_object_mut().unwrap().remove("team");
                        } else if let Some(direct) = &state.direct_team_override {
                            node["team"] = direct.clone();
                        }
                    }
                }
                let pause = state.pause_next_create
                    && request["query"]
                        .as_str()
                        .unwrap()
                        .contains("documentCreate(");
                if pause {
                    state.pause_next_create = false;
                }
                state.requests.push(request);
                drop(state);
                if pause {
                    serving_paused.store(true, Ordering::Release);
                    while !serving_release.load(Ordering::Acquire) {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                if body.get("__http_status").is_some() {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    continue;
                }
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let endpoint =
            super::document_lifecycle::observer::start(&upstream, "synthetic", journal.clone());
        Self {
            endpoint,
            state,
            journal,
            paused,
            release,
        }
    }
}
fn read(stream: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
        assert!(bytes.len() < 2_000_000);
        if let Some(p) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..p]).to_lowercase();
            let len: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            if bytes.len() >= p + 4 + len {
                let key = String::from_utf8_lossy(&bytes[..p])
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                            .map(|(_, value)| value.trim().to_owned())
                    })
                    .unwrap_or_default();
                return (
                    key,
                    serde_json::from_slice(&bytes[p + 4..p + 4 + len]).unwrap(),
                );
            }
        }
    }
}
fn respond(state: &mut State, request: &Value) -> Value {
    let q = request["query"].as_str().unwrap();
    let v = &request["variables"];
    graphql_parser::parse_query::<String>(q).expect("replay requests are GraphQL documents");
    if (state.refuse_issue_deletes && q.contains("issueDelete("))
        || (state.refuse_issue_updates && q.contains("issueUpdate("))
    {
        return json!({"errors":[{"message":"Something went wrong","extensions":{"code":"INTERNAL_SERVER_ERROR"}}]});
    }
    let teams = json!({"nodes":[{"id":"team","key":"ENG","states":{"nodes":[{"id":"open","name":"Todo","type":"unstarted","position":0},{"id":"done","name":"Done","type":"completed","position":1}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}]});
    if q.contains("issueLabels(") {
        let name = v["name"].as_str().unwrap().to_lowercase();
        let lookups = state.label_lookups.entry(name.clone()).or_default();
        *lookups += 1;
        let seen = *lookups;
        let nodes = state
            .labels
            .iter()
            .filter(|(held, (_, lag))| held.to_lowercase() == name && seen > *lag)
            .map(|(_, (id, _))| json!({"id":id}))
            .collect();
        return json!({"data":{"issueLabels":page(nodes)}});
    }
    if q.contains("teams(") && !q.contains("projects(") && !q.contains("documents(") {
        return json!({"data":{"organization":{"id":"synthetic-organization"},"teams":teams,"projectStatuses":{"nodes":[{"id":"status","name":"Planned","type":"planned","position":0}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    }
    for (root, field) in [
        ("documentCreate", "document"),
        ("documentUpdate", "document"),
        ("projectCreate", "project"),
        ("issueCreate", "issue"),
        ("issueUpdate", "issue"),
    ] {
        if q.contains(&format!("{root}(")) {
            let create = root.ends_with("Create");
            let keyed = v["input"]["id"].as_str().map(str::to_owned);
            if create
                && !state.accept_duplicate_ids
                && let Some(id) = &keyed
                && (state.documents.contains_key(id)
                    || state.projects.contains_key(id)
                    || state.issues.contains_key(id)
                    || state.trashed.contains(id))
            {
                // Linear refuses a create under an id it already holds, trashed or not.
                return json!({"errors":[{"message":"Entity already exists","extensions":{"code":"INVALID_INPUT"}}]});
            }
            if root == "documentCreate"
                && let Some(limit) = state.document_limit
                && state.documents_ever >= limit
            {
                return json!({"errors":[{"message":"Usage limit exceeded: the team has reached its document limit","extensions":{"code":"USAGE_LIMIT_EXCEEDED"}}]});
            }
            let id = if let (true, Some(id)) = (create, &keyed) {
                id.clone()
            } else if create {
                state.serial += 1;
                format!("{field}-{}", state.serial)
            } else {
                v["id"].as_str().unwrap().to_owned()
            };
            if root == "documentCreate" {
                state.documents_ever += 1;
            }
            let table = match field {
                "document" => &mut state.documents,
                "project" => &mut state.projects,
                _ => &mut state.issues,
            };
            let template: Value = serde_json::from_str(if field == "document" {
                include_str!("../fixtures/documents.json")
            } else if field == "project" {
                include_str!("../fixtures/projects.json")
            } else {
                include_str!("../fixtures/issues.json")
            })
            .unwrap();
            let mut row = table.get(&id).cloned().unwrap_or_else(|| {
                if field == "document" {
                    template["data"]["documents"]["nodes"][0].clone()
                } else if field == "project" {
                    template["data"]["projects"]["nodes"][0].clone()
                } else {
                    template["data"]["issues"]["nodes"][0].clone()
                }
            });
            row["id"] = json!(id);
            row["archivedAt"] = Value::Null;
            row["url"] = json!(format!("https://linear.app/test/{field}/{id}"));
            row["team"] = json!({"key":"ENG"});
            row["teams"] = json!({"nodes":[{"key":"ENG"}],"pageInfo":{"hasNextPage":false}});
            if create {
                row["createdAt"] = json!(chrono::Utc::now().to_rfc3339());
                row["project"] = Value::Null;
                row["labels"] = page(vec![]);
                row["relations"] = page(vec![]);
                row["comments"] = json!([]);
            }
            for (key, value) in v["input"].as_object().unwrap() {
                match key.as_str() {
                    "teamId" | "teamIds" | "id" => {}
                    "projectId" => {
                        row["project"] = if value.is_null() {
                            Value::Null
                        } else {
                            json!({"id":value,"teams":{"nodes":[{"key":"ENG"}],"pageInfo":{"hasNextPage":false}}})
                        }
                    }
                    "stateId" => row["state"] = json!({"name":"Todo","type":"unstarted"}),
                    "content" | "description" => {
                        if root == "documentUpdate"
                            && state.freeze_document_run_metadata
                            && value
                                .as_str()
                                .is_some_and(|body| body.contains("caller.run"))
                        {
                            continue;
                        }
                        row[key] = value
                            .as_str()
                            .and_then(|s| state.rewrites.get(s))
                            .map(|s| json!(s))
                            .unwrap_or_else(|| value.clone())
                    }
                    _ => row[key] = value.clone(),
                }
            }
            table.insert(id, row.clone());
            if root == "issueCreate" && state.lose_next_issue_create_ack {
                state.lose_next_issue_create_ack = false;
                return json!({"__http_status": 502});
            }
            if root == "documentCreate" && state.uncertain {
                state.uncertain = false;
                return json!({"data":{root:{"success":true,"document":null}}});
            }
            if root == "documentCreate" && state.corrupt_document_create_ack {
                row["content"] =
                    json!("Synthetic changed body in a successful create acknowledgement.");
            }
            return json!({"data":{root:{"success":true,field:row}}});
        }
    }
    for root in ["documentDelete", "projectDelete", "issueDelete"] {
        if q.contains(&format!("{root}(")) {
            let id = v["id"].as_str().unwrap();
            let table = match root {
                "documentDelete" => &mut state.documents,
                "projectDelete" => &mut state.projects,
                _ => &mut state.issues,
            };
            if table.remove(id).is_some() {
                state.trashed.insert(id.to_owned());
            }
            return json!({"data":{root:{"success":true}}});
        }
    }
    if q.contains("commentCreate(") {
        let id = v["input"]["issueId"].as_str().unwrap();
        let mut comment: Value =
            serde_json::from_str(include_str!("../fixtures/comments.json")).unwrap();
        comment = comment["data"]["issue"]["comments"]["nodes"][0].clone();
        comment["body"] = v["input"]["body"].clone();
        comment["createdAt"] = json!(chrono::Utc::now().to_rfc3339());
        comment["updatedAt"] = comment["createdAt"].clone();
        state.issues.get_mut(id).unwrap()["comments"]
            .as_array_mut()
            .unwrap()
            .push(comment.clone());
        return json!({"data":{"commentCreate":{"success":true,"comment":comment}}});
    }
    for (root, table) in [
        ("document", &state.documents),
        ("project", &state.projects),
        ("issue", &state.issues),
    ] {
        if q.contains(&format!("{root}(")) {
            let row = table
                .get(v["id"].as_str().unwrap())
                .cloned()
                .unwrap_or(Value::Null);
            return json!({"data":{root:row}});
        }
    }
    for (root, table) in [
        ("documents", &state.documents),
        ("projects", &state.projects),
        ("issues", &state.issues),
    ] {
        if q.contains(&format!("{root}(")) {
            let mut nodes: Vec<_> = table
                .values()
                .filter(|row| matches(row, &v["filter"]))
                .cloned()
                .collect();
            if state.wrong_team {
                for row in &mut nodes {
                    row["team"] = json!({"key":"OTHER"});
                    row["teams"] = json!({"nodes":[],"pageInfo":{"hasNextPage":true}});
                }
            }
            if state.unverifiable_team {
                for row in &mut nodes {
                    row["team"] = Value::Null;
                    row["project"] = json!({"teams":{"nodes":[],"pageInfo":{"hasNextPage":true}}});
                    row["teams"] = json!({"nodes":[],"pageInfo":{"hasNextPage":true}});
                }
            }
            if state.ambiguous
                && let Some(first) = nodes.first().cloned()
            {
                nodes.push(first);
            }
            let connection = if root == "documents" {
                if let Some(mode) = state.discovery_pagination {
                    let index = v["after"]
                        .as_str()
                        .map(|cursor| cursor.parse::<usize>().expect("synthetic cursor"))
                        .unwrap_or(0);
                    let next = index + 1;
                    let more = mode == "exhausted" || next < nodes.len();
                    let rows = nodes.into_iter().skip(index).take(1).collect();
                    let mut connection = page(rows);
                    connection["pageInfo"]["hasNextPage"] = json!(more);
                    connection["pageInfo"]["endCursor"] = if mode == "missing-cursor" {
                        Value::Null
                    } else if mode == "repeated-cursor" {
                        json!("1")
                    } else if more {
                        json!(next.to_string())
                    } else {
                        Value::Null
                    };
                    connection
                } else {
                    page(nodes)
                }
            } else {
                page(nodes)
            };
            return json!({"data":{root:connection}});
        }
    }
    panic!("unhandled synthetic replay query: {q}")
}
