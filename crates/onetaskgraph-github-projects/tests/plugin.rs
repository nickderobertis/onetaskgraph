//! The source's own suite, driven over a real loopback socket against a board fixture.
//!
//! Nothing here mocks the layer under test: every test builds the plugin through
//! `SourcePlugin::build`, and every request it makes is a real HTTP POST carrying a real
//! GraphQL document, answered by a fixture that keeps board state the way GitHub does.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use onetaskgraph_plugin_api::{
    Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, Direction, ItemKind, ItemWrite,
    Label, NativeId, PageRequest, Project, ProjectQuery, Repository, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, Task, TaskQuery, TaskSource, WriteSupport,
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
    seen: Vec<Value>,
    next: usize,
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
        seen: Vec::new(),
        next: 0,
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
            let data = answer(&served, query, variables);
            let body = json!({ "data": data }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("a response");
        }
    });
    Fixture { endpoint, state }
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

fn operation_name(query: &str) -> &str {
    for name in [
        "createIssue",
        "addProjectV2ItemById",
        "updateIssue",
        "updateProjectV2DraftIssue",
        "updateProjectV2ItemFieldValue",
        "addSubIssue",
        "removeSubIssue",
        "addBlockedBy",
        "removeBlockedBy",
    ] {
        if query.contains(&format!("{name}(input:$input)")) {
            return name;
        }
    }
    "unknown"
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

fn configured(endpoint: &str, extra: Value) -> Box<dyn TaskSource> {
    let mut config = json!({"owner":"octo-org","project_number":7,"endpoint":endpoint,
                            "repository":"acme/work"});
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
async fn the_committed_board_fixture_maps_to_two_projects_one_task_and_no_pull_request() {
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
        ["I_plan", "I_empty"],
        "an issue with sub-issues and a marked empty issue are both projects"
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
        ["I_task"],
        "a sub-issue is the only task, and the pull request is neither"
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
    let capabilities = source.capabilities();
    assert_eq!(capabilities.max_page_size, 100);
    assert_eq!(
        capabilities.project_dependencies,
        onetaskgraph_plugin_api::DependencySupport::BothDirections
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
            raw_server_with_headers("200 OK", "{}", "x-ratelimit-remaining: 0\r\n"),
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
