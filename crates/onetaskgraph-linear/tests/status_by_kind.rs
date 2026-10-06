//! A Linear source's `status_mapping` and its resolution cache, over real HTTP.
//!
//! Every test drives the plugin's own `TaskSource` against a stateful loopback Linear: a real
//! HTTP server holding one team's workflow states, the workspace's project statuses, issues and
//! projects, answering the GraphQL documents the plugin declares, and recording every request
//! it is sent. What a test asserts is what that server was sent and what it holds afterwards.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use onetaskgraph_linear::graphql;
use onetaskgraph_plugin_api::{
    ItemWrite, MetadataKey, NativeId, Project, SecretResolver, SourceError, SourceName,
    SourcePlugin, Status, StatusCategory, TaskSource, TaskUpdate,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        Some("fixture-key".into())
    }
}

/// One Linear workspace as the loopback server holds it.
#[derive(Default)]
struct Held {
    team: String,
    states: Vec<Value>,
    statuses: Vec<Value>,
    issues: Vec<Value>,
    projects: Vec<Value>,
    /// Every request answered, as its document and its variables.
    served: Vec<(String, Value)>,
    /// Refuse the next mutation, as Linear refuses one it will not take.
    refuse_next_mutation: bool,
    /// Close the next connection without answering, as a network failure does.
    drop_next: bool,
    /// Answer the next resolution without its team's states, as a malformed answer would.
    malformed_next_resolution: bool,
    /// How an `issueUpdate` naming no issue is refused.
    missing: Missing,
    /// Say the team's states run on past the page the resolution reads.
    more_states: bool,
}

/// The spellings of Linear's refusal of a mutation naming nothing, each recognised on its own.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// `Entity not found: Issue`, with its user-presentable sentence, on an HTTP 200.
    #[default]
    Both,
    /// The message alone.
    MessageOnly,
    /// A category message, and the user-presentable sentence alone.
    SentenceOnly,
    /// Both, on an HTTP 400.
    Http400,
}

#[derive(Clone)]
struct Workspace(Arc<Mutex<Held>>);

impl Workspace {
    /// A workspace whose team `team` holds `states` — id, name, type — and whose workspace
    /// holds the project `statuses` — id, name, type.
    fn new(team: &str, states: &[(&str, &str, &str)], statuses: &[(&str, &str, &str)]) -> Self {
        Self(Arc::new(Mutex::new(Held {
            team: team.to_owned(),
            states: states
                .iter()
                .map(|(id, name, kind)| json!({"id":id,"name":name,"type":kind}))
                .collect(),
            statuses: statuses
                .iter()
                .enumerate()
                .map(|(at, (id, name, kind))| {
                    json!({"id":id,"name":name,"type":kind,"position":at as f64})
                })
                .collect(),
            ..Held::default()
        })))
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        self.0.lock().unwrap()
    }

    /// Hold one issue at the state `state` names.
    fn issue(&self, id: &str, state: &str, description: Option<&str>) {
        let mut held = self.held();
        let state = held
            .states
            .iter()
            .find(|held| held["name"] == state)
            .map(|held| json!({"name":held["name"],"type":held["type"]}))
            .expect("a state of the team");
        held.issues.push(json!({"id":id,"identifier":format!("ENG-{id}"),"title":format!("Issue {id}"),
            "description":description,"url":null,"createdAt":null,"updatedAt":null,"archivedAt":null,
            "state":state,"priority":0,"labels":{"nodes":[]},"project":null}));
    }

    /// Hold one issue at the state `state` names, filed under the project `project`.
    fn issue_in(&self, id: &str, state: &str, project: &str) {
        self.issue(id, state, None);
        if let Some(issue) = self
            .held()
            .issues
            .iter_mut()
            .find(|issue| issue["id"] == id)
        {
            issue["project"] = json!({"id": project});
        }
    }

    /// Put one held issue in the trash, as Linear's `issueDelete` does.
    fn trash(&self, id: &str) {
        if let Some(issue) = self
            .held()
            .issues
            .iter_mut()
            .find(|issue| issue["id"] == id)
        {
            issue["archivedAt"] = json!("2026-10-05T00:00:00Z");
        }
    }

    /// Hold one project at the status `status` names.
    fn project(&self, id: &str, status: &str) {
        let mut held = self.held();
        let status = held
            .statuses
            .iter()
            .find(|held| held["name"] == status)
            .map(|held| json!({"name":held["name"],"type":held["type"]}))
            .expect("a project status of the workspace");
        held.projects.push(
            json!({"id":id,"name":format!("Project {id}"),"description":null,
            "url":null,"createdAt":null,"updatedAt":null,"archivedAt":null,"status":status,
            "labels":{"nodes":[]}}),
        );
    }

    fn add_state(&self, id: &str, name: &str, kind: &str) {
        self.held()
            .states
            .push(json!({"id":id,"name":name,"type":kind}));
    }

    fn add_status(&self, id: &str, name: &str, kind: &str) {
        self.held()
            .statuses
            .push(json!({"id":id,"name":name,"type":kind,"position":9.0}));
    }

    /// The state an issue is at, by name.
    fn state_of(&self, id: &str) -> Option<String> {
        self.held()
            .issues
            .iter()
            .find(|issue| issue["id"] == id)
            .and_then(|issue| issue["state"]["name"].as_str().map(str::to_owned))
    }

    /// The status a project is at, by name.
    fn status_of(&self, id: &str) -> Option<String> {
        self.held()
            .projects
            .iter()
            .find(|project| project["id"] == id)
            .and_then(|project| project["status"]["name"].as_str().map(str::to_owned))
    }

    fn description_of(&self, id: &str) -> Option<String> {
        self.held()
            .issues
            .iter()
            .find(|issue| issue["id"] == id)
            .and_then(|issue| issue["description"].as_str().map(str::to_owned))
    }

    /// How many requests this workspace has answered.
    fn count(&self) -> usize {
        self.held().served.len()
    }

    /// Every request answered after the `from`th, by the document's name in `graphql`, and
    /// its variables.
    fn since(&self, from: usize) -> Vec<(&'static str, Value)> {
        self.held().served[from..]
            .iter()
            .map(|(query, variables)| (document_name(query), variables.clone()))
            .collect()
    }

    /// The document names alone, of every request answered after the `from`th.
    fn names_since(&self, from: usize) -> Vec<&'static str> {
        self.since(from).into_iter().map(|(name, _)| name).collect()
    }

    /// Serve this workspace over HTTP, answering until the test ends.
    fn serve(&self) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
        let workspace = self.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                if std::mem::take(&mut workspace.held().drop_next) {
                    drop(stream);
                    continue;
                }
                let answer = workspace.answer(&request);
                let status = if answer.get("errors").is_some()
                    && workspace.held().missing == Missing::Http400
                {
                    "400 Bad Request"
                } else {
                    "200 OK"
                };
                let body = answer.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        endpoint
    }

    fn answer(&self, request: &Value) -> Value {
        let query = request["query"].as_str().unwrap_or_default().to_owned();
        let variables = request["variables"].clone();
        let mut held = self.held();
        held.served.push((query.clone(), variables.clone()));
        if query.trim_start().starts_with("mutation")
            && std::mem::take(&mut held.refuse_next_mutation)
        {
            return json!({"errors":[{"message":"Linear could not complete that mutation"}]});
        }
        let id = variables["id"].as_str().unwrap_or_default().to_owned();
        let input = &variables["input"];
        let data = match query.as_str() {
            graphql::RESOLUTION if std::mem::take(&mut held.malformed_next_resolution) => {
                json!({"teams":{"nodes":[{"id":format!("TEAM-{}", held.team)}]},
                       "projectStatuses":{"nodes":held.statuses,"pageInfo":{"hasNextPage":false}}})
            }
            graphql::RESOLUTION => {
                let teams = if variables["key"] == json!(held.team) {
                    json!([{"id":format!("TEAM-{}", held.team),"states":{"nodes":held.states,"pageInfo":{"hasNextPage":held.more_states}}}])
                } else {
                    json!([])
                };
                json!({"teams":{"nodes":teams},"projectStatuses":{"nodes":held.statuses,"pageInfo":{"hasNextPage":false}}})
            }
            graphql::ISSUE => {
                json!({"issue": held.issues.iter().find(|issue| issue["id"] == id)})
            }
            graphql::PROJECT => {
                json!({"project": held.projects.iter().find(|project| project["id"] == id)})
            }
            graphql::ISSUE_RELATIONS | graphql::PROJECT_RELATIONS => {
                let root = if query == graphql::ISSUE_RELATIONS {
                    "issue"
                } else {
                    "project"
                };
                let empty = json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
                json!({(root): {"description":null,"relations":empty,"inverseRelations":empty}})
            }
            graphql::ISSUE_UPDATE_READ | graphql::ISSUE_UPDATE | graphql::ISSUE_REWRITE => {
                let state = match input.get("stateId").and_then(Value::as_str) {
                    Some(state) => match held.states.iter().find(|held| held["id"] == state) {
                        Some(held) => Some(json!({"name":held["name"],"type":held["type"]})),
                        // As Linear refuses a reference to an entity it does not hold.
                        None => {
                            return json!({"errors":[{"message":"Entity not found: WorkflowState",
                                "extensions":{"code":"INVALID_INPUT",
                                    "userPresentableMessage":"Could not find referenced WorkflowState."}}]});
                        }
                    },
                    None => None,
                };
                let missing = held.missing;
                let Some(issue) = held.issues.iter_mut().find(|issue| issue["id"] == id) else {
                    return json!({"errors":[match missing {
                        Missing::MessageOnly => json!({"message":"Entity not found: Issue"}),
                        Missing::SentenceOnly => json!({"message":"Argument Validation Error",
                            "extensions":{"code":"INVALID_INPUT",
                                "userPresentableMessage":"Could not find referenced Issue."}}),
                        Missing::Both | Missing::Http400 => json!({"message":"Entity not found: Issue",
                            "extensions":{"code":"INVALID_INPUT",
                                "userPresentableMessage":"Could not find referenced Issue."}}),
                    }]});
                };
                if let Some(state) = state {
                    issue["state"] = state;
                }
                for member in ["title", "description", "priority"] {
                    if let Some(value) = input.get(member) {
                        issue[member] = value.clone();
                    }
                }
                let answered = match query.as_str() {
                    graphql::ISSUE_UPDATE_READ => issue.clone(),
                    graphql::ISSUE_REWRITE => json!({"id":id,"relations":no_relations()}),
                    _ => json!({"id":id}),
                };
                json!({"issueUpdate":{"success":true,"issue":answered}})
            }
            graphql::ISSUE_CREATE => {
                let new = format!("I-NEW-{}", held.issues.len() + 1);
                let state = input["stateId"].as_str().unwrap_or_default().to_owned();
                let Some(state) = held.states.iter().find(|held| held["id"] == state).cloned()
                else {
                    return json!({"errors":[{"message":"stateId is not valid"}]});
                };
                held.issues.push(json!({"id":new,"identifier":"ENG-NEW","title":input["title"],
                    "description":input["description"],"url":null,"createdAt":null,"updatedAt":null,
                    "archivedAt":null,"state":{"name":state["name"],"type":state["type"]},"priority":0,
                    "labels":{"nodes":[]},"project":null}));
                json!({"issueCreate":{"success":true,"issue":{"id":new}}})
            }
            graphql::PROJECT_CREATE | graphql::PROJECT_UPDATE | graphql::PROJECT_REWRITE => {
                let status = input["statusId"].as_str().unwrap_or_default().to_owned();
                let Some(status) = held
                    .statuses
                    .iter()
                    .find(|held| held["id"] == status)
                    .map(|held| json!({"name":held["name"],"type":held["type"]}))
                else {
                    return json!({"errors":[{"message":"statusId is not valid"}]});
                };
                if query == graphql::PROJECT_CREATE {
                    let new = format!("P-NEW-{}", held.projects.len() + 1);
                    held.projects
                        .push(json!({"id":new,"name":input["name"],"description":null,
                        "url":null,"createdAt":null,"updatedAt":null,"archivedAt":null,
                        "status":status,"labels":{"nodes":[]}}));
                    json!({"projectCreate":{"success":true,"project":{"id":new}}})
                } else {
                    let Some(project) =
                        held.projects.iter_mut().find(|project| project["id"] == id)
                    else {
                        return json!({"errors":[{"message":"Entity not found: Project"}]});
                    };
                    project["status"] = status;
                    json!({"projectUpdate":{"success":true,"project":{"id":id,"relations":no_relations()}}})
                }
            }
            graphql::WORKFLOW_STATE_CREATE | graphql::PROJECT_STATUS_CREATE => {
                let new = json!({"id":format!("NEW-{}", input["name"].as_str().unwrap_or_default()),
                    "name":input["name"],"type":input["type"],"position":input["position"]});
                if query == graphql::WORKFLOW_STATE_CREATE {
                    held.states.push(new.clone());
                    json!({"workflowStateCreate":{"success":true,"workflowState":new}})
                } else {
                    held.statuses.push(new.clone());
                    json!({"projectStatusCreate":{"success":true,"status":new}})
                }
            }
            _ => {
                return json!({"errors":[{"message":"this workspace does not answer that document"}]});
            }
        };
        json!({ "data": data })
    }
}

/// A relation connection holding none.
fn no_relations() -> Value {
    json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}})
}

/// One request off the wire: its JSON body, once the whole of it has arrived.
fn read_request(stream: &mut std::net::TcpStream) -> Option<Value> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.trim().parse::<usize>().ok())?;
        if bytes.len() >= split + 4 + length {
            return serde_json::from_slice(&bytes[split + 4..split + 4 + length]).ok();
        }
    }
}

/// The name a request's document has in `graphql`.
fn document_name(query: &str) -> &'static str {
    [
        ("RESOLUTION", graphql::RESOLUTION),
        ("ISSUE", graphql::ISSUE),
        ("PROJECT", graphql::PROJECT),
        ("ISSUE_RELATIONS", graphql::ISSUE_RELATIONS),
        ("PROJECT_RELATIONS", graphql::PROJECT_RELATIONS),
        ("ISSUE_UPDATE_READ", graphql::ISSUE_UPDATE_READ),
        ("ISSUE_UPDATE", graphql::ISSUE_UPDATE),
        ("ISSUE_REWRITE", graphql::ISSUE_REWRITE),
        ("PROJECT_REWRITE", graphql::PROJECT_REWRITE),
        ("ISSUE_CREATE", graphql::ISSUE_CREATE),
        ("PROJECT_CREATE", graphql::PROJECT_CREATE),
        ("PROJECT_UPDATE", graphql::PROJECT_UPDATE),
        ("WORKFLOW_STATE_CREATE", graphql::WORKFLOW_STATE_CREATE),
        ("PROJECT_STATUS_CREATE", graphql::PROJECT_STATUS_CREATE),
    ]
    .into_iter()
    .find(|(_, document)| *document == query)
    .map_or("OTHER", |(name, _)| name)
}

/// The `hellopatient` mapping, exactly as ai-orchestrator configures it.
fn hellopatient() -> Value {
    json!({
        "backlog":     {"task": "Proposed",        "project": "Proposal"},
        "draft":       {"task": "Backlog",         "project": "Idea"},
        "todo":        {"task": "Todo",            "project": "Planned"},
        "queued":      {"task": "Queued",          "project": "Accepted"},
        "in-progress": "In Progress",
        "unknown":     {"task": "Needs Attention", "project": "Blocked"},
        "done":        {"task": "Done",            "project": "Completed"},
        "cancelled":   "Canceled",
    })
}

/// The `hellopatient-followups` mapping, scoped to one project and never writing a project.
fn hellopatient_followups() -> Value {
    json!({
        "backlog": "Proposed", "draft": "Backlog", "todo": "Todo", "queued": "Queued",
        "in-progress": "In Progress", "unknown": "Needs Attention", "done": "Done",
        "cancelled": "Canceled",
    })
}

/// Hello Patient's team states and project statuses, under ids of `prefix`.
fn hello_patient(prefix: &str) -> Workspace {
    let state = |name: &str| format!("{prefix}-state-{name}");
    let status = |name: &str| format!("{prefix}-status-{name}");
    let states = [
        ("Proposed", "backlog"),
        ("Backlog", "backlog"),
        ("Todo", "unstarted"),
        ("Queued", "unstarted"),
        ("In Progress", "started"),
        ("Needs Attention", "started"),
        ("Done", "completed"),
        ("Canceled", "canceled"),
        ("Triage", "triage"),
        ("In Review", "started"),
    ]
    .map(|(name, kind)| (state(name), name, kind));
    let statuses = [
        ("Idea", "backlog"),
        ("Proposal", "backlog"),
        ("Planned", "planned"),
        ("Accepted", "planned"),
        ("In Progress", "started"),
        ("Blocked", "started"),
        ("Completed", "completed"),
        ("Canceled", "canceled"),
    ]
    .map(|(name, kind)| (status(name), name, kind));
    Workspace::new(
        "ENG",
        &states
            .iter()
            .map(|(id, name, kind)| (id.as_str(), *name, *kind))
            .collect::<Vec<_>>(),
        &statuses
            .iter()
            .map(|(id, name, kind)| (id.as_str(), *name, *kind))
            .collect::<Vec<_>>(),
    )
}

fn build(endpoint: &str, extra: Value) -> Result<Box<dyn TaskSource>, SourceError> {
    let mut config = json!({"endpoint": endpoint, "team": "ENG"});
    for (key, value) in extra.as_object().expect("an object") {
        config[key] = value.clone();
    }
    onetaskgraph_linear::Plugin.build(&SourceName::new("work").unwrap(), &config, &Secrets)
}

fn source(workspace: &Workspace, mapping: Value) -> Box<dyn TaskSource> {
    build(&workspace.serve(), json!({"status_mapping": mapping})).unwrap()
}

fn project_write(category: StatusCategory, target: Option<&str>) -> ItemWrite<Project> {
    ItemWrite {
        target: target.map(NativeId::from),
        item: serde_json::from_value(json!({"id":"plan:P","title":"A project","content":null,
            "status":{"category":category,"name":"whatever it was called there"},
            "labels":[],"repositories":[],"metadata":{}}))
        .unwrap(),
        depends_on: Vec::new(),
    }
}

#[test]
fn every_form_of_the_grammar_loads_and_every_malformed_one_is_refused_naming_the_part() {
    for mapping in [
        json!({"todo": "Todo"}),
        json!({"draft": null}),
        json!({"done": {"task": "Done"}}),
        json!({"done": {"project": "Completed"}}),
        json!({"done": {"task": "Done", "project": "Completed"}}),
        // One name, two categories, two kinds.
        json!({"todo": {"task": "Todo"}, "queued": {"project": "Todo"}}),
        hellopatient(),
        hellopatient_followups(),
    ] {
        build(
            "http://127.0.0.1:1",
            json!({"status_mapping": mapping.clone()}),
        )
        .unwrap_or_else(|error| panic!("{mapping} did not load: {error}"));
    }
    build(
        "http://127.0.0.1:1",
        json!({"project": "P-ONE", "status_mapping": hellopatient_followups()}),
    )
    .expect("hellopatient-followups, scoped to one project, loads");
    for (mapping, said) in [
        (
            json!({"doing": "Doing"}),
            "status_mapping names \"doing\", which is not a status category",
        ),
        (
            json!({"done": {}}),
            "status_mapping.done is an empty object, which maps no kind; to disable done for every kind, write null",
        ),
        (
            json!({"done": {"task": "Done", "epic": "Done"}}),
            "status_mapping.done names \"epic\", which is not an item kind",
        ),
        (
            json!({"done": {"task": null}}),
            "status_mapping.done.task is null",
        ),
        (
            json!({"done": {"project": "  "}}),
            "status_mapping.done.project is blank",
        ),
        (json!({"done": ""}), "status_mapping.done is blank"),
        (
            json!({"todo": "Todo", "queued": "TODO"}),
            "status_mapping sends both todo and queued to the task status \"TODO\"",
        ),
        (
            json!({"todo": {"project": "Planned"}, "queued": {"project": "planned"}}),
            "status_mapping sends both todo and queued to the project status \"planned\"",
        ),
        (
            json!({"todo": {"task": "Todo"}, "queued": "todo"}),
            "status_mapping sends both todo and queued to the task status \"todo\"",
        ),
    ] {
        let refused = build(
            "http://127.0.0.1:1",
            json!({"status_mapping": mapping.clone()}),
        )
        .err()
        .unwrap_or_else(|| panic!("{mapping} loaded"))
        .to_string();
        assert!(
            refused.contains("source work") && refused.contains(said),
            "{mapping} was refused with {refused:?}, not naming {said:?}"
        );
    }
}

#[tokio::test]
async fn building_a_source_sends_nothing_and_its_first_status_write_reads_the_resolution_once() {
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    workspace.project("P-1", "Planned");
    let source = source(&workspace, hellopatient());
    assert_eq!(
        workspace.count(),
        0,
        "building a source asks Linear nothing"
    );

    source
        .set_task_status(&"I-1".into(), StatusCategory::InProgress)
        .await
        .unwrap();
    assert_eq!(
        workspace.names_since(0),
        ["RESOLUTION", "ISSUE_UPDATE_READ"]
    );

    // Warm: each status write after the first, of either kind, sends its mutation alone.
    let from = workspace.count();
    source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .unwrap();
    source
        .write_project(&project_write(StatusCategory::Queued, None))
        .await
        .unwrap();
    source
        .write_project(&project_write(StatusCategory::Done, Some("P-1")))
        .await
        .unwrap();
    assert_eq!(
        workspace.names_since(from),
        ["ISSUE_UPDATE_READ", "PROJECT_CREATE", "PROJECT_REWRITE"],
        "no team, state or project-status read after the first, and a rewrite reads the \
         relations it replaces in its own answer"
    );
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Done"));
    assert_eq!(workspace.status_of("P-1").as_deref(), Some("Completed"));

    // A project write first is what reads it, on a fresh instance.
    let fresh = source_on(&workspace);
    let from = workspace.count();
    fresh
        .write_project(&project_write(StatusCategory::Todo, None))
        .await
        .unwrap();
    assert_eq!(
        workspace.names_since(from),
        ["RESOLUTION", "PROJECT_CREATE"]
    );
}

/// Another source over the same server, as a second process would build one.
fn source_on(workspace: &Workspace) -> Box<dyn TaskSource> {
    source(workspace, hellopatient())
}

#[tokio::test]
async fn two_sources_in_one_process_each_resolve_against_their_own_workspace() {
    let (first, second) = (hello_patient("A"), hello_patient("B"));
    for workspace in [&first, &second] {
        workspace.issue("I-1", "Todo", None);
    }
    let (one, two) = (
        source(&first, hellopatient()),
        source(&second, hellopatient()),
    );
    for _ in 0..2 {
        one.set_task_status(&"I-1".into(), StatusCategory::Queued)
            .await
            .unwrap();
        two.set_task_status(&"I-1".into(), StatusCategory::Queued)
            .await
            .unwrap();
    }
    for (workspace, prefix) in [(&first, "A"), (&second, "B")] {
        let sent = workspace.since(0);
        assert_eq!(
            sent.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            ["RESOLUTION", "ISSUE_UPDATE_READ", "ISSUE_UPDATE_READ"],
            "each source read its own resolution once"
        );
        for (_, variables) in &sent[1..] {
            assert_eq!(
                variables["input"]["stateId"],
                json!(format!("{prefix}-state-Queued")),
                "each wrote its own workspace's id"
            );
        }
    }
}

#[tokio::test]
async fn a_name_added_after_the_resolution_is_found_by_one_fresh_read_and_an_absent_one_refused() {
    let workspace = Workspace::new(
        "ENG",
        &[("S-todo", "Todo", "unstarted")],
        &[("PS-planned", "Planned", "planned")],
    );
    workspace.issue("I-1", "Todo", None);
    let source = source(
        &workspace,
        json!({"todo": {"task": "Todo", "project": "Planned"},
               "queued": {"task": "Queued", "project": "Accepted"},
               "done": {"task": "Shipped"}}),
    );
    source
        .set_task_status(&"I-1".into(), StatusCategory::Todo)
        .await
        .unwrap();

    workspace.add_state("S-queued", "Queued", "unstarted");
    workspace.add_status("PS-accepted", "Accepted", "planned");
    let from = workspace.count();
    source
        .set_task_status(&"I-1".into(), StatusCategory::Queued)
        .await
        .unwrap();
    source
        .write_project(&project_write(StatusCategory::Queued, None))
        .await
        .unwrap();
    assert_eq!(
        workspace.names_since(from),
        ["RESOLUTION", "ISSUE_UPDATE_READ", "PROJECT_CREATE"],
        "one fresh read finds both, and the project write after it is warm"
    );
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Queued"));

    // Absent even from a fresh read: refused naming the name, and nothing written.
    let from = workspace.count();
    let refused = source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("\"Shipped\"") && refused.contains("team ENG"),
        "{refused}"
    );
    assert_eq!(workspace.names_since(from), ["RESOLUTION"]);
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Queued"));
}

#[tokio::test]
async fn a_failed_write_leaves_the_next_one_on_the_same_source_resolving_and_landing() {
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    let source = source(
        &workspace,
        json!({"todo": "Todo", "queued": "Queued", "done": "Done", "cancelled": {"task": "Gone"}}),
    );
    source
        .set_task_status(&"I-1".into(), StatusCategory::Todo)
        .await
        .unwrap();

    // Refused by Linear.
    workspace.held().refuse_next_mutation = true;
    source
        .set_task_status(&"I-1".into(), StatusCategory::Queued)
        .await
        .expect_err("Linear refused it");
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Todo"));
    let from = workspace.count();
    source
        .set_task_status(&"I-1".into(), StatusCategory::Queued)
        .await
        .unwrap();
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Queued"));
    assert_eq!(
        workspace.names_since(from),
        ["RESOLUTION", "ISSUE_UPDATE_READ"],
        "what a refused write carried is read again rather than trusted"
    );

    // A network failure at the endpoint.
    workspace.held().drop_next = true;
    let failed = source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .expect_err("nothing answered");
    assert!(
        matches!(failed, SourceError::Unavailable { .. }),
        "{failed:?}"
    );
    source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .unwrap();
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Done"));

    // A miss.
    source
        .set_task_status(&"I-1".into(), StatusCategory::Cancelled)
        .await
        .expect_err("the team has no Gone");
    source
        .set_task_status(&"I-1".into(), StatusCategory::Todo)
        .await
        .unwrap();
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Todo"));
}

#[tokio::test]
async fn a_status_write_reads_nothing_before_its_mutation() {
    let workspace = hello_patient("A");
    workspace.issue("I-TODO", "Todo", None);
    workspace.issue("I-REVIEW", "In Review", None);
    let source = source(&workspace, hellopatient());

    // Already in the category: the same state, sent again.
    let answered = source
        .set_task_status(&"I-TODO".into(), StatusCategory::Todo)
        .await
        .unwrap();
    assert_eq!(
        answered,
        Some(Status {
            category: StatusCategory::Todo,
            name: "Todo".into()
        })
    );
    assert_eq!(
        workspace.since(0),
        [
            ("RESOLUTION", json!({"key": "ENG"})),
            (
                "ISSUE_UPDATE_READ",
                json!({"id": "I-TODO", "input": {"stateId": "A-state-Todo"}})
            ),
        ]
    );

    // An unmapped state reads as `unknown`; setting `unknown` moves it to the mapped name.
    assert_eq!(
        source
            .get_task(&"I-REVIEW".into())
            .await
            .unwrap()
            .unwrap()
            .status,
        Status {
            category: StatusCategory::Unknown,
            name: "In Review".into()
        }
    );
    let from = workspace.count();
    source
        .set_task_status(&"I-REVIEW".into(), StatusCategory::Unknown)
        .await
        .unwrap();
    assert_eq!(
        workspace.since(from),
        [(
            "ISSUE_UPDATE_READ",
            json!({"id": "I-REVIEW", "input": {"stateId": "A-state-Needs Attention"}})
        )]
    );
    assert_eq!(
        source
            .get_task(&"I-REVIEW".into())
            .await
            .unwrap()
            .unwrap()
            .status,
        Status {
            category: StatusCategory::Unknown,
            name: "Needs Attention".into()
        }
    );

    // An issue Linear does not hold is no such task, from the mutation's own refusal.
    let from = workspace.count();
    assert_eq!(
        source
            .set_task_status(&"I-NOPE".into(), StatusCategory::Done)
            .await
            .unwrap(),
        None
    );
    assert_eq!(workspace.names_since(from), ["ISSUE_UPDATE_READ"]);
    assert_eq!(
        source
            .set_task_status_reading(&"I-NOPE".into(), StatusCategory::Done)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_status_only_update_reads_nothing_and_a_settlement_reads_the_issue_once_to_merge_its_slot()
 {
    let workspace = hello_patient("A");
    let held =
        "A person's own words.\n\n<!-- onetaskgraph.metadata `{\"caller.unrelated\":\"kept\"}` -->";
    workspace.issue("I-1", "Todo", Some(held));
    let source = source(&workspace, hellopatient());
    let status = |category| TaskUpdate {
        status: Some(Status {
            category,
            name: "ignored".into(),
        }),
        ..TaskUpdate::default()
    };

    // A status alone: no read before the mutation, and none after it.
    let outcome = source
        .update_task(&"I-1".into(), &status(StatusCategory::InProgress))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(outcome.task.status.name, "In Progress");
    assert_eq!(
        workspace.names_since(0),
        ["RESOLUTION", "ISSUE_UPDATE_READ"]
    );

    // A status and a metadata key: one read, because the slot is merged into what the issue
    // holds, and the person's text and the other key come through byte for byte.
    let from = workspace.count();
    let mut settlement = status(StatusCategory::Done);
    settlement.metadata_set.insert(
        MetadataKey::new("onepipeline.settlement").unwrap(),
        json!({"outcome": "landed"}),
    );
    let outcome = source
        .update_task(&"I-1".into(), &settlement)
        .await
        .unwrap()
        .expect("held");
    assert_eq!(workspace.names_since(from), ["ISSUE", "ISSUE_UPDATE_READ"]);
    assert_eq!(outcome.task.status.name, "Done");
    assert_eq!(outcome.task.metadata["caller.unrelated"], "kept");
    let description = workspace.description_of("I-1").expect("a description");
    assert!(
        description.starts_with("A person's own words.\n\n"),
        "{description}"
    );
    assert!(
        description.contains("\"caller.unrelated\":\"kept\"")
            && description.contains("\"onepipeline.settlement\":{\"outcome\":\"landed\"}"),
        "{description}"
    );
}

#[tokio::test]
async fn a_project_write_its_mapping_names_no_status_for_sends_no_mutation() {
    let workspace = hello_patient("A");
    workspace.project("P-1", "Planned");
    let source = source(
        &workspace,
        json!({"todo": {"task": "Todo"}, "cancelled": null,
               "done": {"task": "Done", "project": "Shipped"}}),
    );
    for (category, said) in [
        (StatusCategory::Draft, "status_mapping.draft.project"),
        (
            StatusCategory::Cancelled,
            "status_mapping.cancelled.project",
        ),
        (StatusCategory::Todo, "status_mapping.todo.project"),
        (StatusCategory::Done, "\"Shipped\""),
    ] {
        for target in [None, Some("P-1")] {
            let from = workspace.count();
            let refused = source
                .write_project(&project_write(category, target))
                .await
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("source work")
                    && refused.contains("project status")
                    && refused.contains(said),
                "{category:?} {target:?}: {refused}"
            );
            assert!(
                workspace
                    .names_since(from)
                    .iter()
                    .all(|name| *name == "RESOLUTION"),
                "{category:?} {target:?}: {:?}",
                workspace.names_since(from)
            );
        }
    }
    assert_eq!(workspace.status_of("P-1").as_deref(), Some("Planned"));
}

#[tokio::test]
async fn each_spelling_of_linears_not_found_refusal_is_no_such_task_and_any_other_is_a_refusal() {
    for missing in [
        Missing::Both,
        Missing::MessageOnly,
        Missing::SentenceOnly,
        Missing::Http400,
    ] {
        let workspace = hello_patient("A");
        workspace.issue("I-1", "Todo", None);
        workspace.held().missing = missing;
        let source = source(&workspace, hellopatient());
        assert_eq!(
            source
                .set_task_status(&"I-NOPE".into(), StatusCategory::Done)
                .await
                .unwrap(),
            None,
            "a missing issue is no such task, however Linear spelled it"
        );
    }
    // A referenced entity Linear does not hold — the state the write names — is a refusal of
    // the write, never no such task.
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    let held = source(&workspace, hellopatient());
    held.set_task_status(&"I-1".into(), StatusCategory::Todo)
        .await
        .unwrap();
    workspace
        .held()
        .states
        .retain(|state| state["name"] != "Done");
    workspace.add_state("A-state-Done-gone", "Done", "completed");
    // The held resolution still carries the old id, which Linear no longer holds.
    workspace
        .held()
        .states
        .retain(|state| state["id"] != "A-state-Done");
    let refused = held
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .expect_err("a state Linear does not hold");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.contains("WorkflowState")),
        "{refused:?}"
    );
    // Any other refusal of the same mutation is a refusal, never no such task.
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    let source = source(&workspace, hellopatient());
    source
        .set_task_status(&"I-1".into(), StatusCategory::Todo)
        .await
        .unwrap();
    workspace.held().refuse_next_mutation = true;
    let refused = source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .expect_err("a refusal that names no missing entity");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.contains("could not complete")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_malformed_resolution_is_not_held_and_the_next_write_reads_it_again() {
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    let source = source(&workspace, hellopatient());
    workspace.held().malformed_next_resolution = true;
    let failed = source
        .set_task_status(&"I-1".into(), StatusCategory::Queued)
        .await
        .expect_err("a resolution without the team's states");
    assert!(
        matches!(failed, SourceError::Malformed { .. }),
        "{failed:?}"
    );
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Todo"));
    let from = workspace.count();
    source
        .set_task_status(&"I-1".into(), StatusCategory::Queued)
        .await
        .unwrap();
    assert_eq!(
        workspace.names_since(from),
        ["RESOLUTION", "ISSUE_UPDATE_READ"],
        "nothing of the malformed answer was held"
    );
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Queued"));
}

#[tokio::test]
async fn a_scoped_source_reads_an_issue_before_its_status_write_and_never_writes_one_elsewhere() {
    let workspace = hello_patient("A");
    workspace.issue_in("I-IN", "Todo", "P-SCOPE");
    workspace.issue_in("I-OUT", "Todo", "P-OTHER");
    let source = build(
        &workspace.serve(),
        json!({"project": "P-SCOPE", "status_mapping": hellopatient()}),
    )
    .unwrap();
    // Its own project's issue: read, because Linear has no update conditional on where an
    // issue is filed, then written.
    let answered = source
        .set_task_status(&"I-IN".into(), StatusCategory::InProgress)
        .await
        .unwrap();
    assert_eq!(
        answered,
        Some(Status {
            category: StatusCategory::InProgress,
            name: "In Progress".into()
        })
    );
    assert_eq!(
        workspace.names_since(0),
        ["RESOLUTION", "ISSUE", "ISSUE_UPDATE_READ"]
    );
    assert_eq!(workspace.state_of("I-IN").as_deref(), Some("In Progress"));
    // Another project's issue is no task of this source: read, and never written.
    let from = workspace.count();
    assert_eq!(
        source
            .set_task_status(&"I-OUT".into(), StatusCategory::InProgress)
            .await
            .unwrap(),
        None
    );
    assert_eq!(workspace.names_since(from), ["ISSUE"]);
    assert_eq!(workspace.state_of("I-OUT").as_deref(), Some("Todo"));
}

#[tokio::test]
async fn a_status_write_to_a_trashed_issue_answers_no_such_task() {
    let workspace = hello_patient("A");
    workspace.issue("I-GONE", "Todo", None);
    workspace.trash("I-GONE");
    let source = source(&workspace, hellopatient());
    // No read before it, so the mutation reaches the trashed issue — Linear takes it — and what
    // its answer reports is an issue this source does not hold.
    assert_eq!(
        source
            .set_task_status(&"I-GONE".into(), StatusCategory::Done)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        workspace.names_since(0),
        ["RESOLUTION", "ISSUE_UPDATE_READ"]
    );
    assert_eq!(
        source
            .set_task_status_reading(&"I-GONE".into(), StatusCategory::Done)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_resolution_that_does_not_fit_one_page_is_refused_rather_than_read_short() {
    let workspace = hello_patient("A");
    workspace.issue("I-1", "Todo", None);
    workspace.held().more_states = true;
    let source = source(&workspace, hellopatient());
    let refused = source
        .set_task_status(&"I-1".into(), StatusCategory::Done)
        .await
        .expect_err("more states than one page");
    assert!(
        refused
            .to_string()
            .contains("workflow states of its team in one page of 250, and Linear holds more"),
        "{refused}"
    );
    assert_eq!(
        workspace.names_since(0),
        ["RESOLUTION"],
        "nothing was written"
    );
    assert_eq!(workspace.state_of("I-1").as_deref(), Some("Todo"));
}
