//! The source's own suite, driven over a real loopback socket against a board fixture.
//!
//! Nothing here mocks the layer under test: every test builds the plugin through
//! `SourcePlugin::build`, and every request it makes is a real HTTP POST carrying a real
//! GraphQL document, answered by a fixture that keeps board state the way GitHub does.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, ItemKind, ItemWrite, Label, LabelFilter, NativeId, PageRequest, Project,
    ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskSource, TextFields, TextQuery,
    WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, var: &str) -> Option<SecretString> {
        (var == "GH_PROJECTS_TOKEN").then(|| "test-token".into())
    }
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

fn resume(cursor: &str, limit: u32) -> PageRequest {
    PageRequest {
        cursor: Some(Cursor(cursor.to_owned())),
        limit,
    }
}

/// One issue or draft as the fixture holds it.
#[derive(Clone)]
struct Item {
    item_id: String,
    content_id: String,
    typename: &'static str,
    title: String,
    body: Option<String>,
    state: &'static str,
    state_reason: Option<String>,
    parent: Option<String>,
    sub_issues: u64,
    repository: Option<&'static str>,
    labels: Vec<(&'static str, &'static str)>,
    status: Option<String>,
    origin: Option<String>,
}

impl Item {
    fn issue(id: &str, title: &str) -> Self {
        Self {
            item_id: format!("PVTI_{id}"),
            content_id: id.to_owned(),
            typename: "Issue",
            title: title.to_owned(),
            body: None,
            state: "OPEN",
            state_reason: None,
            parent: None,
            sub_issues: 0,
            repository: Some("acme/work"),
            labels: vec![],
            status: None,
            origin: None,
        }
    }
    fn draft(id: &str, title: &str) -> Self {
        Self {
            typename: "DraftIssue",
            repository: None,
            ..Self::issue(id, title)
        }
    }
    fn pull_request(id: &str) -> Self {
        Self {
            typename: "PullRequest",
            ..Self::issue(id, "a change")
        }
    }
    fn body(mut self, body: &str) -> Self {
        self.body = Some(body.to_owned());
        self
    }
    fn status(mut self, status: &str) -> Self {
        self.status = Some(status.to_owned());
        self
    }
    fn parent(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_owned());
        self
    }
    fn sub_issues(mut self, total: u64) -> Self {
        self.sub_issues = total;
        self
    }
    fn closed(mut self, reason: Option<&str>) -> Self {
        self.state = "CLOSED";
        self.state_reason = reason.map(str::to_owned);
        self
    }
    fn labelled(mut self, labels: &[(&'static str, &'static str)]) -> Self {
        self.labels = labels.to_vec();
        self
    }

    fn field_values(&self, options: &Value) -> Value {
        let mut nodes = Vec::new();
        if let Some(status) = &self.status {
            nodes.push(
                json!({"name":status,"field":{"id":"FIELD_status","name":"Status","options":options}}),
            );
        }
        nodes.push(
            json!({"text":self.origin.clone().unwrap_or_default(),"field":{"id":"FIELD_origin","name":"onetaskgraph.origin"}}),
        );
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }

    fn content(&self) -> Value {
        match self.typename {
            "PullRequest" => json!({"__typename":"PullRequest","id":self.content_id}),
            "DraftIssue" => json!({"__typename":"DraftIssue","id":self.content_id,
                "title":self.title,"body":self.body,"createdAt":null,"updatedAt":null}),
            _ => json!({"__typename":"Issue","id":self.content_id,"title":self.title,
                "body":self.body.clone().unwrap_or_default(),
                "url":format!("https://github.example/{}", self.content_id),
                "createdAt":null,"updatedAt":null,"state":self.state,
                "stateReason":self.state_reason,
                "repository":self.repository.map(|r| json!({"nameWithOwner":r})),
                "parent":self.parent.as_ref().map(|id| json!({"id":id})),
                "subIssuesSummary":{"total":self.sub_issues},
                "labels":{"nodes":self.labels.iter().map(|(id,name)| json!({"id":id,"name":name,"color":null})).collect::<Vec<_>>(),
                          "pageInfo":{"hasNextPage":false}}}),
        }
    }
}

/// Everything the fixture remembers between requests.
struct State {
    items: Vec<Item>,
    /// Issues created but not yet added to the board.
    pending: Vec<Item>,
    options: Vec<(&'static str, &'static str)>,
    origin_field: bool,
    status_field: bool,
    blocked_by: BTreeMap<String, Vec<String>>,
    /// Mutations this board answers with a GraphQL error rather than performing. GitHub
    /// fails one call of the several a write is, and what the source does about the calls
    /// that already landed is only readable if one of them can be made to fail.
    refuses: BTreeSet<String>,
    seen: Vec<Value>,
    next: usize,
    /// GitHub's two rate limiters, as far as a board fixture can spell them.
    limits: Limits,
}

/// One canned HTTP refusal, in the exact shape GitHub answers a rate limit with.
#[derive(Clone)]
struct Refusal {
    status: &'static str,
    headers: String,
    body: String,
}

impl Refusal {
    /// A secondary rate limit under a forbidden status, which is how GitHub answers it far
    /// more often than with too-many-requests.
    fn secondary_forbidden() -> Self {
        Self {
            status: "403 Forbidden",
            headers: String::new(),
            body: json!({"message":"You have exceeded a secondary rate limit and have been \
                                   temporarily blocked from content creation. Please retry \
                                   your request again later.",
                         "documentation_url":"https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api"})
                .to_string(),
        }
    }
    /// The same refusal, asking for a wait of `seconds`.
    fn after(mut self, seconds: u64) -> Self {
        self.headers = format!("retry-after: {seconds}\r\n");
        self
    }
}

/// What this board refuses for going too fast, and what it counts while it does.
#[derive(Default)]
struct Limits {
    /// Canned refusals to answer the next requests with, oldest first.
    scripted: Vec<Refusal>,
    /// Canned refusals to answer the next requests *carrying one operation* with.
    scripted_for: BTreeMap<String, Vec<Refusal>>,
    /// Shortest interval this board accepts between two mutations. Anything faster is
    /// refused the way GitHub refuses a secondary rate limit.
    min_mutation_interval: Option<Duration>,
    /// Refuse every mutation with a secondary rate limit, however slowly it arrives.
    refuse_every_mutation: bool,
    /// Operations this board answers normally, with the budget reported as spent — which
    /// is what GitHub sends on the last request a budget allows.
    spends_the_budget: BTreeSet<String>,
    /// When the last mutation arrived, for the interval above.
    last_mutation: Option<Instant>,
    /// How many mutations this board refused for arriving too fast.
    too_fast: u32,
    /// When each request arrived, which operation it carried, and whether that operation
    /// creates content.
    arrivals: Vec<(Instant, String, bool)>,
}

impl Limits {
    /// The refusal this request earns, or `None` to let the board answer it.
    fn refusal(&mut self, operation: &str, mutation: bool) -> Option<Refusal> {
        let now = Instant::now();
        self.arrivals.push((now, operation.to_owned(), mutation));
        if let Some(scripted) = self
            .scripted_for
            .get_mut(operation)
            .filter(|scripted| !scripted.is_empty())
        {
            return Some(scripted.remove(0));
        }
        if !self.scripted.is_empty() {
            return Some(self.scripted.remove(0));
        }
        if !mutation {
            return None;
        }
        if self.refuse_every_mutation {
            return Some(Refusal::secondary_forbidden());
        }
        let too_fast = self.min_mutation_interval.is_some_and(|interval| {
            self.last_mutation
                .is_some_and(|last| now.duration_since(last) < interval)
        });
        self.last_mutation = Some(now);
        if too_fast {
            self.too_fast += 1;
            return Some(Refusal::secondary_forbidden());
        }
        None
    }
}

impl State {
    fn options(&self) -> Value {
        Value::Array(
            self.options
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect(),
        )
    }
    fn fields(&self) -> Value {
        let mut nodes = Vec::new();
        if self.status_field {
            nodes.push(
                json!({"__typename":"ProjectV2SingleSelectField","id":"FIELD_status",
                              "name":"Status","options":self.options()}),
            );
        }
        if self.origin_field {
            nodes.push(
                json!({"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}),
            );
        }
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }
    fn find(&mut self, content_id: &Value) -> &mut Item {
        let wanted = content_id.as_str().expect("a content id");
        self.items
            .iter_mut()
            .find(|item| item.content_id == wanted)
            .expect("the fixture holds the item being written")
    }
}

/// A running board fixture.
struct Fixture {
    endpoint: String,
    state: Arc<Mutex<State>>,
}

impl Fixture {
    /// The mutation inputs the source sent, in order, as `[operation, input]` pairs.
    fn seen(&self) -> Vec<Value> {
        self.state.lock().unwrap().seen.clone()
    }
    fn item(&self, content_id: &str) -> Item {
        self.state
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item.content_id == content_id)
            .expect("the fixture holds that item")
            .clone()
    }
    /// Whether this board still holds an item, which is a different question from
    /// `item` — one asserts on what it carries, this on whether it is there at all.
    fn holds(&self, content_id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .items
            .iter()
            .any(|item| item.content_id == content_id)
    }
    /// Fail this mutation from here on, the way GitHub fails one call part way through a
    /// write: everything before it has landed, and nothing after it runs.
    fn refuse(&self, operation: &str) {
        self.state
            .lock()
            .unwrap()
            .refuses
            .insert(operation.to_owned());
    }
    /// Answer the next requests with these canned HTTP refusals, oldest first.
    fn script(&self, refusals: Vec<Refusal>) {
        self.state.lock().unwrap().limits.scripted = refusals;
    }
    /// Answer the next requests carrying `operation` with these canned refusals, oldest
    /// first. A write is several calls, so refusing one of them by name is the only way to
    /// say *which* the limiter caught.
    fn script_for(&self, operation: &str, refusals: Vec<Refusal>) {
        self.state
            .lock()
            .unwrap()
            .limits
            .scripted_for
            .insert(operation.to_owned(), refusals);
    }
    /// Refuse any mutation arriving less than `interval` after the one before it, the way
    /// GitHub's secondary limiter refuses a burst of content creation.
    fn rate_limit_mutations(&self, interval: Duration) {
        self.state.lock().unwrap().limits.min_mutation_interval = Some(interval);
    }
    /// Refuse every mutation with a secondary rate limit, however slowly it arrives.
    fn refuse_every_mutation(&self) {
        self.state.lock().unwrap().limits.refuse_every_mutation = true;
    }
    /// Answer `operation` normally, reporting the budget spent — which is what GitHub
    /// sends on the last request a budget allows, not only on the ones it then refuses.
    fn spend_the_budget_on(&self, operation: &str) {
        self.state
            .lock()
            .unwrap()
            .limits
            .spends_the_budget
            .insert(operation.to_owned());
    }
    /// How many mutations this board refused for arriving faster than it allows.
    fn too_fast(&self) -> u32 {
        self.state.lock().unwrap().limits.too_fast
    }
    /// How many requests carried `operation`, refused ones included.
    fn requests(&self, operation: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .limits
            .arrivals
            .iter()
            .filter(|(_, seen, _)| seen == operation)
            .count()
    }
    /// The gaps between consecutive arrivals of any content-creating mutation.
    fn mutation_gaps(&self) -> Vec<Duration> {
        let state = self.state.lock().unwrap();
        let times = state
            .limits
            .arrivals
            .iter()
            .filter(|(_, _, mutation)| *mutation)
            .map(|(at, _, _)| *at)
            .collect::<Vec<_>>();
        times.windows(2).map(|pair| pair[1] - pair[0]).collect()
    }
}

fn board(items: Vec<Item>) -> Fixture {
    board_with(items, true, true)
}

fn board_with(items: Vec<Item>, status_field: bool, origin_field: bool) -> Fixture {
    let state = Arc::new(Mutex::new(State {
        items,
        pending: Vec::new(),
        options: vec![
            ("OPT_backlog", "Backlog"),
            ("OPT_todo", "Todo"),
            ("OPT_doing", "In Progress"),
            ("OPT_shipped", "Shipped"),
        ],
        origin_field,
        status_field,
        blocked_by: BTreeMap::new(),
        refuses: BTreeSet::new(),
        seen: Vec::new(),
        next: 0,
        limits: Limits::default(),
    }));
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let served = Arc::clone(&state);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("fixture connection");
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("a GraphQL document");
            graphql_parser::parse_query::<String>(query).expect("a valid GraphQL document");
            let variables = &request["variables"];
            // The limiter answers before the board does, exactly as GitHub's does: a
            // refused request never reaches the board and changes nothing on it.
            let limited = served
                .lock()
                .unwrap()
                .limits
                .refusal(operation_name(query), is_mutation(query));
            let spent = served
                .lock()
                .unwrap()
                .limits
                .spends_the_budget
                .contains(operation_name(query));
            let (status, headers, body) = match limited {
                Some(refusal) => (refusal.status, refusal.headers, refusal.body),
                None => (
                    "200 OK",
                    if spent {
                        "x-ratelimit-remaining: 0\r\n".to_owned()
                    } else {
                        String::new()
                    },
                    match refused(&served, query, variables) {
                        Some(message) => json!({"errors":[{"message":message}]}).to_string(),
                        None => json!({ "data": answer(&served, query, variables) }).to_string(),
                    },
                ),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("a response");
        }
    });
    Fixture { endpoint, state }
}

/// The GraphQL error a refused mutation answers with, or `None` to perform it.
///
/// A refused call is still recorded as seen and still changes nothing: that is what GitHub
/// failing one mutation of a write looks like from here.
fn refused(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Option<String> {
    let mut state = state.lock().unwrap();
    let operation = operation_name(query);
    if !state.refuses.contains(operation) {
        return None;
    }
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state.seen.push(json!([operation, input]));
    }
    Some(format!("{operation} is refused by this board"))
}

fn answer(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Value {
    let mut state = state.lock().unwrap();
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state
            .seen
            .push(json!([operation_name(query), input.clone()]));
    }
    if query.contains("repository(owner:$owner,name:$name)") {
        return if variables["name"] == "missing" {
            json!({ "repository": null })
        } else {
            json!({"repository":{"id":"REPO_1","nameWithOwner":format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())}})
        };
    }
    if query.contains("deleteIssue(input:$input)") {
        let id = input["issueId"].as_str().expect("an issue id").to_owned();
        state.items.retain(|item| item.content_id != id);
        return json!({"deleteIssue":{"repository":{"id":"REPO_1"}}});
    }
    if query.contains("createIssue(input:$input)") {
        state.next += 1;
        let id = format!("I_new{}", state.next);
        let mut created = Item::issue(&id, input["title"].as_str().unwrap_or_default());
        created.body = input["body"].as_str().map(str::to_owned);
        state.pending.push(created);
        return json!({"createIssue":{"issue":{"id":id}}});
    }
    if query.contains("addProjectV2ItemById(input:$input)") {
        let content = input["contentId"]
            .as_str()
            .expect("a content id")
            .to_owned();
        let position = state
            .pending
            .iter()
            .position(|item| item.content_id == content)
            .expect("the issue was created first");
        let item = state.pending.remove(position);
        let item_id = item.item_id.clone();
        state.items.push(item);
        return json!({"addProjectV2ItemById":{"item":{"id":item_id}}});
    }
    if query.contains("updateIssue(input:$input)") {
        let item = state.find(&input["id"]);
        if let Some(title) = input["title"].as_str() {
            item.title = title.to_owned();
        }
        if input.get("body").is_some() {
            item.body = input["body"].as_str().map(str::to_owned);
        }
        if let Some(state_input) = input.get("stateInput").filter(|value| !value.is_null()) {
            item.state = if state_input["value"] == "CLOSED" {
                "CLOSED"
            } else {
                "OPEN"
            };
            item.state_reason = state_input["stateReason"].as_str().map(str::to_owned);
        }
        return json!({"updateIssue":{"issue":{"id":input["id"]}}});
    }
    if query.contains("updateProjectV2DraftIssue(input:$input)") {
        let item = state.find(&input["draftIssueId"]);
        item.title = input["title"].as_str().unwrap_or_default().to_owned();
        item.body = input["body"].as_str().map(str::to_owned);
        return json!({"updateProjectV2DraftIssue":{"draftIssue":{"id":input["draftIssueId"]}}});
    }
    if query.contains("updateProjectV2ItemFieldValue(input:$input)") {
        let item_id = input["itemId"].as_str().unwrap().to_owned();
        let option = input["value"]["singleSelectOptionId"]
            .as_str()
            .and_then(|id| {
                state
                    .options
                    .iter()
                    .find(|(known, _)| *known == id)
                    .map(|(_, name)| (*name).to_owned())
            });
        let text = input["value"]["text"].as_str().map(str::to_owned);
        let item = state
            .items
            .iter_mut()
            .find(|item| item.item_id == item_id)
            .expect("a field update names a board item");
        if let Some(option) = option {
            item.status = Some(option);
        }
        if let Some(text) = text {
            item.origin = Some(text);
        }
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id}}});
    }
    if query.contains("addSubIssue(input:$input)") || query.contains("removeSubIssue(input:$input)")
    {
        let adding = query.contains("addSubIssue");
        let parent = input["issueId"].as_str().unwrap().to_owned();
        let child = input["subIssueId"].clone();
        state.find(&child).parent = adding.then(|| parent.clone());
        let held = state
            .items
            .iter()
            .filter(|item| item.parent.as_deref() == Some(parent.as_str()))
            .count() as u64;
        if let Some(item) = state
            .items
            .iter_mut()
            .find(|item| item.content_id == parent)
        {
            item.sub_issues = held;
        }
        let root = if adding {
            "addSubIssue"
        } else {
            "removeSubIssue"
        };
        return json!({root:{"issue":{"id":parent},"subIssue":{"id":child}}});
    }
    if query.contains("addBlockedBy(input:$input)")
        || query.contains("removeBlockedBy(input:$input)")
    {
        let adding = query.contains("addBlockedBy");
        let issue = input["issueId"].as_str().unwrap().to_owned();
        let blocker = input["blockingIssueId"].as_str().unwrap().to_owned();
        let edges = state.blocked_by.entry(issue.clone()).or_default();
        if adding {
            edges.push(blocker.clone());
        } else {
            edges.retain(|held| held != &blocker);
        }
        let root = if adding {
            "addBlockedBy"
        } else {
            "removeBlockedBy"
        };
        return json!({root:{"issue":{"id":issue},"blockingIssue":{"id":blocker}}});
    }
    if query.contains("node(id:$id)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let related = |ids: Vec<String>| {
            Value::Array(
                ids.into_iter()
                    .map(|id| {
                        let far = state.items.iter().find(|item| item.content_id == id);
                        json!({"id":id,
                               "body":far.and_then(|item| item.body.clone()),
                               "parent":far.and_then(|item| item.parent.clone()).map(|id| json!({"id":id})),
                               "subIssuesSummary":{"total":far.map_or(0, |item| item.sub_issues)}})
                    })
                    .collect(),
            )
        };
        let blocked = state.blocked_by.get(&id).cloned().unwrap_or_default();
        let blocking = state
            .blocked_by
            .iter()
            .filter(|(_, blockers)| blockers.contains(&id))
            .map(|(issue, _)| issue.clone())
            .collect::<Vec<_>>();
        return json!({"node":{"__typename":"Issue",
            "blockedBy":{"nodes":related(blocked),"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":related(blocking),"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    }
    assert!(
        query.contains("projectV2(number:$number)"),
        "the fixture received an unknown operation: {query}"
    );
    assert_eq!(variables["duplicates"], json!(true));
    let offset = match &variables["after"] {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
        other => panic!("after must be null or a string: {other}"),
    };
    let first = variables["first"].as_u64().expect("first") as usize;
    let end = (offset + first).min(state.items.len());
    let options = state.options();
    let nodes = state.items[offset..end]
        .iter()
        .map(|item| {
            json!({"id":item.item_id,"fieldValues":item.field_values(&options),"content":item.content()})
        })
        .collect::<Vec<_>>();
    json!({"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap","fields":state.fields(),
        "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < state.items.len(),"endCursor":end.to_string()}}}}})
}

/// Whether this document creates content, which is what GitHub's secondary limiter counts.
fn is_mutation(query: &str) -> bool {
    query.trim_start().starts_with("mutation")
}

/// The operation this document carries, read out of the document itself.
///
/// Derived rather than matched against a list of the operations this source sends: a list
/// here would be a second copy of the production mutation inventory, and a mutation added
/// there and not here would go uncounted and unnamed in silence — which is exactly what
/// the pacing assertions below measure.
fn operation_name(query: &str) -> &str {
    let body = query
        .split_once('{')
        .map_or(query, |(_, rest)| rest)
        .trim_start();
    let root = &body[..body
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(body.len())];
    // The two reads answer to what they read rather than to their GraphQL root, because
    // `owner` and `node` say nothing about what a test is counting. Nothing is enumerated:
    // every mutation's name is the one its own document spells.
    match root {
        "owner" => "board",
        "node" => "issueDependencies",
        other => other,
    }
}

fn read_http_json(stream: &mut impl Read) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("a fixture request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header terminator")
        + 4;
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(headers.contains("authorization: Bearer test-token"));
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .expect("a content length");
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("a request body");
        assert!(count > 0, "the request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).expect("request JSON")
}

fn raw_server(status: &str, body: &str) -> String {
    raw_server_with_headers(status, body, "")
}

fn raw_server_with_headers(status: &str, body: &str, headers: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (body, status, headers) = (body.to_owned(), status.to_owned(), headers.to_owned());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

fn sequence_server(bodies: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

/// A source over one fixture endpoint.
///
/// Pacing is switched off in the base configuration on purpose. The shipped default
/// spaces a mutation every 750 ms against github.com, and every write test here would
/// otherwise spend that per call proving something about status mapping or metadata. The
/// tests that are about pacing and about waiting a limiter out say so themselves, by
/// passing a `pacing` block of their own — so what is proven about the schedule is proven
/// where it is the subject, and `the_shipped_pacing_defaults_are_githubs_published_limits`
/// pins the shipped values against the ones a source built with no `pacing` block uses.
fn configured(endpoint: &str, extra: Value) -> Box<dyn TaskSource> {
    let mut config = json!({"owner":"octo-org","project_number":7,"endpoint":endpoint,
                            "repository":"acme/work",
                            "pacing":{"min_mutation_interval_ms":0,"retry_budget_ms":0}});
    for (key, value) in extra.as_object().expect("an object of overrides") {
        if value.is_null() {
            config.as_object_mut().unwrap().remove(key);
        } else {
            config[key] = value.clone();
        }
    }
    Plugin
        .build(&SourceName::new("work").unwrap(), &config, &Secrets)
        .expect("a usable configuration")
}

use onetaskgraph_github_projects::Plugin;

fn source(fixture: &Fixture) -> Box<dyn TaskSource> {
    configured(&fixture.endpoint, json!({}))
}

fn refusal(error: SourceError) -> String {
    error.to_string()
}

/// The refusal a configuration that cannot be built answers with.
fn build_refusal(config: Value) -> String {
    match Plugin.build(&SourceName::new("work").unwrap(), &config, &Secrets) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("{config} was supposed to be refused"),
    }
}

fn task(id: &str, title: &str, status: Status) -> Task {
    Task {
        id: NativeId(id.to_owned()),
        title: title.to_owned(),
        content: None,
        status,
        labels: vec![],
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

fn project(id: &str, title: &str, status: Status) -> Project {
    Project {
        id: NativeId(id.to_owned()),
        title: title.to_owned(),
        content: None,
        status,
        labels: vec![],
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

fn status(category: StatusCategory, name: &str) -> Status {
    Status {
        category,
        name: name.to_owned(),
    }
}

fn write<T>(item: T) -> ItemWrite<T> {
    ItemWrite {
        target: None,
        item,
        depends_on: vec![],
    }
}

#[tokio::test]
async fn the_committed_board_fixture_maps_to_two_projects_their_tasks_an_orphan_and_no_pull_request()
 {
    // The committed fixture is the drift artifact the pinned-schema test validates, so it
    // is read here through a real socket rather than paraphrased.
    let fixture: Value = serde_json::from_str(include_str!("fixtures/project.json")).unwrap();
    let endpoint = raw_server("200 OK", &fixture.to_string());
    let source = configured(&endpoint, json!({}));

    let projects = source
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .expect("the board lists its projects");
    assert_eq!(
        projects
            .items
            .iter()
            .map(|project| project.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_plan", "I_next"],
        "an issue with sub-issues and an issue carrying the kind marker are both projects"
    );
    assert_eq!(
        projects.items[0].content.as_deref(),
        Some("the delivery plan")
    );
    assert_eq!(projects.items[0].metadata["caller.enabled"], json!(true));
    assert_eq!(
        projects.items[0].repositories,
        vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()]
    );
    assert_eq!(
        projects.items[0].status,
        status(StatusCategory::InProgress, "In Progress")
    );

    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists its tasks");
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_task", "I_notes", "I_loose"],
        "each project's own sub-issue is a task, so is the issue under no parent, and the \
         pull request is neither"
    );
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.project.as_ref().map(|id| id.0.as_str()))
            .collect::<Vec<_>>(),
        [Some("I_plan"), Some("I_next"), None],
        "the board holds a task under each of its two projects and one under neither"
    );
    let one = &tasks.items[0];
    assert_eq!(one.project, Some(NativeId("I_plan".to_owned())));
    assert_eq!(one.content.as_deref(), Some("details"));
    assert_eq!(one.metadata["caller.number"], json!(7));
    assert_eq!(
        one.metadata["onetaskgraph.origin"],
        json!("notes:T-1"),
        "the copy origin is kept in a field of its own, not in the body slot"
    );
    assert!(
        !one.metadata.contains_key(ItemKind::METADATA_KEY),
        "the kind marker is this source's own encoding and never travels as metadata"
    );
    assert_eq!(
        one.labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["bug", "team"]
    );
}

/// The committed board fixture, served over a real socket.
///
/// It holds two projects, a task under each of them, a task under neither, and a pull
/// request, so one board answers every shape of the project filter.
fn committed_board() -> Box<dyn TaskSource> {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/project.json")).unwrap();
    configured(&raw_server("200 OK", &fixture.to_string()), json!({}))
}

async fn selected_tasks(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    source
        .query_tasks(query, &page(10))
        .await
        .expect("the board answers a task query")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect()
}

async fn selected_projects(source: &dyn TaskSource, query: &ProjectQuery) -> Vec<String> {
    source
        .query_projects(query, &page(10))
        .await
        .expect("the board answers a project query")
        .items
        .into_iter()
        .map(|project| project.id.0)
        .collect()
}

fn label_filter(any_of: &[&str], all_of: &[&str], none_of: &[&str]) -> LabelFilter {
    let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    LabelFilter {
        any_of: owned(any_of),
        all_of: owned(all_of),
        none_of: owned(none_of),
    }
}

fn text(terms: &str, fields: TextFields) -> Option<TextQuery> {
    Some(TextQuery {
        terms: terms.to_owned(),
        fields,
    })
}

#[tokio::test]
async fn a_task_read_scoped_to_one_project_returns_only_that_projects_tasks() {
    // This source declares `projects` native, so the engine pushes the filter down and
    // applies nothing of its own. Ignoring it here returned every task on the board, which
    // is how a second plan on one board corrupted the first.
    let source = committed_board();

    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_plan".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_task"],
        "a board holding a second project must not answer with that project's tasks"
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_next".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_notes"]
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Orphans,
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_loose"],
        "a task under no parent is the one a project-less selection keeps"
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_nothing".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        Vec::<String>::new(),
        "a project the board does not hold selects nothing rather than everything"
    );
    assert_eq!(
        selected_tasks(source.as_ref(), &TaskQuery::default()).await,
        ["I_task", "I_notes", "I_loose"],
        "an unconstrained query still answers with the whole board"
    );
}

#[tokio::test]
async fn every_predicate_a_task_query_carries_is_applied() {
    let source = committed_board();
    let query =
        |labels: LabelFilter, statuses: Vec<StatusCategory>, text: Option<TextQuery>| TaskQuery {
            text,
            labels,
            statuses,
            project: ProjectFilter::Any,
        };
    let none = LabelFilter::default();

    // `I_loose` carries `chore` and `I_task` carries `bug`, so a label filter that is
    // applied and one that is dropped answer with different rows. Under a board where both
    // carried `bug` the two answers were the same list.
    for (expected, query) in [
        (
            vec!["I_task"],
            query(label_filter(&["bug"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_loose"],
            query(label_filter(&["bug", "chore"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_notes"],
            query(label_filter(&["bug", "docs"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task"],
            query(label_filter(&[], &["bug", "team"], &[]), vec![], None),
        ),
        (
            vec!["I_notes", "I_loose"],
            query(label_filter(&[], &[], &["bug"]), vec![], None),
        ),
        (
            vec![],
            query(label_filter(&["bug"], &[], &["bug"]), vec![], None),
        ),
        // Names match case-insensitively, the way the local Markdown source matches them.
        (
            vec!["I_task"],
            query(label_filter(&["BUG"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_loose"],
            query(none.clone(), vec![StatusCategory::Todo], None),
        ),
        (
            vec!["I_notes"],
            query(none.clone(), vec![StatusCategory::InProgress], None),
        ),
        (
            vec!["I_task", "I_notes", "I_loose"],
            query(
                none.clone(),
                vec![StatusCategory::Todo, StatusCategory::InProgress],
                None,
            ),
        ),
        (
            vec!["I_loose"],
            query(none.clone(), vec![], text("sweep", TextFields::Title)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("filed", TextFields::Title)),
        ),
        (
            vec!["I_loose"],
            query(none.clone(), vec![], text("filed", TextFields::Content)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("sweep", TextFields::Content)),
        ),
        (
            vec!["I_notes"],
            query(
                none.clone(),
                vec![],
                text("QUARTER", TextFields::TitleOrContent),
            ),
        ),
        (
            vec!["I_task"],
            query(none.clone(), vec![], text("sHIP", TextFields::Title)),
        ),
        // Every predicate at once narrows rather than widens.
        (
            vec!["I_loose"],
            TaskQuery {
                text: text("work", TextFields::Content),
                labels: label_filter(&["chore"], &[], &["team"]),
                statuses: vec![StatusCategory::Todo],
                project: ProjectFilter::Orphans,
            },
        ),
    ] {
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            expected,
            "{query:?}"
        );
    }
}

#[tokio::test]
async fn every_predicate_a_project_query_carries_is_applied() {
    let fixture = board(vec![
        Item::issue("P_engine", "Engine plan")
            .body("runtime work")
            .status("Todo")
            .sub_issues(1)
            .labelled(&[("L_core", "core")]),
        Item::issue("P_docs", "Docs plan")
            .body("prose work")
            .status("In Progress")
            .sub_issues(1)
            .labelled(&[("L_chore", "chore")]),
        Item::issue("I_task", "a task").status("Todo"),
    ]);
    let source = source(&fixture);
    let query = |labels: LabelFilter, statuses: Vec<StatusCategory>, text: Option<TextQuery>| {
        ProjectQuery {
            text,
            labels,
            statuses,
        }
    };
    let none = LabelFilter::default();

    for (expected, query) in [
        (
            vec!["P_engine", "P_docs"],
            query(none.clone(), vec![], None),
        ),
        (
            vec!["P_engine"],
            query(label_filter(&["core"], &[], &[]), vec![], None),
        ),
        (
            vec![],
            query(label_filter(&[], &["core", "chore"], &[]), vec![], None),
        ),
        (
            vec!["P_docs"],
            query(label_filter(&[], &[], &["core"]), vec![], None),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![StatusCategory::InProgress], None),
        ),
        (
            vec!["P_engine", "P_docs"],
            query(
                none.clone(),
                vec![StatusCategory::Todo, StatusCategory::InProgress],
                None,
            ),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![], text("docs", TextFields::Title)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("prose", TextFields::Title)),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![], text("prose", TextFields::Content)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("docs", TextFields::Content)),
        ),
        (
            vec!["P_engine", "P_docs"],
            query(
                none.clone(),
                vec![],
                text("work", TextFields::TitleOrContent),
            ),
        ),
        (
            vec!["P_docs"],
            ProjectQuery {
                text: text("plan", TextFields::Title),
                labels: label_filter(&["chore"], &[], &[]),
                statuses: vec![StatusCategory::InProgress],
            },
        ),
    ] {
        assert_eq!(
            selected_projects(source.as_ref(), &query).await,
            expected,
            "{query:?}"
        );
    }
}

#[tokio::test]
async fn a_filtered_task_or_project_result_is_paged_after_it_is_filtered() {
    // Paging the board and then filtering the page would answer this walk with one item
    // and then stop: `I_2` would consume the first page and leave nothing in it.
    let fixture = board(
        (1..=5)
            .map(|n| {
                let item = Item::issue(&format!("I_{n}"), "step").status("Todo");
                if n % 2 == 1 {
                    item.labelled(&[("L_keep", "keep")])
                } else {
                    item.labelled(&[("L_drop", "drop")])
                }
            })
            .chain((1..=3).map(|n| {
                let item = Item::issue(&format!("P_{n}"), "plan")
                    .status("Todo")
                    .sub_issues(1);
                if n % 2 == 1 {
                    item.labelled(&[("L_keep", "keep")])
                } else {
                    item.labelled(&[("L_drop", "drop")])
                }
            }))
            .collect(),
    );
    let source = source(&fixture);
    let query = TaskQuery {
        labels: label_filter(&["keep"], &[], &[]),
        ..TaskQuery::default()
    };

    let first = source.query_tasks(&query, &page(2)).await.unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_1", "I_3"],
        "a page of a filtered result is a page of the survivors"
    );
    assert!(first.next.is_some(), "a third survivor is still owed");

    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let answered = source.query_tasks(&query, &request).await.unwrap();
        walked.extend(answered.items.into_iter().map(|task| task.id.0));
        match answered.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        walked,
        ["I_1", "I_3", "I_5"],
        "a walk to exhaustion returns every survivor exactly once in a stable order"
    );

    let projects = ProjectQuery {
        labels: label_filter(&["keep"], &[], &[]),
        ..ProjectQuery::default()
    };
    let first = source.query_projects(&projects, &page(1)).await.unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|project| project.id.0.as_str())
            .collect::<Vec<_>>(),
        ["P_1"]
    );
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let answered = source.query_projects(&projects, &request).await.unwrap();
        walked.extend(answered.items.into_iter().map(|project| project.id.0));
        match answered.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        walked,
        ["P_1", "P_3"],
        "a page smaller than the surviving projects walks to exhaustion over survivors"
    );
}

#[tokio::test]
async fn a_pull_request_is_neither_a_project_nor_a_task() {
    // A behaviour change: this source used to map every `ProjectV2Item` content shape it
    // recognised into a task, so a pull request on the board was listed as one. A pull
    // request is somebody's change rather than a unit of plan, and it now appears in
    // neither listing and cannot be fetched by either id.
    let fixture = board(vec![
        Item::issue("I_1", "a task"),
        Item::pull_request("PR_1"),
    ]);
    let source = source(&fixture);
    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap();
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_1"]
    );
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        source
            .get_task(&NativeId("PR_1".to_owned()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        source
            .get_project(&NativeId("PR_1".to_owned()))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn every_arm_of_the_project_or_task_rule_decides_the_same_way() {
    let marker = |kind: &str| {
        format!("<!-- onetaskgraph.metadata\n{{\"onetaskgraph.item_kind\":\"{kind}\"}}\n-->")
    };
    let fixture = board(vec![
        // Sub-issues and no marker: a project a person authored by hand.
        Item::issue("I_subs", "authored plan").sub_issues(2),
        // A marker and no sub-issues: the empty project a copy passes through.
        Item::issue("I_marked", "empty plan").body(&marker("project")),
        // Neither: an ordinary task.
        Item::issue("I_plain", "a task"),
        // A sub-issue that has sub-issues of its own AND claims to be a project. Being a
        // sub-issue wins, and no marker overrides it.
        Item::issue("I_deep", "a deep task")
            .parent("I_subs")
            .sub_issues(3)
            .body(&marker("project")),
        // A marker saying `task` carries no information the sub-issue rules did not
        // already decide, so an unmarked-looking task stays a task.
        Item::issue("I_said_task", "a marked task").body(&marker("task")),
    ]);
    let source = source(&fixture);
    assert_eq!(
        source
            .query_projects(&ProjectQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|project| project.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_subs", "I_marked"]
    );
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|task| task.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_plain", "I_deep", "I_said_task"]
    );
    assert_eq!(
        source
            .get_task(&NativeId("I_deep".to_owned()))
            .await
            .unwrap()
            .expect("a sub-issue is a task")
            .project,
        Some(NativeId("I_subs".to_owned()))
    );
}

#[tokio::test]
async fn a_malformed_kind_marker_is_refused_by_name() {
    let fixture = board(vec![Item::issue("I_1", "a task").body(
        "<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"epic\"}\n-->",
    )]);
    let message = refusal(
        source(&fixture)
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a marker this contract cannot read is refused"),
    );
    assert!(message.contains("onetaskgraph.item_kind"), "{message}");
    assert!(message.contains("I_1"), "{message}");
}

#[tokio::test]
async fn unbounded_caller_metadata_and_long_prose_round_trip_through_the_body_slot() {
    // The board's own `shortDescription` is capped at 300 characters and a project text
    // field is length-bounded, which is why neither is where this goes.
    let goal = "g".repeat(400);
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = project(
        "P-source",
        "Published roadmap",
        status(StatusCategory::Todo, "Todo"),
    );
    item.content = Some(goal.clone());
    item.metadata = BTreeMap::from([
        ("caller.shape".to_owned(), json!({"nested":[1, true, null]})),
        ("caller.number".to_owned(), json!(3.5)),
        ("caller.text".to_owned(), json!("plain")),
        ("onepipeline.steps".to_owned(), json!(["a".repeat(500)])),
    ]);
    let written = source
        .write_project(&write(item.clone()))
        .await
        .expect("a project longer than any GitHub text field copies");
    let read = source
        .get_project(&written)
        .await
        .unwrap()
        .expect("the created project reads back");
    assert_eq!(read.content.as_deref(), Some(goal.as_str()));
    assert_eq!(read.metadata, item.metadata);
    assert!(
        fixture.item(&written.0).body.unwrap().ends_with("\n-->"),
        "the slot is a trailing Markdown comment, which GitHub does not render"
    );
}

#[tokio::test]
async fn a_comment_that_is_not_at_the_end_is_the_authors_own_content() {
    let fixture = board(vec![Item::issue("I_1", "a task").body(
        "<!-- onetaskgraph.metadata\n{\"caller.x\":1}\n-->\n\nand then more prose",
    )]);
    let held = source(&fixture)
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert!(held.metadata.is_empty());
    assert!(held.content.unwrap().contains("more prose"));
}

#[tokio::test]
async fn a_slot_this_source_cannot_read_is_refused_rather_than_dropped() {
    for (body, problem) in [
        (
            "<!-- onetaskgraph.metadata\n{\"caller.x\":1}",
            "unterminated",
        ),
        ("<!-- onetaskgraph.metadata\nnot json\n-->", "invalid"),
    ] {
        let fixture = board(vec![Item::issue("I_1", "a task").body(body)]);
        let message = refusal(
            source(&fixture)
                .get_task(&NativeId("I_1".to_owned()))
                .await
                .expect_err("a slot this source cannot read is refused"),
        );
        assert!(message.contains(problem), "{message}");
    }
}

#[tokio::test]
async fn a_write_without_a_configured_repository_is_refused_naming_the_field() {
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"repository": null}));
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a board has no repository of its own"),
    );
    assert!(message.contains("repository"), "{message}");
    assert!(message.contains("owner/name"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    assert!(
        fixture.seen().is_empty(),
        "nothing is written before the refusal"
    );
}

#[tokio::test]
async fn a_repository_the_token_cannot_see_is_refused_by_name() {
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"repository":"acme/missing"}));
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a repository nothing resolves is refused"),
    );
    assert!(message.contains("acme"), "{message}");
    assert!(message.contains("missing"), "{message}");
}

#[tokio::test]
async fn repositories_are_derived_from_the_issue_and_recorded_only_when_they_differ() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let own = Repository::try_from("github.com/acme/work".to_owned()).unwrap();
    let elsewhere = Repository::try_from("github.com/acme/other".to_owned()).unwrap();

    let mut derived = task("T-1", "Derived", status(StatusCategory::Todo, "Todo"));
    derived.repositories = vec![own.clone()];
    let derived_id = source.write_task(&write(derived)).await.unwrap();
    assert!(
        !fixture
            .item(&derived_id.0)
            .body
            .unwrap_or_default()
            .contains(Repository::METADATA_KEY),
        "a list that is exactly the issue's own repository is derived, never written down"
    );
    assert_eq!(
        source
            .get_task(&derived_id)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![own.clone()]
    );

    let mut recorded = task("T-2", "Recorded", status(StatusCategory::Todo, "Todo"));
    recorded.repositories = vec![elsewhere.clone(), own.clone()];
    let recorded_id = source.write_task(&write(recorded)).await.unwrap();
    assert!(
        fixture
            .item(&recorded_id.0)
            .body
            .unwrap()
            .contains(Repository::METADATA_KEY)
    );
    assert_eq!(
        source
            .get_task(&recorded_id)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![elsewhere, own],
        "a plan node naming its own repositories is reported as it named them"
    );
}

#[tokio::test]
async fn the_shipped_mapping_puts_each_category_where_it_says_it_does() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    for (category, name, expected_option, expected_state) in [
        (StatusCategory::Backlog, "Backlog", Some("Backlog"), "OPEN"),
        (StatusCategory::Todo, "Todo", Some("Todo"), "OPEN"),
        (
            StatusCategory::InProgress,
            "In Progress",
            Some("In Progress"),
            "OPEN",
        ),
        (StatusCategory::Done, "Done", None, "CLOSED"),
        (StatusCategory::Cancelled, "Cancelled", None, "CLOSED"),
    ] {
        let id = source
            .write_task(&write(task("T", "one", status(category, name))))
            .await
            .unwrap_or_else(|error| panic!("{category:?}: {error}"));
        let held = fixture.item(&id.0);
        assert_eq!(held.status.as_deref(), expected_option, "{category:?}");
        assert_eq!(held.state, expected_state, "{category:?}");
    }
    let closed = fixture
        .seen()
        .into_iter()
        .filter(|call| call[0] == "updateIssue")
        .map(|call| call[1]["stateInput"]["stateReason"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        closed,
        vec![json!("COMPLETED"), json!("NOT_PLANNED")],
        "done is precisely COMPLETED and cancelled is precisely NOT_PLANNED"
    );
}

#[tokio::test]
async fn done_closes_by_default_and_an_override_puts_it_back_on_a_column() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"done":"Shipped"}}),
    );
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Done, "Shipped"),
        )))
        .await
        .unwrap();
    let held = fixture.item(&id.0);
    assert_eq!(held.state, "OPEN", "an overridden done stays a column");
    assert_eq!(held.status.as_deref(), Some("Shipped"));
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().status,
        status(StatusCategory::Done, "Shipped")
    );
}

#[tokio::test]
async fn a_disabled_status_is_refused_naming_the_status_and_the_instance() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"backlog":null}}),
    );
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "one",
                status(StatusCategory::Backlog, "Backlog"),
            )))
            .await
            .expect_err("a disabled status cannot be written"),
    );
    assert!(message.contains("backlog"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    assert!(message.contains("status_mapping"), "{message}");
    assert!(fixture.seen().is_empty(), "nothing is written first");
}

#[tokio::test]
async fn draft_is_refused_because_a_draft_issue_cannot_have_sub_issues() {
    let fixture = board(vec![]);
    let message = refusal(
        source(&fixture)
            .write_task(&write(task(
                "T-1",
                "one",
                status(StatusCategory::Draft, "Draft"),
            )))
            .await
            .expect_err("draft is disabled by the shipped mapping"),
    );
    assert!(message.contains("draft"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    assert!(message.contains("sub-issue"), "{message}");
}

#[tokio::test]
async fn a_status_the_board_cannot_represent_is_refused_naming_the_status_and_the_instance() {
    for (extra, missing) in [
        (json!({"status_mapping":{"todo":"Nowhere"}}), "Nowhere"),
        (json!({}), "Backlog"),
    ] {
        let fixture = board_with(vec![], !extra.as_object().unwrap().is_empty(), true);
        let source = configured(&fixture.endpoint, extra.clone());
        let category = if extra.as_object().unwrap().is_empty() {
            (StatusCategory::Backlog, "Backlog")
        } else {
            (StatusCategory::Todo, "Todo")
        };
        let message = refusal(
            source
                .write_task(&write(task("T-1", "one", status(category.0, category.1))))
                .await
                .expect_err("a status the board cannot hold is refused"),
        );
        assert!(message.contains(missing), "{message}");
        assert!(message.contains("work"), "the instance is named: {message}");
        assert!(fixture.seen().is_empty(), "nothing is written first");
    }
}

#[tokio::test]
async fn a_closed_issue_reports_the_closed_category_and_its_column_name() {
    let fixture = board(vec![
        Item::issue("I_done", "shipped")
            .closed(Some("COMPLETED"))
            .status("Shipped"),
        Item::issue("I_bare", "shipped").closed(Some("COMPLETED")),
        Item::issue("I_cancelled", "dropped").closed(Some("NOT_PLANNED")),
        Item::issue("I_duplicate", "again")
            .closed(Some("DUPLICATE"))
            .status("Shipped"),
        Item::issue("I_reopened", "odd").closed(Some("REOPENED")),
        Item::issue("I_legacy", "old").closed(None),
        Item::issue("I_open", "doing").status("In Progress"),
    ]);
    let source = source(&fixture);
    async fn read(source: &dyn TaskSource, id: &str) -> Status {
        source
            .get_task(&NativeId(id.to_owned()))
            .await
            .unwrap()
            .unwrap()
            .status
    }
    assert_eq!(
        read(source.as_ref(), "I_done").await,
        status(StatusCategory::Done, "Shipped"),
        "the closed state decides the category and the option decides the name"
    );
    assert_eq!(
        read(source.as_ref(), "I_bare").await,
        status(StatusCategory::Done, "Done")
    );
    assert_eq!(
        read(source.as_ref(), "I_cancelled").await,
        status(StatusCategory::Cancelled, "Cancelled")
    );
    assert_eq!(
        read(source.as_ref(), "I_duplicate").await,
        status(StatusCategory::Unknown, "Shipped"),
        "a closed-as-duplicate task is not finished work, whatever column it sits in"
    );
    assert_eq!(
        read(source.as_ref(), "I_reopened").await,
        status(StatusCategory::Unknown, "Closed")
    );
    assert_eq!(
        read(source.as_ref(), "I_legacy").await,
        status(StatusCategory::Done, "Done")
    );
    assert_eq!(
        read(source.as_ref(), "I_open").await,
        status(StatusCategory::InProgress, "In Progress"),
        "an open issue reads both from its option"
    );
}

#[tokio::test]
async fn writing_a_non_terminal_status_reopens_a_closed_issue_so_a_copy_settles() {
    let fixture = board(vec![
        Item::issue("I_1", "shipped")
            .closed(Some("COMPLETED"))
            .status("Shipped"),
    ]);
    let source = source(&fixture);
    let mut back = task("T-1", "shipped", status(StatusCategory::Todo, "Todo"));
    back.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: back.clone(),
            depends_on: vec![],
        })
        .await
        .expect("a non-terminal status is writable over a closed issue");
    assert_eq!(fixture.item("I_1").state, "OPEN");
    let read = source
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.status,
        status(StatusCategory::Todo, "Todo"),
        "without the reopen this would read Unknown and a copy would report a change forever"
    );
    assert_eq!(read.title, back.title);
}

#[tokio::test]
async fn an_unknown_status_category_key_names_the_instance() {
    let message = build_refusal(json!({"owner":"octo-org","project_number":7,
        "endpoint":"https://api.github.com/graphql",
        "status_mapping":{"shipped":"Shipped"}}));
    assert!(message.contains("shipped"), "{message}");
    assert!(message.contains("work"), "{message}");
    assert!(message.contains("in-progress"), "{message}");
}

#[tokio::test]
async fn two_categories_cannot_share_one_board_option() {
    let message = build_refusal(json!({"owner":"octo-org","project_number":7,
        "endpoint":"https://api.github.com/graphql",
        "status_mapping":{"todo":"Backlog"}}));
    assert!(message.contains("Backlog"), "{message}");
    assert!(message.contains("todo"), "{message}");
    assert!(message.contains("backlog"), "{message}");
}

#[tokio::test]
async fn a_blank_option_name_and_a_malformed_target_are_refused() {
    for mapping in [json!({"todo":""}), json!({"todo":{"closed":"maybe"}})] {
        assert!(
            Plugin
                .build(
                    &SourceName::new("work").unwrap(),
                    &json!({"owner":"octo-org","project_number":7,
                            "endpoint":"https://api.github.com/graphql",
                            "status_mapping":mapping}),
                    &Secrets,
                )
                .is_err(),
            "{mapping}"
        );
    }
}

#[tokio::test]
async fn a_project_copy_creates_an_issue_files_its_tasks_under_it_and_never_writes_the_board() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("a project is created as an issue");
    let mut child = task("T-1", "First step", status(StatusCategory::Todo, "Todo"));
    child.project = Some(plan.clone());
    let filed = source.write_task(&write(child)).await.unwrap();

    assert_eq!(
        source
            .get_project(&plan)
            .await
            .unwrap()
            .expect("the empty project was readable before it had a task")
            .title,
        "Published roadmap"
    );
    assert_eq!(
        source.get_task(&filed).await.unwrap().unwrap().project,
        Some(plan.clone())
    );
    assert_eq!(fixture.item(&plan.0).sub_issues, 1);
    assert!(
        fixture
            .seen()
            .iter()
            .all(|call| call[0] != "updateProjectV2"),
        "nothing this source does writes the board's own fields"
    );
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addSubIssue" && call[1]["issueId"] == plan.0.as_str())
    );
}

#[tokio::test]
async fn a_task_moved_between_projects_leaves_the_one_it_came_from() {
    let fixture = board(vec![
        Item::issue("I_a", "plan a").sub_issues(1),
        Item::issue("I_b", "plan b")
            .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->"),
        Item::issue("I_task", "a step").parent("I_a").status("Todo"),
    ]);
    let source = source(&fixture);
    let mut moved = task("T", "a step", status(StatusCategory::Todo, "Todo"));
    moved.project = Some(NativeId("I_b".to_owned()));
    moved.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_task".to_owned())),
            item: moved,
            depends_on: vec![],
        })
        .await
        .unwrap();
    assert_eq!(fixture.item("I_task").parent.as_deref(), Some("I_b"));
    let calls = fixture.seen();
    assert!(calls.iter().any(|call| call[0] == "removeSubIssue"));
    assert!(calls.iter().any(|call| call[0] == "addSubIssue"));
}

#[tokio::test]
async fn a_second_copy_of_the_same_item_updates_it_rather_than_duplicating_it() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let first = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .unwrap();
    let mut revised = task(
        "T-1",
        "Publish, revised",
        status(StatusCategory::Todo, "Todo"),
    );
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let second = source
        .write_task(&ItemWrite {
            target: Some(first.clone()),
            item: revised,
            depends_on: vec![],
        })
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(fixture.item(&first.0).title, "Publish, revised");
}

#[tokio::test]
async fn the_copy_origin_is_kept_in_the_boards_own_text_field() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    let id = source.write_task(&write(item)).await.unwrap();
    assert_eq!(fixture.item(&id.0).origin.as_deref(), Some("notes:T-1"));
    assert!(
        !fixture
            .item(&id.0)
            .body
            .unwrap_or_default()
            .contains("origin"),
        "a short typed value belongs in a typed field, not in the caller's own prose"
    );
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().metadata["onetaskgraph.origin"],
        json!("notes:T-1")
    );
}

#[tokio::test]
async fn a_board_without_the_origin_field_refuses_an_item_that_carries_one() {
    let fixture = board_with(vec![], true, false);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    let message = refusal(source.write_task(&write(item)).await.expect_err("no field"));
    assert!(message.contains("onetaskgraph.origin"), "{message}");
}

#[tokio::test]
async fn write_refusals_name_stale_targets_and_labels_this_destination_cannot_carry() {
    let fixture = board(vec![Item::issue("I_1", "held").labelled(&[("L_1", "bug")])]);
    let source = source(&fixture);
    let stale = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("I_missing".to_owned())),
                item: task("T", "x", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![],
            })
            .await
            .expect_err("a target the destination no longer holds"),
    );
    assert!(stale.contains("I_missing"), "{stale}");

    let created = refusal(
        source
            .write_task(&write(Task {
                labels: vec![Label {
                    id: NativeId("L_1".to_owned()),
                    name: "bug".to_owned(),
                    color: None,
                }],
                ..task("T", "x", status(StatusCategory::Todo, "Todo"))
            }))
            .await
            .expect_err("creation carries no labels"),
    );
    assert!(created.contains("labels"), "{created}");

    let mismatched = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: task("T", "x", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![],
            })
            .await
            .expect_err("labels differ from the ones held"),
    );
    assert!(mismatched.contains("labels"), "{mismatched}");
}

#[tokio::test]
async fn a_draft_item_is_a_task_this_destination_updates_but_never_closes() {
    let fixture = board(vec![Item::draft("D_1", "a draft").status("Todo")]);
    let source = source(&fixture);
    let held = source
        .get_task(&NativeId("D_1".to_owned()))
        .await
        .unwrap()
        .expect("a draft reads as a task");
    assert_eq!(held.project, None);
    assert!(held.repositories.is_empty());

    let mut revised = task(
        "D_1",
        "a revised draft",
        status(StatusCategory::Todo, "Todo"),
    );
    revised.content = Some("prose".to_owned());
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("D_1".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("a draft's visible fields are writable");
    assert_eq!(fixture.item("D_1").title, "a revised draft");

    let closed = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("D_1".to_owned())),
                item: task("D_1", "done", status(StatusCategory::Done, "Done")),
                depends_on: vec![],
            })
            .await
            .expect_err("a draft has no open or closed state"),
    );
    assert!(closed.contains("draft"), "{closed}");

    let filed = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("D_1".to_owned())),
                item: Task {
                    project: Some(NativeId("I_plan".to_owned())),
                    ..task("D_1", "x", status(StatusCategory::Todo, "Todo"))
                },
                depends_on: vec![],
            })
            .await
            .expect_err("a draft cannot be a sub-issue"),
    );
    assert!(filed.contains("sub-issue"), "{filed}");
}

/// Every edge one direction reports, walked to exhaustion.
///
/// The native connection is answered first and the recorded tail resumes under a cursor of
/// its own, so a caller that stops at the first page has read half the answer.
async fn walk(
    source: &dyn TaskSource,
    id: &str,
    kind: ItemKind,
    direction: Direction,
    limit: u32,
) -> Result<Vec<DependencyEdge>, SourceError> {
    let mut cursor = None;
    let mut edges = Vec::new();
    loop {
        let request = match cursor {
            None => page(limit),
            Some(Cursor(ref cursor)) => resume(cursor, limit),
        };
        let id = NativeId(id.to_owned());
        let read = match kind {
            ItemKind::Task => source.task_dependencies(&id, direction, &request).await?,
            ItemKind::Project => {
                source
                    .project_dependencies(&id, direction, &request)
                    .await?
            }
        };
        edges.extend(read.items);
        match read.next {
            Some(next) => cursor = Some(next),
            None => return Ok(edges),
        }
    }
}

fn edge(from: (&str, ItemKind), to: (&str, ItemKind)) -> DependencyEdge {
    DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId(from.0.to_owned()), from.1),
        to: DependencyEndpoint::from_native(NativeId(to.0.to_owned()), to.1),
        kind: DependencyKind::Blocks,
    }
}

#[tokio::test]
async fn project_dependencies_are_answered_by_the_issues_own_blocked_by() {
    // The aggregate walk over `projectItems` existed only because one board was one
    // project. With project issues the native relationship answers directly.
    let fixture = board(vec![
        Item::issue("I_p1", "plan one").sub_issues(1),
        Item::issue("I_p2", "plan two").sub_issues(1),
        Item::issue("I_t1", "step").parent("I_p1").status("Todo"),
        Item::issue("I_t2", "step").parent("I_p2").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_project(&ItemWrite {
            target: Some(NativeId("I_p1".to_owned())),
            item: Project {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..project("P", "plan one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![edge(
                ("I_p1", ItemKind::Project),
                ("I_p2", ItemKind::Project),
            )],
        })
        .await
        .expect("a project dependency is native");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == "I_p2")
    );
    let forward = source
        .project_dependencies(
            &NativeId("I_p1".to_owned()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        forward.items,
        vec![edge(
            ("I_p1", ItemKind::Project),
            ("I_p2", ItemKind::Project)
        )]
    );
    let reverse = source
        .project_dependencies(
            &NativeId("I_p2".to_owned()),
            Direction::DependedOnBy,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        reverse.items, forward.items,
        "one relationship reads the same from either end"
    );
}

#[tokio::test]
async fn a_task_dependency_reports_the_far_ends_own_kind() {
    let fixture = board(vec![
        Item::issue("I_plan", "plan").sub_issues(1),
        Item::issue("I_task", "step")
            .parent("I_plan")
            .status("Todo"),
        Item::issue("I_other", "another step").status("Todo"),
    ]);
    let source = source(&fixture);
    let mut item = task("T", "step", status(StatusCategory::Todo, "Todo"));
    item.project = Some(NativeId("I_plan".to_owned()));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_task".to_owned())),
            item,
            depends_on: vec![edge(
                ("I_task", ItemKind::Task),
                ("I_other", ItemKind::Task),
            )],
        })
        .await
        .unwrap();
    let forward = source
        .task_dependencies(
            &NativeId("I_task".to_owned()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        forward.items,
        vec![edge(
            ("I_task", ItemKind::Task),
            ("I_other", ItemKind::Task)
        )]
    );
}

#[tokio::test]
async fn a_far_end_no_issue_relationship_can_name_is_read_from_the_reserved_key() {
    let fixture = board(vec![
        Item::issue("I_1", "step").status("Todo").body(
            "<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[{\"id\":\"elsewhere:P-9\",\"kind\":\"project\"}]}\n-->",
        ),
    ]);
    let source = source(&fixture);
    let forward = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .unwrap();
    assert_eq!(forward.len(), 1);
    assert_eq!(forward[0].to.id(), "elsewhere:P-9");
    assert_eq!(forward[0].to.kind, ItemKind::Project);
    assert!(
        walk(
            source.as_ref(),
            "I_1",
            ItemKind::Task,
            Direction::DependedOnBy,
            10
        )
        .await
        .unwrap()
        .is_empty(),
        "the reverse of a recorded edge belongs to the far end"
    );
}

#[tokio::test]
async fn an_item_may_not_record_a_far_end_its_own_relationship_can_name() {
    for (recorded, near, kind, problem) in [
        (json!(["I_2"]), "I_1", ItemKind::Task, "relate natively"),
        (
            json!([{"id":"work:I_2","kind":"task"}]),
            "I_1",
            ItemKind::Task,
            "relate natively",
        ),
        (
            json!({"id":"elsewhere:P-9"}),
            "I_1",
            ItemKind::Task,
            "not a list of dependency endpoints",
        ),
        (
            json!([{"id":"bad source:P-9","kind":"project"}]),
            "I_1",
            ItemKind::Task,
            "source name",
        ),
    ] {
        let body = format!(
            "<!-- onetaskgraph.metadata\n{}\n-->",
            json!({ "onetaskgraph.depends_on": recorded })
        );
        let fixture = board(vec![
            Item::issue("I_1", "step").status("Todo").body(&body),
            Item::issue("I_2", "other").status("Todo"),
        ]);
        let source = source(&fixture);
        let message = refusal(
            match kind {
                ItemKind::Task => {
                    source
                        .task_dependencies(
                            &NativeId(near.to_owned()),
                            Direction::DependsOn,
                            &page(10),
                        )
                        .await
                }
                ItemKind::Project => {
                    source
                        .project_dependencies(
                            &NativeId(near.to_owned()),
                            Direction::DependsOn,
                            &page(10),
                        )
                        .await
                }
            }
            .expect_err("a reserved key holding what it must not"),
        );
        assert!(message.contains(problem), "{recorded}: {message}");
        assert!(message.contains("onetaskgraph.depends_on"), "{message}");
    }
}

#[tokio::test]
async fn a_draft_may_record_the_far_end_an_issue_may_not() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft")
            .status("Todo")
            .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[\"I_2\"]}\n-->"),
        Item::issue("I_2", "other").status("Todo"),
    ]);
    let source = source(&fixture);
    let edges = walk(
        source.as_ref(),
        "D_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .expect("a backend with no relationship at all records anything");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to.id(), "I_2");
}

#[tokio::test]
async fn a_recorded_tail_pages_and_refuses_a_cursor_no_reverse_walk_issues() {
    let recorded = json!({"onetaskgraph.depends_on":[
        {"id":"elsewhere:A","kind":"task"},
        {"id":"elsewhere:B","kind":"task"}
    ]});
    let fixture =
        board(vec![Item::issue("I_1", "step").status("Todo").body(
            &format!("<!-- onetaskgraph.metadata\n{recorded}\n-->"),
        )]);
    let source = source(&fixture);
    let walked = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        walked
            .iter()
            .map(|edge| edge.to.id().to_owned())
            .collect::<Vec<_>>(),
        ["elsewhere:A", "elsewhere:B"]
    );
    let cursor = source
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(1))
        .await
        .unwrap()
        .next
        .expect("a recorded tail resumes");

    let message = refusal(
        source
            .task_dependencies(
                &NativeId("I_1".to_owned()),
                Direction::DependedOnBy,
                &resume(&cursor.0, 1),
            )
            .await
            .expect_err("a reverse read never issues a recorded cursor"),
    );
    assert!(message.contains("resume it in the direction"), "{message}");
    for invalid in ["onetaskgraph.depends_on:nope"] {
        assert!(
            source
                .task_dependencies(
                    &NativeId("I_1".to_owned()),
                    Direction::DependsOn,
                    &resume(invalid, 1)
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn a_dependency_write_refuses_a_far_end_of_a_kind_this_board_says_it_is_not() {
    // The caller names the far end's kind and the board holds the far end itself, so a
    // disagreement is settled rather than stored: `I_2` has sub-issues, which is what
    // makes it a project, and an edge naming it a task would otherwise be written as a
    // native `blockedBy` link standing for a relationship at a level it is not at.
    let fixture = board(vec![
        Item::issue("I_1", "step").status("Todo"),
        Item::issue("I_2", "plan").status("Todo").sub_issues(1),
    ]);
    let message = refusal(
        source(&fixture)
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: task("T", "step", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task))],
            })
            .await
            .expect_err("a far end the board says is a project"),
    );
    assert!(
        message.contains("I_2") && message.contains("project") && message.contains("task"),
        "the entry and both kinds are named: {message}"
    );
}

#[tokio::test]
async fn a_dependency_write_refuses_a_same_source_far_end_the_board_does_not_hold() {
    let fixture = board(vec![Item::issue("I_1", "step").status("Todo")]);
    let message = refusal(
        source(&fixture)
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: Task {
                    repositories: vec![
                        Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                    ],
                    ..task("T", "step", status(StatusCategory::Todo, "Todo"))
                },
                depends_on: vec![edge(("I_1", ItemKind::Task), ("I_gone", ItemKind::Task))],
            })
            .await
            .expect_err("a far end this board does not hold"),
    );
    assert!(message.contains("I_gone"), "{message}");
}

#[tokio::test]
async fn a_same_source_far_end_keeps_every_colon_its_own_native_id_holds() {
    // A qualified id is `<source>:<native>` split at its *first* colon, and a GitHub node id
    // is opaque, so the native half may hold colons of its own. Split at the last one, this
    // far end would be looked up as `2`, and an edge to an item this board holds would be
    // refused as missing.
    let fixture = board(vec![
        Item::issue("I_1", "one").status("Todo"),
        Item::issue("I:urn:2", "two").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: Task {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..task("T", "one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("I_1".to_owned()), ItemKind::Task),
                to: DependencyEndpoint::new("work:I:urn:2".to_owned(), ItemKind::Task).unwrap(),
                kind: DependencyKind::Blocks,
            }],
        })
        .await
        .expect("a same-source far end this board holds is written natively");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == "I:urn:2"),
        "the whole native id is the far end: {:?}",
        fixture.seen()
    );
    assert!(
        !fixture
            .item("I_1")
            .body
            .unwrap_or_default()
            .contains("work:I:urn:2"),
        "a far end this board holds is linked natively rather than recorded"
    );
}

#[tokio::test]
async fn dependencies_of_an_item_nothing_holds_are_refused_rather_than_empty() {
    let fixture = board(vec![]);
    let message = refusal(
        source(&fixture)
            .task_dependencies(
                &NativeId("I_missing".to_owned()),
                Direction::DependsOn,
                &page(10),
            )
            .await
            .expect_err("a dependency read is never silently empty"),
    );
    assert!(message.contains("I_missing"), "{message}");
}

#[tokio::test]
async fn tasks_projects_and_labels_page_to_exhaustion_in_a_stable_order() {
    let fixture = board(vec![
        Item::issue("I_p", "plan").sub_issues(2),
        Item::issue("I_1", "one")
            .parent("I_p")
            .labelled(&[("L_a", "alpha")]),
        Item::issue("I_2", "two")
            .parent("I_p")
            .labelled(&[("L_b", "beta")]),
    ]);
    let source = source(&fixture);
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let page = source
            .query_tasks(&TaskQuery::default(), &request)
            .await
            .unwrap();
        walked.extend(page.items.into_iter().map(|task| task.id.0));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, ["I_1", "I_2"]);

    let labels = source.labels(&page(10)).await.unwrap();
    assert_eq!(
        labels
            .items
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(1))
        .await
        .unwrap();
    assert_eq!(projects.items.len(), 1);
    assert!(projects.next.is_none(), "one project fits one page");
}

#[tokio::test]
async fn a_board_larger_than_one_page_is_walked_before_it_is_answered() {
    let fixture = board(
        (1..=5)
            .map(|n| Item::issue(&format!("I_{n}"), "step").status("Todo"))
            .collect(),
    );
    let source = source(&fixture);
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(100))
            .await
            .unwrap()
            .items
            .len(),
        5
    );
    assert!(
        source
            .get_task(&NativeId("I_5".to_owned()))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_zero_limit_and_a_nonsense_cursor_are_refused() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(0))
            .await
            .is_err()
    );
    assert!(source.labels(&page(0)).await.is_err());
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page(0))
            .await
            .is_err()
    );
    assert!(
        source
            .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(0))
            .await
            .is_err()
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &resume("not-a-number", 5))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn health_names_the_board_it_read_and_the_source_declares_what_it_applies() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let health = source.health().await.unwrap();
    assert!(health.reachable);
    assert!(health.detail.unwrap().contains("Roadmap"));
    assert_eq!(source.kind(), onetaskgraph_github_projects::KIND);
    assert_eq!(source.writes(), WriteSupport::Supported);
    // Every field, rather than a subset: this source applies every predicate a query
    // carries, in process, over a board it has already walked in full, and GitHub's
    // project-items connection has no filter argument to push any of them into.
    assert_eq!(
        source.capabilities(),
        Capabilities {
            projects: Support::Native,
            documents: Support::Unsupported,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: onetaskgraph_github_projects::MAX_PAGE_SIZE,
        }
    );
    assert_eq!(
        onetaskgraph_github_projects::MAX_PAGE_SIZE,
        100,
        "the declared page size is GitHub's own connection maximum"
    );
}

#[test]
fn the_config_schema_is_strict_and_build_validates_every_input() {
    let schema = serde_json::to_value(Plugin.config_schema()).unwrap();
    assert_eq!(schema["additionalProperties"], json!(false));
    for key in ["owner", "project_number", "repository", "status_mapping"] {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }
    for invalid in [
        json!({"owner":"","project_number":7}),
        json!({"owner":"-bad","project_number":7}),
        json!({"owner":"octo","project_number":0}),
        json!({"owner":"octo","project_number":7,"token_env":"1BAD"}),
        json!({"owner":"octo","project_number":7,"endpoint":"not a url"}),
        json!({"owner":"octo","project_number":7,"endpoint":"http://example.invalid/graphql"}),
        json!({"owner":"octo","project_number":7,"repository":"nameless"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/a name"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/.."}),
        json!({"owner":"octo","project_number":7,"repository":"acme/a/b"}),
        json!({"owner":"octo","project_number":7,
               "repository":format!("acme/{}", "n".repeat(101))}),
        json!({"owner":"octo","project_number":7,"unknown":true}),
    ] {
        assert!(
            Plugin
                .build(&SourceName::new("work").unwrap(), &invalid, &Secrets)
                .is_err(),
            "{invalid}"
        );
    }
    struct NoSecret;
    impl SecretResolver for NoSecret {
        fn get(&self, _: &str) -> Option<SecretString> {
            None
        }
    }
    let message = match Plugin.build(
        &SourceName::new("work").unwrap(),
        &json!({"owner":"octo","project_number":7}),
        &NoSecret,
    ) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("a missing credential is refused"),
    };
    assert!(message.contains("GH_PROJECTS_TOKEN"), "{message}");
    assert!(!message.contains("test-token"), "{message}");
}

#[tokio::test]
async fn transport_http_json_and_graphql_failures_each_reach_the_caller_intact() {
    let cases: Vec<(String, &str)> = vec![
        (
            raw_server_with_headers("429 Too Many Requests", "{}", "retry-after: 30\r\n"),
            "rate",
        ),
        (
            raw_server_with_headers("403 Forbidden", "{}", "x-ratelimit-remaining: 0\r\n"),
            "rate",
        ),
        (raw_server("401 Unauthorized", "{}"), "credential"),
        (raw_server("500 Internal Server Error", "{}"), "HTTP"),
        (raw_server("200 OK", "not json"), "invalid JSON"),
        (
            raw_server("200 OK", r#"{"errors":{"message":"x"}}"#),
            "not an array",
        ),
        (
            raw_server("200 OK", r#"{"errors":[{"message":"boom"}]}"#),
            "boom",
        ),
        (
            raw_server(
                "200 OK",
                r#"{"errors":[{"message":"Resource not accessible"}]}"#,
            ),
            "grant",
        ),
        (raw_server("200 OK", r#"{"errors":[]}"#), "no data object"),
        (
            raw_server("200 OK", r#"{"data":{"owner":null}}"#),
            "was not found",
        ),
        (
            raw_server(
                "200 OK",
                r#"{"data":{"owner":{"projectV2":{"title":"T","fields":{"nodes":[],"pageInfo":{"hasNextPage":false}},"items":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}}"#,
            ),
            "missing string field id",
        ),
    ];
    for (endpoint, expected) in cases {
        let message = refusal(
            configured(&endpoint, json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase()),
            "expected {expected} in {message}"
        );
    }
    assert!(
        configured("http://127.0.0.1:1/graphql", json!({}))
            .health()
            .await
            .is_err()
    );
    let untitled = raw_server(
        "200 OK",
        r#"{"data":{"owner":{"projectV2":{"id":"B","fields":{"nodes":[],"pageInfo":{"hasNextPage":false}},"items":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}}"#,
    );
    let message = refusal(
        configured(&untitled, json!({}))
            .health()
            .await
            .expect_err("a board with no title"),
    );
    assert!(message.contains("missing string field title"), "{message}");
}

#[tokio::test]
async fn malformed_board_shapes_are_named_rather_than_guessed_at() {
    let board = |items: Value, fields: Value| json!({"data":{"owner":{"projectV2":{"id":"B","title":"T","fields":fields,"items":items}}}});
    let complete = json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
    let cases = [
        (
            board(
                json!({"nodes":"no","pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "items.nodes is not an array",
        ),
        (board(json!({"nodes":[]}), complete.clone()), "no pageInfo"),
        (
            board(
                json!({"nodes":[{"id":"PVTI"}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "missing content",
        ),
        (
            board(
                json!({"nodes":[{"id":"PVTI","content":{"__typename":"Issue","id":"I","subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "missing fieldValues",
        ),
        (
            board(
                json!({"nodes":[{"id":"PVTI","fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":true}},
                                 "content":{"__typename":"Issue","id":"I","subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "exceeds the supported nested connection size",
        ),
        (
            board(
                json!({"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":""}}),
                complete.clone(),
            ),
            "did not advance",
        ),
    ];
    for (body, expected) in cases {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_mutation_that_answers_about_another_item_is_refused_as_malformed() {
    let complete = json!({"nodes":[{"__typename":"ProjectV2SingleSelectField","id":"FIELD_status",
                                    "name":"Status","options":[{"id":"OPT_todo","name":"Todo"}]}],
                          "pageInfo":{"hasNextPage":false}});
    let empty_board = json!({"data":{"owner":{"projectV2":{"id":"B","title":"T","fields":complete,
        "items":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    for (bodies, expected) in [
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":null}}}),
            ],
            "returned no issue",
        ),
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":{"id":"I_new"}}}}),
                json!({"data":{"addProjectV2ItemById":{"item":null}}}),
            ],
            "returned no project item",
        ),
    ] {
        let mut bodies = bodies;
        bodies.insert(
            1,
            json!({"data":{"repository":{"id":"R","nameWithOwner":"acme/work"}}}),
        );
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

fn board_json(fields: Value, items: Value) -> Value {
    json!({"data":{"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap",
        "fields":fields,"items":items}}}})
}

fn complete(nodes: Value) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}})
}

fn usable_fields() -> Value {
    complete(json!([
        {"__typename":"ProjectV2SingleSelectField","id":"FIELD_status","name":"Status",
         "options":[{"id":"OPT_todo","name":"Todo"}]},
        {"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}
    ]))
}

fn issue_item(content: Value) -> Value {
    json!({"id":"PVTI_1","fieldValues":complete(json!([])),"content":content})
}

fn plain_issue() -> Value {
    json!({"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
           "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
           "parent":null,"subIssuesSummary":{"total":0},
           "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}})
}

#[tokio::test]
async fn every_board_shape_this_source_will_not_guess_at_is_named() {
    let cases = [
        (
            board_json(
                usable_fields(),
                complete(
                    json!([{"id":"PVTI_1","fieldValues":{"nodes":"no","pageInfo":{"hasNextPage":false}},"content":plain_issue()}]),
                ),
            ),
            "fieldValues.nodes is not an array",
        ),
        (
            board_json(
                usable_fields(),
                complete(
                    json!([{"id":"PVTI_1","fieldValues":{"nodes":[]},"content":plain_issue()}]),
                ),
            ),
            "has no pageInfo",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":7})
                )])),
            ),
            "field body is not a string or null",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":"",
                    "createdAt":"not-a-time","subIssuesSummary":{"total":0}})
                )])),
            ),
            "is not a timestamp",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":"",
                    "subIssuesSummary":{"total":0},
                    "labels":{"nodes":"no","pageInfo":{"hasNextPage":false}}})
                )])),
            ),
            "content labels.nodes is not an array",
        ),
        (
            board_json(usable_fields(), json!({"nodes":[],"pageInfo":{}})),
            "missing boolean field hasNextPage",
        ),
    ];
    for (body, expected) in cases {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn an_item_whose_content_the_token_cannot_see_is_left_out_rather_than_guessed_at() {
    let body = board_json(
        usable_fields(),
        complete(json!([
            {"id":"PVTI_hidden","fieldValues":complete(json!([])),"content":null},
            issue_item(plain_issue()),
        ])),
    );
    let source = configured(&raw_server("200 OK", &body.to_string()), json!({}));
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &resume("99", 10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "a cursor past the end is an empty last page rather than a panic"
    );
}

#[tokio::test]
async fn a_board_answered_in_two_pages_is_walked_before_it_is_answered() {
    let first = json!({"data":{"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap",
        "fields":usable_fields(),
        "items":{"nodes":[issue_item(plain_issue())],"pageInfo":{"hasNextPage":true,"endCursor":"1"}}}}}});
    let mut second_content = plain_issue();
    second_content["id"] = json!("I_2");
    let second = board_json(
        usable_fields(),
        complete(json!([issue_item(second_content)])),
    );
    let endpoint = sequence_server(vec![first, second]);
    assert_eq!(
        configured(&endpoint, json!({}))
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|task| task.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_1", "I_2"]
    );
}

#[tokio::test]
async fn a_graphql_error_with_nothing_to_say_still_says_something() {
    let message = refusal(
        configured(
            &raw_server("200 OK", r#"{"errors":[{"code":9}],"data":null}"#),
            json!({}),
        )
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("an error array with no message"),
    );
    assert!(message.contains("GraphQL errors"), "{message}");
}

#[tokio::test]
async fn a_status_or_origin_field_of_the_wrong_shape_is_refused_by_name() {
    for (fields, expected) in [
        (
            complete(json!([
                {"__typename":"ProjectV2Field","id":"FIELD_status","name":"Status"},
                {"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}
            ])),
            "not a single-select field",
        ),
        (
            complete(json!([
                {"__typename":"ProjectV2SingleSelectField","id":"FIELD_status","name":"Status",
                 "options":[{"id":"OPT_todo","name":"Todo"}]},
                {"__typename":"ProjectV2SingleSelectField","id":"FIELD_origin",
                 "name":"onetaskgraph.origin","options":[]}
            ])),
            "is not a text field",
        ),
        (
            json!({"nodes":"no","pageInfo":{"hasNextPage":false}}),
            "fields.nodes is not an array",
        ),
    ] {
        let body = board_json(fields, complete(json!([])));
        let endpoint = sequence_server(vec![
            body.clone(),
            json!({"data":{"repository":{"id":"R","nameWithOwner":"acme/work"}}}),
            json!({"data":{"createIssue":{"issue":{"id":"I_new"}}}}),
            json!({"data":{"addProjectV2ItemById":{"item":{"id":"PVTI_new"}}}}),
        ]);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_category_configured_closed_closes_the_issue_it_is_written_to() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"in-progress":{"closed":"not-planned"}}}),
    );
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .unwrap();
    assert_eq!(fixture.item(&id.0).state, "CLOSED");
    assert_eq!(
        fixture.item(&id.0).state_reason.as_deref(),
        Some("NOT_PLANNED")
    );
}

#[tokio::test]
async fn a_far_end_in_another_source_is_recorded_and_a_native_one_is_taken_back_out() {
    let fixture = board(vec![
        Item::issue("I_1", "one").status("Todo"),
        Item::issue("I_2", "two").status("Todo"),
    ]);
    let source = source(&fixture);
    let held = |edges: Vec<DependencyEdge>| ItemWrite {
        target: Some(NativeId("I_1".to_owned())),
        item: Task {
            repositories: vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()],
            ..task("T", "one", status(StatusCategory::Todo, "Todo"))
        },
        depends_on: edges,
    };
    source
        .write_task(&held(vec![
            edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task)),
            DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("I_1".to_owned()), ItemKind::Task),
                to: DependencyEndpoint::new("elsewhere:T-9".to_owned(), ItemKind::Task).unwrap(),
                kind: DependencyKind::Blocks,
            },
        ]))
        .await
        .unwrap();
    assert!(
        fixture.item("I_1").body.unwrap().contains("elsewhere:T-9"),
        "a far end no relationship here can name goes to the reserved key"
    );
    let walked = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .unwrap();
    assert_eq!(
        walked
            .iter()
            .map(|edge| edge.to.id().to_owned())
            .collect::<Vec<_>>(),
        ["I_2", "elsewhere:T-9"]
    );

    // Writing the item again without either edge takes the native one back out and clears
    // the recorded one.
    source.write_task(&held(vec![])).await.unwrap();
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "removeBlockedBy")
    );
    assert!(
        walk(
            source.as_ref(),
            "I_1",
            ItemKind::Task,
            Direction::DependsOn,
            10
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
async fn a_far_end_that_is_a_sub_issue_or_carries_a_broken_marker_is_read_as_it_is() {
    let node = |body: Value| {
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_far","body":body,"parent":null,
                                   "subIssuesSummary":{"total":0}}],
                        "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}})
    };
    let sub_issue = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[{"id":"I_far","body":null,"parent":{"id":"I_plan"},
                               "subIssuesSummary":{"total":4}}],
                    "pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let empty_board = board_json(usable_fields(), complete(json!([])));
    let endpoint = sequence_server(vec![sub_issue, empty_board.clone()]);
    let edges = configured(&endpoint, json!({}))
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
        .await
        .unwrap();
    assert_eq!(
        edges.items[0].to.kind,
        ItemKind::Task,
        "a sub-issue is a task however many sub-issues of its own it has"
    );

    let endpoint = sequence_server(vec![node(json!(
        "<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"epic\"}\n-->"
    ))]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
            .await
            .expect_err("a far end whose marker this contract cannot read"),
    );
    assert!(message.contains("I_far"), "{message}");
}

#[tokio::test]
async fn a_dependency_read_for_an_item_no_longer_on_the_board_has_no_recorded_tail() {
    let node = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let endpoint = sequence_server(vec![node, board_json(usable_fields(), complete(json!([])))]);
    let edges = configured(&endpoint, json!({}))
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
        .await
        .unwrap();
    assert!(edges.items.is_empty());
    assert!(edges.next.is_none());
}

#[tokio::test]
async fn malformed_dependency_connections_are_named_rather_than_read_as_empty() {
    for (body, expected) in [
        (
            json!({"data":{"node":{"__typename":"Issue"}}}),
            "missing its connection",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue",
                "blockedBy":{"nodes":"no","pageInfo":{"hasNextPage":false}}}}}),
            "nodes is not an array",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[]}}}}),
            "missing pageInfo",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue",
                "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":""}}}}}),
            "did not advance",
        ),
    ] {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn every_write_mutation_that_answers_about_the_wrong_item_is_refused_as_malformed() {
    let fields = usable_fields();
    let board_with_one = board_json(
        fields.clone(),
        complete(json!([{"id":"PVTI_1","fieldValues":complete(json!([])),
                         "content":{"__typename":"Issue","id":"I_1","title":"one","body":"",
                                    "state":"OPEN","stateReason":null,
                                    "repository":{"nameWithOwner":"acme/work"},
                                    "parent":{"id":"I_old"},"subIssuesSummary":{"total":0},
                                    "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}])),
    );
    let ok_update = json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}});
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![board_with_one.clone(), json!({"data":{"updateIssue":{}}})],
            "item update returned no item",
        ),
        (
            vec![
                board_with_one.clone(),
                json!({"data":{"updateIssue":{"issue":{"id":"I_other"}}}}),
            ],
            "item update returned the wrong item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{}}}),
            ],
            "field update returned no project item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_other"}}}}),
            ],
            "field update returned the wrong project item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"issue":{"id":"I_old"}}}}),
            ],
            "sub-issue update returned no sub-issue",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"subIssue":{"id":"I_1"}}}}),
            ],
            "sub-issue update returned no issue",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"issue":{"id":"I_wrong"},"subIssue":{"id":"I_1"}}}}),
            ],
            "sub-issue update returned the wrong issues",
        ),
    ];
    for (bodies, expected) in cases {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&ItemWrite {
                    target: Some(NativeId("I_1".to_owned())),
                    item: Task {
                        repositories: vec![
                            Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                        ],
                        ..task("T", "one", status(StatusCategory::Todo, "Todo"))
                    },
                    depends_on: vec![],
                })
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_malformed_dependency_mutation_or_reconciliation_read_is_refused() {
    let board_with_two = board_json(
        usable_fields(),
        complete(json!([
            {"id":"PVTI_1","fieldValues":complete(json!([])),
             "content":{"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
                        "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                        "parent":null,"subIssuesSummary":{"total":0},
                        "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}},
            {"id":"PVTI_2","fieldValues":complete(json!([])),
             "content":{"__typename":"Issue","id":"I_2","title":"two","body":"","state":"OPEN",
                        "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                        "parent":null,"subIssuesSummary":{"total":0},
                        "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}
        ])),
    );
    let ok_update = json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}});
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let held = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue"}}}),
            ],
            "no blockedBy connection",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue",
                    "blockedBy":{"nodes":"no","pageInfo":{"hasNextPage":false}}}}}),
            ],
            "nodes is not an array",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"blockingIssue":{"id":"I_2"}}}}),
            ],
            "dependency update returned no issue",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"issue":{"id":"I_1"}}}}),
            ],
            "returned no blocking issue",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_9"}}}}),
            ],
            "returned the wrong issues",
        ),
    ];
    for (bodies, expected) in cases {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&ItemWrite {
                    target: Some(NativeId("I_1".to_owned())),
                    item: Task {
                        repositories: vec![
                            Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                        ],
                        ..task("T", "one", status(StatusCategory::Todo, "Todo"))
                    },
                    depends_on: vec![edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task))],
                })
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_blocked_by_connection_answered_in_pages_is_walked_before_it_is_reconciled() {
    let board_one = board_json(
        usable_fields(),
        complete(json!([{"id":"PVTI_1","fieldValues":complete(json!([])),
            "content":{"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
                       "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                       "parent":null,"subIssuesSummary":{"total":0},
                       "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}])),
    );
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let endpoint = sequence_server(vec![
        board_one,
        json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}}),
        ok_field.clone(),
        ok_field,
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_a"}],"pageInfo":{"hasNextPage":true,"endCursor":"c1"}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_b"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_a"}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_b"}}}}),
    ]);
    configured(&endpoint, json!({}))
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: Task {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..task("T", "one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![],
        })
        .await
        .expect("both pages of held blockers are taken back out");
}

#[test]
fn the_plugin_names_the_kind_the_registry_knows_it_by() {
    assert_eq!(Plugin.kind(), onetaskgraph_github_projects::KIND);
}

#[tokio::test]
async fn a_closed_status_still_selects_the_column_that_spells_it_so_a_copy_settles() {
    // The closed state carries the category and the option carries the name, so a write
    // that closed the issue and left the option alone would read back under whatever
    // column the item happened to sit in — and a copy would report a change forever.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Done, "Shipped"),
        )))
        .await
        .unwrap();
    assert_eq!(fixture.item(&id.0).state, "CLOSED");
    assert_eq!(fixture.item(&id.0).status.as_deref(), Some("Shipped"));
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().status,
        status(StatusCategory::Done, "Shipped")
    );
}

#[tokio::test]
async fn a_sub_issue_count_this_source_cannot_read_is_refused_rather_than_read_as_none() {
    // Reading an absent or non-integer `subIssuesSummary.total` as zero would classify a
    // project as a task — quietly, and in exactly the case the kind marker exists for.
    let mut without = plain_issue();
    without.as_object_mut().unwrap().remove("subIssuesSummary");
    let mut malformed = plain_issue();
    malformed["subIssuesSummary"] = json!({"total":"many"});
    for content in [without, malformed] {
        let body = board_json(usable_fields(), complete(json!([issue_item(content)])));
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err("a sub-issue count this source cannot read"),
        );
        assert!(message.contains("subIssuesSummary"), "{message}");
    }

    for far in [
        json!({"id":"I_far","body":null,"parent":null}),
        json!({"id":"I_far","body":null,"parent":null,"subIssuesSummary":{"total":-1}}),
    ] {
        let node = json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[far],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let message = refusal(
            configured(&raw_server("200 OK", &node.to_string()), json!({}))
                .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
                .await
                .expect_err("a far end whose sub-issue count this source cannot read"),
        );
        assert!(message.contains("subIssuesSummary"), "{message}");
    }
}

#[tokio::test]
async fn a_drafts_dependency_on_an_issue_is_recorded_rather_than_lost() {
    // A draft has neither `blockedBy` nor `blocking`, so an edge of one classified as
    // native would be written nowhere: a draft's native reconciliation never runs.
    let fixture = board(vec![
        Item::draft("D_1", "a draft").status("Todo"),
        Item::issue("I_2", "an issue").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("D_1".to_owned())),
            item: task("D", "a draft", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![edge(("D_1", ItemKind::Task), ("I_2", ItemKind::Task))],
        })
        .await
        .unwrap();
    assert_eq!(
        walk(
            source.as_ref(),
            "D_1",
            ItemKind::Task,
            Direction::DependsOn,
            10
        )
        .await
        .unwrap()
        .iter()
        .map(|edge| edge.to.id().to_owned())
        .collect::<Vec<_>>(),
        ["I_2"]
    );
}

#[tokio::test]
async fn an_origin_that_is_not_a_qualified_id_is_refused_before_anything_is_created() {
    // The board's origin field is text, so a value of another JSON type has nowhere to
    // go; storing it as no origin at all would leave the copy unable to find this item
    // again. The refusal comes before `createIssue`, like every other one this write owes.
    let fixture = board_with(vec![], true, true);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([(
        "onetaskgraph.origin".to_owned(),
        json!({"source":"notes","id":"T-1"}),
    )]);
    let said = refusal(
        source
            .write_task(&write(item))
            .await
            .expect_err("an origin of the wrong JSON type"),
    );
    assert!(
        said.contains("onetaskgraph.origin") && said.contains("qualified id"),
        "the key and what it holds are named: {said}"
    );
    assert!(
        fixture.seen().is_empty(),
        "nothing was written before the refusal: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_board_that_cannot_carry_the_origin_refuses_before_it_creates_anything() {
    // Refusing after `createIssue` would leave an issue behind that nothing asked for.
    let fixture = board_with(vec![], true, false);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    source
        .write_task(&write(item))
        .await
        .expect_err("a board with no origin field");
    assert!(
        fixture.seen().is_empty(),
        "nothing was written before the refusal: {:?}",
        fixture.seen()
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "and no issue was left on the board"
    );
}

#[tokio::test]
async fn a_write_that_fails_part_way_takes_back_only_the_item_it_created() {
    // Everything this source can refuse before the first mutation is refused there, so what
    // is left is GitHub failing part way — and the two halves of that are different. An item
    // this call created is taken back, because a retry would otherwise create a second. An
    // item that was already there is not: taking it back would destroy the very state the
    // engine's copy journal exists to write back, and nothing a user typed asked for a
    // delete.
    let created = board(vec![]);
    let maker = source(&created);
    created.refuse("updateProjectV2ItemFieldValue");
    let message = refusal(
        maker
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("the board refused the field update"),
    );
    assert!(
        message.contains("updateProjectV2ItemFieldValue"),
        "the write's own failure is what the caller is told: {message}"
    );
    assert!(
        created.seen().iter().any(|call| call[0] == "deleteIssue"),
        "the issue this call created was left behind: {:?}",
        created.seen()
    );
    assert!(
        maker
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "and the board holds nothing this failed write made"
    );

    let held = board(vec![Item::issue("I_1", "one").body("first").status("Todo")]);
    let holder = source(&held);
    held.refuse("updateProjectV2ItemFieldValue");
    let mut revised = task("T-1", "one, revised", status(StatusCategory::Todo, "Todo"));
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let message = refusal(
        holder
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: revised,
                depends_on: vec![],
            })
            .await
            .expect_err("the board refused the field update"),
    );
    assert!(
        message.contains("updateProjectV2ItemFieldValue"),
        "the write's own failure is what the caller is told: {message}"
    );
    assert!(
        held.holds("I_1"),
        "a write took back an item it did not create: {:?}",
        held.seen()
    );
    assert!(
        held.seen().iter().all(|call| call[0] != "deleteIssue"),
        "a write that did not create the item asked for it to be deleted: {:?}",
        held.seen()
    );
    assert_eq!(
        held.item("I_1").title,
        "one, revised",
        "the mutation before the failure landed, and writing that back is the engine's \
         journal's job — which it can only do while the item is still there"
    );
}

/// A server which answers every request the same way, with headers of its own.
///
/// `raw_server_with_headers` cannot spell a status *and* a body a limiter needs together
/// with more than one header line, and reading a body under a non-success status is the
/// whole point here.
fn always(refusal: &Refusal) -> String {
    raw_server_with_headers(refusal.status, &refusal.body, &refusal.headers)
}

/// A source whose pacing and waiting are stated rather than defaulted.
fn paced(endpoint: &str, pacing: Value) -> Box<dyn TaskSource> {
    configured(endpoint, json!({ "pacing": pacing }))
}

/// No waiting at all: the first refusal is what the caller is told.
fn no_waiting() -> Value {
    json!({"min_mutation_interval_ms":0,"retry_budget_ms":0})
}

#[tokio::test]
async fn every_shape_a_rate_limit_arrives_in_is_classified_as_one_and_never_as_a_credential() {
    // Three shapes, because GitHub sends three and this source once read only the status:
    // a forbidden status, which it called a credential problem, and a *successful*
    // response, which it called an unexplained refusal.
    let secondary_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"type":"RATE_LIMITED",
                                "message":"You have exceeded a secondary rate limit. Please \
                                           wait a few minutes before you try again."}]})
        .to_string(),
    };
    let too_many = Refusal {
        status: "429 Too Many Requests",
        headers: "retry-after: 30\r\n".to_owned(),
        body: "{}".to_owned(),
    };
    for (what, shape, expected_hint, limiter) in [
        (
            "a too-many-requests status",
            too_many,
            Some(30),
            "primary API rate limit",
        ),
        (
            "a forbidden status naming it",
            Refusal::secondary_forbidden(),
            None,
            "secondary rate limit",
        ),
        (
            "a successful response naming it",
            secondary_in_a_success,
            None,
            "secondary rate limit",
        ),
    ] {
        let error = paced(&always(&shape), no_waiting())
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err(what);
        // The exact variant, not merely "not Auth": the kind is what a caller matches on,
        // and a rate limit reported under any other kind is a caller that cannot tell this
        // from a permission problem or a refusal it should not retry.
        let SourceError::RateLimited {
            retry_after_seconds,
            message: Some(said),
        } = &error
        else {
            panic!("{what} was not classified as a rate limit: {error:?}");
        };
        assert_eq!(
            *retry_after_seconds, expected_hint,
            "{what} did not carry the wait GitHub asked for"
        );
        assert!(
            said.contains(limiter),
            "{what} does not name which limiter refused: {said}"
        );
        assert!(
            said.contains("reading the board"),
            "{what} does not say what this source was doing: {said}"
        );
        assert!(
            !said.contains("Projects and Issues read/write"),
            "{what} still sends the operator to re-scope a token that is fine: {said}"
        );
        // And the whole of it reaches a caller that only renders the error.
        assert!(
            error.to_string().contains(said.as_str()),
            "the diagnostic is carried but not rendered: {error}"
        );
    }
}

#[tokio::test]
async fn a_credential_the_board_really_rejects_is_still_a_credential_problem() {
    // The other half of the same fix: a forbidden status carrying none of the limiter's
    // wording is a token that genuinely lacks the access, and the advice it earns is the
    // access it needs. A fix which simply stopped calling 403 a credential problem would
    // pass the test above and fail this one.
    for shape in [
        Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: json!({"message":"Resource not accessible by personal access token"}).to_string(),
        },
        Refusal {
            status: "401 Unauthorized",
            headers: String::new(),
            body: "{}".to_owned(),
        },
    ] {
        let error = paced(&always(&shape), no_waiting())
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a rejected credential");
        assert!(
            matches!(error, SourceError::Auth { .. }),
            "a rejected credential stopped being one: {error:?}"
        );
        let message = refusal(error);
        assert!(
            message.contains("Projects and Issues read/write")
                && message.contains("Pull requests read-only"),
            "the refusal no longer names the access the credential needs: {message}"
        );
    }
}

#[tokio::test]
async fn a_wait_hint_is_honoured_and_the_call_retried_rather_than_reported() {
    // The defect: a refusal carrying `retry-after` became an error with the hint attached
    // and no attempt made to honour it. A source still carrying it never sends the second
    // request, so `requests("board")` is 1 and the read fails.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![Refusal::secondary_forbidden().after(1)]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_budget_ms":10_000}),
    );
    let started = Instant::now();
    let page = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry after the hinted wait");
    let waited = started.elapsed();
    assert_eq!(
        page.items.len(),
        1,
        "the retry returned the board's own item"
    );
    assert!(
        waited >= Duration::from_secs(1),
        "the hinted second was not waited out: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        2,
        "the refused read was not retried"
    );
}

#[tokio::test]
async fn a_refusal_with_no_hint_is_retried_on_a_growing_schedule() {
    // A refusal carrying no hint had no schedule at all. The assertion that catches a
    // constant one: three waits of a flat 80 ms are 240 ms, and a doubling 80/160/320 is
    // 560 ms, so the floor below is above anything but growth.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![
        Refusal::secondary_forbidden(),
        Refusal::secondary_forbidden(),
        Refusal::secondary_forbidden(),
    ]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":80,"retry_budget_ms":10_000}),
    );
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the fourth attempt, once the board stopped refusing");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(80 + 160 + 320),
        "the schedule did not grow: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        4,
        "three refusals were not each retried"
    );
}

#[tokio::test]
async fn a_limiter_that_never_lets_up_ends_in_a_diagnostic_rather_than_an_unbounded_wait() {
    // Bounded, and what the bound reports. An operator who reads a secondary refusal as a
    // primary one polls `gh api rate_limit`, sees budget, and retries harder — which
    // extends the limit — so the diagnostic has to say which limiter, what this source was
    // doing, and that the endpoint reporting the primary budget does not report this one.
    let fixture = board(vec![]);
    fixture.refuse_every_mutation();
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":300}),
    );
    let started = Instant::now();
    let error = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect_err("a limiter that never lets up");
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(10),
        "the bounded schedule did not end: {waited:?}"
    );
    assert!(
        !matches!(error, SourceError::Auth { .. }),
        "an unrelenting limiter was reported as a credential problem: {error:?}"
    );
    let message = refusal(error);
    assert!(
        message.contains("secondary rate limit"),
        "the diagnostic does not name which limiter refused: {message}"
    );
    assert!(
        message.contains("creating an issue"),
        "the diagnostic does not say what this source was doing: {message}"
    );
    assert!(
        message.contains("gh api rate_limit") && message.contains("does not report this one"),
        "the diagnostic does not say the primary endpoint is silent about this: {message}"
    );
    assert!(
        message.contains("2 refusals"),
        "the diagnostic does not say a wait was taken at all: {message}"
    );
    assert!(
        !message.contains("Projects and Issues read/write"),
        "the diagnostic still sends the operator to re-scope a token: {message}"
    );
}

#[tokio::test]
async fn a_read_refused_past_the_budget_reports_it_rather_than_hanging() {
    // The same bound over a read, and against a limiter that answers with a *successful*
    // response — the shape that used to reach `Refused` carrying GitHub's own sentence and
    // nothing about what it meant.
    let secondary_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"message":"You have exceeded a secondary rate limit"}]})
            .to_string(),
    };
    let source = paced(
        &always(&secondary_in_a_success),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":40,"retry_budget_ms":200}),
    );
    let started = Instant::now();
    let message = refusal(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a limiter that never lets up"),
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the bounded schedule did not end"
    );
    assert!(
        message.contains("reading the board") && message.contains("secondary rate limit"),
        "{message}"
    );
}

#[tokio::test]
async fn a_project_copy_reads_the_board_and_the_repository_once_for_the_whole_command() {
    // Six items, so a source reading the board per item written reads it seven times and
    // the repository six — which is the burst this counts, not the writes themselves.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("the project issue");
    for step in 0..5 {
        let mut child = task(
            &format!("T-{step}"),
            &format!("step {step}"),
            status(StatusCategory::Todo, "Todo"),
        );
        child.project = Some(plan.clone());
        source.write_task(&write(child)).await.expect("a task");
    }
    assert_eq!(
        fixture.requests("board"),
        1,
        "the board was re-read per item written"
    );
    assert_eq!(
        fixture.requests("repository"),
        1,
        "the destination repository was re-resolved per issue created"
    );
    assert_eq!(
        fixture.item(&plan.0).sub_issues,
        5,
        "and the copy still filed every task under its project"
    );
}

#[tokio::test]
async fn the_board_this_command_reads_holds_what_this_command_has_itself_written() {
    // The half a cache gets wrong. GitHub's project items are eventually consistent, so a
    // board read is completed from what this run created; a cache that served the snapshot
    // taken *before* those writes would break exactly what that completion exists to fix.
    //
    // Both halves are asserted: an item created after the cached read is depended on by a
    // later one and resolves, and an item that was already on the board reads back through
    // the source with what the second write gave it rather than what it had before.
    let fixture = board(vec![Item::issue("I_old", "already there").status("Todo")]);
    let source = source(&fixture);
    let first = source
        .write_task(&write(task(
            "T-1",
            "the one depended on",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("the first task");
    let second = task(
        "T-2",
        "the one that depends",
        status(StatusCategory::Todo, "Todo"),
    );
    let landed = source
        .write_task(&ItemWrite {
            target: None,
            item: second,
            depends_on: vec![edge(("T-2", ItemKind::Task), (&first.0, ItemKind::Task))],
        })
        .await
        .expect("an item created earlier in this command resolves as a far end");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == first.0.as_str()),
        "the dependency on the item this command created was not recorded: {:?}",
        fixture.seen()
    );

    let mut revised = task("T", "renamed", status(StatusCategory::Todo, "Todo"));
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_old".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("a second write of an item that was already on the board");
    assert_eq!(
        source
            .get_task(&NativeId("I_old".to_owned()))
            .await
            .unwrap()
            .expect("the item is still there")
            .title,
        "renamed",
        "this command's own view of the board went stale after it wrote to it"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "and it took one board read to answer all of that"
    );
    assert!(landed.0.starts_with("I_new"));
}

#[test]
fn the_shipped_pacing_defaults_are_githubs_published_limits() {
    assert_eq!(onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS, 750);
    assert_eq!(
        60_000 / onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS,
        80,
        "the shipped interval is no longer the published per-minute limit"
    );
    // The other two defaults, because the constant is where each one's reasoning is
    // written down and a value that drifts from it makes that reasoning a lie.
    assert_eq!(onetaskgraph_github_projects::RETRY_BACKOFF_MS, 1_000);
    let mut wait = onetaskgraph_github_projects::RETRY_BACKOFF_MS;
    let mut doublings = 0_u32;
    while wait < 60_000 {
        wait *= 2;
        doublings += 1;
    }
    assert_eq!(
        doublings, 6,
        "the shipped backoff no longer reaches a minute in six waits, which is what its \
         own reasoning claims for it"
    );
    assert_eq!(onetaskgraph_github_projects::RETRY_BUDGET_MS, 120_000);
    const {
        assert!(
            onetaskgraph_github_projects::RETRY_BUDGET_MS > 0
                && onetaskgraph_github_projects::RETRY_BUDGET_MS
                    <= onetaskgraph_github_projects::MAX_PACING_MS,
            "a shipped budget of zero would never wait and one past the cap could not be \
             configured, and the bound is what makes the wait a wait rather than a hang"
        );
    }
}

#[tokio::test]
async fn the_shipped_backoff_is_what_an_unhinted_refusal_waits_when_nothing_is_configured() {
    // Every other limiter test states its own backoff, so the path a board on github.com
    // actually takes — the one where the configuration says nothing and `Pacing::resolve`
    // falls back to `RETRY_BACKOFF_MS` — was the one path never driven against a refusal.
    // The window below is read from the constant on purpose: what this proves is the
    // *wiring*, that the defaulted path waits the shipped backoff and not some other
    // number, and the test above is what pins the number itself.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![Refusal::secondary_forbidden()]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    let shipped = Duration::from_millis(onetaskgraph_github_projects::RETRY_BACKOFF_MS);
    let started = Instant::now();
    let page = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry the shipped backoff schedules");
    let waited = started.elapsed();
    assert_eq!(
        page.items.len(),
        1,
        "the retry returned the board's own item"
    );
    assert_eq!(
        fixture.requests("board"),
        2,
        "the unhinted refusal was not retried at the shipped default"
    );
    assert!(
        waited >= shipped && waited < shipped * 2,
        "the defaulted path did not wait the shipped backoff of {shipped:?}: {waited:?}"
    );
}

#[tokio::test]
async fn the_shipped_budget_bounds_a_wait_no_configuration_asked_for() {
    // The other half of the defaulted path: a hint this source cannot afford. GitHub is
    // entitled to ask for longer than one call may spend waiting, and what bounds that at
    // the shipped defaults is `RETRY_BUDGET_MS` alone. A source defaulting to an unbounded
    // budget honours the hint instead and sits here for two minutes; so does one whose
    // default budget is longer than the hint. Neither reaches the assertions below.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    let past_the_budget = onetaskgraph_github_projects::RETRY_BUDGET_MS / 1_000 + 1;
    fixture.script(vec![Refusal::secondary_forbidden().after(past_the_budget)]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    let started = Instant::now();
    let error = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a hint past the shipped budget");
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(5),
        "the shipped budget did not bound a hint of {past_the_budget}s: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "a wait it could not afford was taken anyway"
    );
    let message = refusal(error);
    assert!(
        message.contains(&format!(
            "{:.1}s one call may spend waiting",
            onetaskgraph_github_projects::RETRY_BUDGET_MS as f64 / 1_000.0
        )),
        "the diagnostic does not name the shipped budget that bounded it: {message}"
    );
    assert!(
        message.contains("secondary rate limit") && message.contains("reading the board"),
        "the bounded refusal stopped naming which limiter and what this source was doing: \
         {message}"
    );
}

#[tokio::test]
async fn content_creating_mutations_leave_this_source_no_faster_than_the_shipped_rate() {
    // Driven at the *shipped* default rather than a configured one, because the shipped
    // default is what a board on github.com meets. A source with no pacing at all sends
    // these four mutations inside a millisecond of each other, so every gap below fails.
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("one task");
    let gaps = fixture.mutation_gaps();
    assert!(
        gaps.len() >= 3,
        "a created task is several mutations, and this saw {}",
        gaps.len() + 1
    );
    let floor = Duration::from_millis(onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS);
    // The source spaces the moments it *releases* a mutation, and what a fixture on the
    // far side of a socket sees is the moments they arrive; the two differ by however long
    // each request spent in transit, which varies by a millisecond or so either way. So
    // the floor is the shipped interval less a tolerance for that, which is still three
    // orders of magnitude above the arrival gap of a source that paces nothing.
    let tolerance = Duration::from_millis(10);
    assert!(
        gaps.iter().all(|gap| *gap >= floor - tolerance),
        "a mutation left this source faster than the shipped rate: {gaps:?}"
    );
    assert!(
        gaps.iter().sum::<Duration>() >= floor * u32::try_from(gaps.len()).unwrap() - tolerance,
        "the mutations of one write did not cost the shipped rate between them: {gaps:?}"
    );
    // And reads are not paced: pacing what the secondary limiter does not count would
    // charge every listing for a limit it cannot trip.
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a read");
    assert!(
        started.elapsed() < floor,
        "a read was paced as though it created content"
    );
}

#[tokio::test]
async fn a_copy_of_a_project_of_many_tasks_is_not_refused_by_a_board_enforcing_that_rate() {
    // The whole point, end to end: a board which refuses any mutation arriving too soon
    // after the one before it, and a copy of a project holding many tasks which is never
    // refused by it. `retry_budget_ms: 0` is deliberate — nothing here may be rescued by a
    // retry, so what completes the copy is the pacing and only the pacing.
    //
    // The board's threshold sits a little under the source's own interval on purpose: the
    // source spaces its *releases*, and what the board measures is arrivals, so the gap
    // absorbs loopback jitter. An unpaced source arrives at roughly zero spacing and is
    // refused on its second mutation.
    let fixture = board(vec![]);
    fixture.rate_limit_mutations(Duration::from_millis(45));
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":60,"retry_budget_ms":0}),
    );
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("the project issue was not refused");
    for step in 0..8 {
        let mut child = task(
            &format!("T-{step}"),
            &format!("step {step}"),
            status(StatusCategory::Todo, "Todo"),
        );
        child.project = Some(plan.clone());
        source
            .write_task(&write(child))
            .await
            .unwrap_or_else(|error| panic!("task {step} was refused: {error}"));
    }
    assert_eq!(
        fixture.too_fast(),
        0,
        "the board refused mutations for arriving too fast"
    );
    assert_eq!(fixture.item(&plan.0).sub_issues, 8);
    assert!(
        fixture.mutation_gaps().len() >= 30,
        "a project of eight tasks is far more than a handful of mutations"
    );
}

#[test]
fn a_pacing_setting_that_would_not_pace_is_refused_when_the_source_is_built() {
    // A zero backoff with a budget to spend is a schedule of zero-length waits: it
    // consumes none of the budget, so the loop that ends when the budget runs out never
    // ends. And every setting is bounded, because a wait budget past that bound is the
    // unbounded wait this mechanism exists to replace.
    let refused = build_refusal(json!({"owner":"octo-org","project_number":7,
        "pacing":{"retry_backoff_ms":0,"retry_budget_ms":5000}}));
    assert!(
        refused.contains("retry_backoff_ms")
            && refused.contains("retry_budget_ms")
            && refused.contains("forever"),
        "{refused}"
    );
    for field in [
        "min_mutation_interval_ms",
        "retry_backoff_ms",
        "retry_budget_ms",
    ] {
        let refused = build_refusal(json!({"owner":"octo-org","project_number":7,
            "pacing":{field: onetaskgraph_github_projects::MAX_PACING_MS + 1}}));
        assert!(
            refused.contains(field) && refused.contains("an hour"),
            "{field}: {refused}"
        );
    }
    // And a zero backoff beside a zero budget is not a schedule at all — it reports the
    // first refusal, which is what every fixture-driven test here asks for.
    assert!(
        Plugin
            .build(
                &SourceName::new("work").unwrap(),
                &json!({"owner":"octo-org","project_number":7,
                        "pacing":{"retry_backoff_ms":0,"retry_budget_ms":0}}),
                &Secrets,
            )
            .is_ok()
    );
}

#[tokio::test]
async fn the_primary_budget_is_waited_out_and_then_reported_as_the_rate_limit_it_is() {
    // The other limiter. Both report as `SourceError::RateLimited`, because that is what
    // happened; what tells them apart is the message, and the primary one sends the
    // operator to the endpoint that really does report it. It reaches this source in two
    // shapes of its own: the `x-ratelimit-remaining: 0` header, and a successful response
    // whose GraphQL errors name it.
    let exhausted = Refusal {
        status: "403 Forbidden",
        headers: "x-ratelimit-remaining: 0\r\n".to_owned(),
        body: "{}".to_owned(),
    };
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![exhausted.clone()]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":60,"retry_budget_ms":5_000}),
    );
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry once the budget was no longer reported spent");
    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "no wait taken"
    );
    assert_eq!(fixture.requests("board"), 2, "the read was not retried");

    // Unrelenting, it reports as a rate limit carrying the wait GitHub asked for.
    let error = paced(
        &always(&Refusal {
            status: "429 Too Many Requests",
            headers: "retry-after: 30\r\n".to_owned(),
            body: "{}".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("an exhausted primary budget");
    let SourceError::RateLimited {
        retry_after_seconds: Some(30),
        message: Some(said),
    } = &error
    else {
        panic!("the primary budget reported as {error:?}");
    };
    assert!(
        said.contains("primary API rate limit") && said.contains("`gh api rate_limit` reports"),
        "the primary limit does not send the operator to the endpoint that reports it: {said}"
    );

    // And in the shape that arrives as a successful response.
    let named_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"type":"RATE_LIMITED",
                                "message":"API rate limit exceeded for user ID 1."}]})
        .to_string(),
    };
    let error = paced(&always(&named_in_a_success), no_waiting())
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a primary rate limit named in a successful response");
    let SourceError::RateLimited {
        retry_after_seconds: None,
        message: Some(said),
    } = &error
    else {
        panic!("a primary rate limit inside a successful response reported as {error:?}");
    };
    assert!(said.contains("primary API rate limit"), "{said}");
}

#[tokio::test]
async fn a_wait_hint_of_nothing_is_still_a_wait_and_still_ends() {
    // GitHub really does answer `retry-after: 0`. Honoured literally it is a retry with no
    // wait at all, which spends none of the budget — so the schedule would never end, and
    // retrying at once is the one move that extends a secondary limit. A source honouring
    // it literally hangs here rather than failing.
    let unrelenting = Refusal::secondary_forbidden().after(0);
    let source = paced(
        &always(&unrelenting),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":200}),
    );
    let started = Instant::now();
    let message = refusal(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a hint of nothing, from a limiter that never lets up"),
    );
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "it never ended: {took:?}");
    assert!(
        took >= Duration::from_millis(50),
        "the hint of nothing was honoured literally: {took:?}"
    );
    assert!(message.contains("secondary rate limit"), "{message}");
}

#[tokio::test]
async fn an_exhausted_budget_reports_when_it_comes_back_rather_than_burning_the_wait_on_it() {
    // `x-ratelimit-reset` is the primary budget's own hint, spelled as the moment it
    // refills rather than as a wait. A reset an hour out is past anything one command may
    // spend waiting, so this reports at once — carrying that wait — instead of sitting in
    // the schedule for a limit that will not lift inside it.
    let refills_in_an_hour = chrono::Utc::now().timestamp() + 3_600;
    let source = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: format!(
                "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {refills_in_an_hour}\r\n"
            ),
            body: "{}".to_owned(),
        }),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let started = Instant::now();
    let error = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a budget that does not come back inside the wait");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it waited on a reset it could never reach"
    );
    let SourceError::RateLimited {
        retry_after_seconds: Some(seconds),
        ..
    } = error
    else {
        panic!("an exhausted primary budget reported as {error:?}");
    };
    assert!(
        (3_500..=3_600).contains(&seconds),
        "the reset was not reported as the wait it is: {seconds}"
    );
}

#[tokio::test]
async fn a_board_holding_work_about_rate_limits_is_not_read_as_a_rate_limit() {
    // A board is where people write about their own work, and this product's own board
    // holds tasks named after the very wordings a refusal carries. Matched across the raw
    // response text — which is where a forbidden status really does carry them — a
    // perfectly good answer would become a refusal this source then waited out and
    // reported. So classification reads what a response says about *itself* and never the
    // work it carries, and a source matching the whole body fails here on both counts.
    let fixture = board(vec![
        Item::issue("I_1", "You have exceeded a secondary rate limit").status("Todo"),
        Item::issue("I_2", "triage the abuse detection mechanism").body("API rate limit exceeded"),
    ]);
    let source = source(&fixture);
    let listed = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a board whose items are about rate limits is still a board");
    assert_eq!(listed.items.len(), 2);
    assert_eq!(
        fixture.requests("board"),
        1,
        "an answer was retried as though it were a refusal"
    );
    source
        .write_task(&write(task(
            "T-1",
            "You have exceeded a secondary rate limit",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("and writing one is not a refusal either");
}

#[tokio::test]
async fn a_mutation_refused_for_a_rate_limit_is_retried_and_lands() {
    // The recovery a copy actually needs: a refused mutation never ran, so replaying it is
    // safe, and the item it was creating ends up on the board rather than the copy ending
    // half done. A source that reports a refused mutation instead of retrying it leaves
    // the board empty here.
    let fixture = board(vec![]);
    fixture.script_for("createIssue", vec![Refusal::secondary_forbidden()]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":60,"retry_budget_ms":5_000}),
    );
    let landed = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("the write past a refusal it waited out");
    assert!(fixture.holds(&landed.0), "the retried write did not land");
    assert_eq!(
        fixture.item(&landed.0).title,
        "Publish",
        "and it landed with what it was given"
    );
    assert_eq!(
        fixture.requests("createIssue"),
        2,
        "the refused creation was not retried"
    );
    assert_eq!(
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "createIssue")
            .count(),
        1,
        "a refused call never ran, so exactly one creation reached the board"
    );
}

#[tokio::test]
async fn every_wording_github_refuses_a_burst_with_is_read_as_the_secondary_limiter() {
    // GitHub has renamed this limiter and reworded its refusal more than once, and the
    // older wordings still come back from some endpoints. Each is a separate arm of the
    // classification, so each is driven rather than one standing in for the rest — and a
    // wording GitHub sends that this source does not know is a credential problem again.
    for said in [
        "You have exceeded a secondary rate limit and have been temporarily blocked from \
         content creation.",
        "You have exceeded a secondary rate limit. Please wait a few minutes.",
        "You have triggered an abuse detection mechanism.",
        "You have exceeded a secondary rate limit for this endpoint.",
        "Your request was submitted too quickly. Please wait and try again.",
    ] {
        let error = paced(
            &always(&Refusal {
                status: "403 Forbidden",
                headers: String::new(),
                body: json!({ "message": said }).to_string(),
            }),
            no_waiting(),
        )
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err(said);
        let SourceError::RateLimited {
            message: Some(diagnostic),
            ..
        } = &error
        else {
            panic!("{said:?} was not read as a rate limit: {error:?}");
        };
        assert!(diagnostic.contains("secondary rate limit"), "{diagnostic}");
    }

    // A failing response that is not JSON at all — a proxy's own page, say. There is
    // nothing structured to read, so the text it did send is what classification has.
    let error = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: "<html><body>You have exceeded a secondary rate limit</body></html>".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("a secondary limit that did not arrive as JSON");
    assert!(
        matches!(error, SourceError::RateLimited { .. }),
        "a non-JSON refusal naming the limiter reported as {error:?}"
    );

    // And the same shape saying nothing about a limit is still the credential it is.
    let error = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: "<html><body>Forbidden</body></html>".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("a plain forbidden page");
    assert!(
        matches!(error, SourceError::Auth { .. }),
        "a forbidden response saying nothing about a limit reported as {error:?}"
    );
}

#[tokio::test]
async fn the_diagnostic_names_whichever_call_the_limiter_caught() {
    // "what the source was doing" is per operation, and a copy is many of them — the one
    // an operator needs named is whichever was refused, not whichever happens to come
    // first. Each is refused on its own, by name, against a board that answers the rest.
    let cases: Vec<(&str, &str)> = vec![
        ("repository", "reading the destination repository"),
        ("createIssue", "creating an issue"),
        ("addProjectV2ItemById", "adding an issue to the board"),
        ("updateProjectV2ItemFieldValue", "writing a board field"),
        ("addSubIssue", "filing an issue under its project"),
        ("addBlockedBy", "recording a dependency"),
    ];
    for (operation, doing) in cases {
        let fixture = board(vec![Item::issue("I_far", "the far end").status("Todo")]);
        fixture.script_for(operation, vec![Refusal::secondary_forbidden()]);
        let source = paced(&fixture.endpoint, no_waiting());
        let plan = source
            .write_project(&write(project(
                "P-1",
                "Published roadmap",
                status(StatusCategory::InProgress, "In Progress"),
            )))
            .await;
        // A project write is `repository`, `createIssue`, `addProjectV2ItemById` and the
        // board fields; the two dependency operations need an item with a far end, so
        // those reach the refusal through the task written under the project instead.
        let error = match plan {
            Err(error) => error,
            Ok(plan) => {
                let mut child = task("T-1", "a step", status(StatusCategory::Todo, "Todo"));
                child.project = Some(plan);
                source
                    .write_task(&ItemWrite {
                        target: None,
                        item: child,
                        depends_on: vec![edge(("T-1", ItemKind::Task), ("I_far", ItemKind::Task))],
                    })
                    .await
                    .expect_err(operation)
            }
        };
        let SourceError::RateLimited {
            message: Some(said),
            ..
        } = &error
        else {
            panic!("{operation} refused for a rate limit reported as {error:?}");
        };
        assert!(
            said.contains(doing),
            "a limiter that caught {operation} says {said:?} rather than naming {doing:?}"
        );
    }
}

#[tokio::test]
async fn spending_the_last_of_the_budget_still_answers_rather_than_refusing() {
    // GitHub sets `x-ratelimit-remaining: 0` on the last request the budget allowed as
    // well as on the ones it then refuses. Reading the header alone made that successful
    // read a rate-limit failure — throwing away an answer it already had — and, once
    // refusals were retried, replayed a request that had already taken effect. A response
    // is a refusal because of its status or its own wording; a spent budget only explains
    // one.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.spend_the_budget_on("board");
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let answered = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a response that spent the last of the budget is still a response");
    assert_eq!(
        answered.items.len(),
        1,
        "the answer that response already carried was thrown away"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "a request that had already taken effect was replayed"
    );

    // A write, where replaying is the half that costs something: the creation runs once
    // and exactly one issue ends up on the board.
    let fixture = board(vec![]);
    fixture.spend_the_budget_on("createIssue");
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let landed = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a write whose creation spent the last of the budget");
    assert_eq!(
        fixture.requests("createIssue"),
        1,
        "an issue creation that had already taken effect was sent a second time"
    );
    assert!(fixture.holds(&landed.0));
    assert_eq!(
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "createIssue")
            .count(),
        1,
        "and the board holds exactly the one issue that was asked for"
    );
}
