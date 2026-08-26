//! The one table every journey is written against.
//!
//! A journey is written once and run against **every** source kind, so no plugin is ever
//! proven by a suite of its own writing. A row says which registry plugin it stands for,
//! how to configure one over the shared dataset below, and what that configuration
//! *declares* — which is what lets one journey assert both the rows and the plan against
//! a source that filters natively and one that does not.
//!
//! `scripts/check-journey-matrix.sh` fails, naming the plugin, when a plugin the registry
//! knows has no row here. Every registered plugin is implemented, so every row carries a
//! working source fixture.

use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use crate::common::{Sandbox, SourceBoundary};

/// One row: a source kind, in one configuration, over the shared dataset.
pub struct Row {
    /// The registry plugin kind this row stands for.
    pub plugin: &'static str,
    /// This row's own name, unique across rows, used in failure messages.
    pub name: &'static str,
    /// How to build it.
    pub fixture: Ready,
}

/// Everything a journey needs in order to drive one configured source.
pub struct Ready {
    /// The `config:` block, given a sandbox to write into if the source needs files.
    pub block: fn(&Sandbox) -> Value,
    /// What this configuration declares it applies itself.
    pub declared: Declared,
    /// Whether this source can represent the complete cross-plugin dataset.
    pub complete_dataset: bool,
}

/// What one row's source declares, so a journey can assert the plan as well as the rows.
pub struct Declared {
    /// Whether the source filters by label itself.
    pub filter_by_label: bool,
    /// Whether the source filters by status itself.
    pub filter_by_status: bool,
    /// Whether the source searches titles itself.
    pub search_title: bool,
    /// Whether the source searches bodies itself.
    pub search_content: bool,
    /// Whether the source can select tasks belonging to no project.
    pub orphan_tasks: bool,
    /// Whether the source answers reverse task dependencies itself.
    pub reverse_task_dependencies: bool,
    /// Whether the source answers reverse project dependencies itself.
    pub reverse_project_dependencies: bool,
}

impl Row {
    /// This row as a configuration document naming one source, `work`.
    ///
    /// Written as JSON, which the YAML reader accepts, so a fixture is a value rather
    /// than a string a test has to indent correctly.
    pub fn document(&self, sandbox: &Sandbox) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block}
        }))
    }

    /// This row as a document naming `work` and one writable Markdown folder beside it.
    ///
    /// The copy journeys need a destination, and a folder of Markdown is the one every
    /// row can be copied into: it is the source this plan makes writable, and it is what
    /// the user's own flow writes into and edits.
    pub fn document_with_folder(&self, sandbox: &Sandbox, folder: &str) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block},
            folder: {"plugin": "local-md", "config": empty_folder(sandbox, folder)},
        }))
    }

    /// What this row declares.
    pub fn declared(&self) -> &Declared {
        &self.fixture.declared
    }
}

/// An empty Markdown folder, ready to be copied into.
///
/// The status mapping covers every status name the shared dataset spells, because a
/// destination that would read a written status back as something else refuses the write
/// rather than narrowing it — which is right, and is not what these journeys are about.
pub fn empty_folder(sandbox: &Sandbox, relative: &str) -> Value {
    json!({
        "root": sandbox.subdirectory(relative),
        "status_mapping": {"todo": "todo", "doing": "in-progress", "shipped": "done"},
    })
}

pub fn document(sources: &Value) -> String {
    serde_json::to_string_pretty(&json!({ "sources": sources })).expect("a fixture renders")
}

/// The name every single-source journey configures its source under.
pub const SOURCE: &str = "work";

/// The name the two-source journeys give the row that applies everything itself.
pub const NATIVE: &str = "native";

/// The name they give the row that applies none of it and walks forwards only.
pub const SCANNED: &str = "scanned";

/// The capability pair built on either side of the process boundary.
pub fn pair_at(sandbox: &Sandbox, boundary: SourceBoundary) -> String {
    let mut sources = serde_json::Map::new();
    for (name, row) in [(NATIVE, &ROWS[0]), (SCANNED, &ROWS[1])] {
        sources.insert(
            name.to_owned(),
            boundary.source(row.plugin, (row.fixture.block)(sandbox)),
        );
    }
    document(&Value::Object(sources))
}

/// `<source>:<native>`, the form a user types.
///
/// Spelled here rather than inline so a journey asserting on an id is asserting on the
/// rendering under test rather than on its own `format!`.
pub fn qualified(source: &str, native: &str) -> String {
    format!("{source}:{native}")
}

/// Every row a journey runs against.
///
/// The two `in-memory` rows are the pair that proves pushdown and compensation return
/// one correct answer by two different plans: they hold the same dataset and the same
/// dependency graph, and differ only in what they declare. One answers reverse
/// dependencies itself and one does not, which is what makes the engine's emulated
/// reverse scan exercised deliberately here rather than incidentally by whichever plugin
/// happens to be poor at it.
pub const ROWS: &[Row] = &[
    Row {
        plugin: "in-memory",
        name: "in-memory (declares everything native)",
        fixture: Ready {
            block: native_block,
            complete_dataset: true,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: true,
                search_content: true,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        },
    },
    Row {
        plugin: "in-memory",
        name: "in-memory (declares nothing native, forward-only)",
        fixture: Ready {
            block: compensated_block,
            complete_dataset: true,
            declared: Declared {
                filter_by_label: false,
                filter_by_status: false,
                search_title: false,
                search_content: false,
                orphan_tasks: false,
                reverse_task_dependencies: false,
                reverse_project_dependencies: false,
            },
        },
    },
    Row {
        plugin: "subprocess",
        name: "subprocess (the in-memory source over a real pipe)",
        fixture: Ready {
            block: hosted_block,
            complete_dataset: true,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: true,
                search_content: true,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        },
    },
    Row {
        plugin: "local-md",
        name: "local-md",
        fixture: Ready {
            block: local_md_block,
            complete_dataset: true,
            // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The journeys use
            // these expectations to assert the real binary's reported plan, whose values
            // come from `LocalMdSource::capabilities`; changing that implementation without
            // this fixture makes those public-boundary assertions fail, so the journeys are
            // the drift gate rather than a second authoritative declaration.
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: true,
                search_content: true,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        },
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Ready {
            block: linear_block,
            // Linear models the whole table: two projects, an orphan, and dependencies in
            // both directions, so it drives the shared complete-dataset journeys.
            complete_dataset: true,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: false,
                search_content: false,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        },
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Ready {
            block: github_projects_block,
            // One GitHub source is exactly one ProjectV2 board and every item belongs to
            // it. It cannot faithfully represent the table's two projects and orphan.
            // Focused journeys drive this working row over the subset GitHub can model.
            complete_dataset: false,
            declared: Declared {
                filter_by_label: false,
                filter_by_status: false,
                search_title: false,
                search_content: false,
                orphan_tasks: false,
                reverse_task_dependencies: false,
                reverse_project_dependencies: false,
            },
        },
    },
];

fn github_projects_block(sandbox: &Sandbox) -> Value {
    github_projects_server(sandbox, None)
}

/// The same board, with `T-1` recording `recorded` under the reserved dependency key.
///
/// The journeys that drive a key holding something it must not need a board that holds
/// it, and the shared row cannot be that board — it is the one every other journey reads.
pub fn github_projects_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    github_projects_server(sandbox, Some(recorded))
}

fn github_projects_server(sandbox: &Sandbox, recorded: Option<Value>) -> Value {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    let written = Arc::new(Mutex::new(Vec::<Value>::new()));
    let project_write = Arc::new(Mutex::new(None::<Value>));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("GitHub fixture connection");
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("GraphQL query string");
            graphql_parser::parse_query::<String>(query).expect("valid GraphQL document");
            let variables = request["variables"]
                .as_object()
                .expect("GraphQL variables object");
            let variables = &Value::Object(variables.clone());
            let data = if query.contains("addProjectV2DraftIssue(input:$input)") {
                let input = &variables["input"];
                assert_eq!(input["projectId"], "P-1");
                assert!(
                    input["title"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert!(input["body"].is_string() || input["body"].is_null());
                let mut rows = written.lock().unwrap();
                let number = rows.len() + 1;
                let content_id = format!("DRAFT-{number}");
                let item_id = format!("ITEM-DRAFT-{number}");
                rows.push(json!({"id":item_id,"fieldValues":{"nodes":[
                    {"name":"Todo","field":{"id":"FIELD-status","name":"Status","options":[{"id":"OPT-todo","name":"Todo"},{"id":"OPT-doing","name":"Doing"},{"id":"OPT-shipped","name":"Shipped"}]}},
                    {"text":"{}","field":{"id":"FIELD-metadata","name":"onetaskgraph.metadata"}}
                ],"pageInfo":{"hasNextPage":false}},"content":{"__typename":"DraftIssue","id":content_id,
                    "title":input["title"],"body":input["body"],"createdAt":null,"updatedAt":null}}));
                json!({"addProjectV2DraftIssue":{"projectItem":{"id":item_id,"content":{"id":content_id}}}})
            } else if query.contains("updateProjectV2DraftIssue(input:$input)") {
                let input = &variables["input"];
                assert!(
                    input["draftIssueId"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert!(input["title"].is_string());
                assert!(input["body"].is_string() || input["body"].is_null());
                let mut rows = written.lock().unwrap();
                let row = rows
                    .iter_mut()
                    .find(|row| row["content"]["id"] == input["draftIssueId"])
                    .expect("updated fixture draft exists");
                row["content"]["title"] = input["title"].clone();
                row["content"]["body"] = input["body"].clone();
                json!({"updateProjectV2DraftIssue":{"draftIssue":{"id":input["draftIssueId"]}}})
            } else if query.contains("updateIssue(input:$input)") {
                assert!(
                    variables["input"]["title"]
                        .as_str()
                        .is_some_and(|title| !title.is_empty())
                );
                assert!(
                    variables["input"]["body"].is_string() || variables["input"]["body"].is_null()
                );
                if variables["input"]["id"] == "T-1" {
                    assert_eq!(variables["input"]["title"], "Alpha engine revised");
                    assert_eq!(variables["input"]["body"], "the engine core");
                }
                json!({"updateIssue":{"issue":{"id":variables["input"]["id"]}}})
            } else if query.contains("updateProjectV2ItemFieldValue(input:$input)") {
                let input = &variables["input"];
                for key in ["projectId", "itemId", "fieldId"] {
                    assert!(input[key].as_str().is_some_and(|value| !value.is_empty()));
                }
                assert!(
                    input["value"]
                        .as_object()
                        .is_some_and(|value| value.len() == 1)
                );
                assert!(
                    input["value"]["text"].is_string()
                        || input["value"]["singleSelectOptionId"].is_string()
                );
                let mut rows = written.lock().unwrap();
                let row = rows.iter_mut().find(|row| row["id"] == input["itemId"]);
                if let Some(row) = row {
                    if input["fieldId"] == "FIELD-metadata" {
                        row["fieldValues"]["nodes"][1]["text"] = input["value"]["text"].clone();
                    }
                    if input["fieldId"] == "FIELD-status" {
                        let option = input["value"]["singleSelectOptionId"].as_str().unwrap();
                        row["fieldValues"]["nodes"][0]["name"] = json!(match option {
                            "OPT-doing" => "Doing",
                            "OPT-shipped" => "Shipped",
                            _ => "Todo",
                        });
                    }
                } else {
                    assert!(input["itemId"].as_str().unwrap().starts_with("ITEM-T-"));
                    if input["itemId"] == "ITEM-T-1" && input["fieldId"] == "FIELD-metadata" {
                        let metadata: Value =
                            serde_json::from_str(input["value"]["text"].as_str().unwrap()).unwrap();
                        assert_eq!(metadata["onetaskgraph.origin"], "notes:T-1");
                    }
                    if input["itemId"] == "ITEM-T-1" && input["fieldId"] == "FIELD-status" {
                        assert_eq!(input["value"]["singleSelectOptionId"], "OPT-todo");
                    }
                }
                json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":input["itemId"]}}})
            } else if query.contains("updateProjectV2(input:$input)") {
                assert_eq!(variables["input"]["projectId"], "P-1");
                assert!(
                    variables["input"]["title"]
                        .as_str()
                        .is_some_and(|title| !title.is_empty())
                );
                assert!(
                    variables["input"]["shortDescription"].is_string()
                        || variables["input"]["shortDescription"].is_null()
                );
                assert!(variables["input"]["closed"].is_boolean());
                *project_write.lock().unwrap() = Some(variables["input"].clone());
                json!({"updateProjectV2":{"projectV2":{"id":"P-1"}}})
            } else if query.contains("addBlockedBy(input:$input)") {
                assert!(
                    variables["input"]["issueId"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert!(
                    variables["input"]["blockingIssueId"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert_eq!(variables["input"]["issueId"], "T-1");
                assert_eq!(variables["input"]["blockingIssueId"], "T-3");
                json!({"addBlockedBy":{"issue":{"id":variables["input"]["issueId"]},"blockingIssue":{"id":variables["input"]["blockingIssueId"]}}})
            } else if query.contains("removeBlockedBy(input:$input)") {
                assert!(
                    variables["input"]["issueId"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert!(
                    variables["input"]["blockingIssueId"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
                assert_eq!(variables["input"]["issueId"], "T-1");
                assert_eq!(variables["input"]["blockingIssueId"], "T-2");
                json!({"removeBlockedBy":{"issue":{"id":variables["input"]["issueId"]},"blockingIssue":{"id":variables["input"]["blockingIssueId"]}}})
            } else if query.contains("node(id:$id)") {
                let id = variables["id"].as_str().expect("dependency id");
                let first = variables["first"]
                    .as_u64()
                    .expect("dependency first must be an unsigned integer");
                assert!(
                    (1..=100).contains(&first),
                    "dependency first is out of range"
                );
                assert!(
                    variables["after"].is_null() || variables["after"].is_string(),
                    "dependency after must be null or a string"
                );
                if written
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|row| row["content"]["id"] == id)
                {
                    json!({"node":{"__typename":"DraftIssue"}})
                } else {
                    // T-2 sits on a second board, so aggregating this board's issue edges
                    // yields a real project-level edge rather than one this board makes with
                    // itself, which the source drops.
                    let blockers = match id {
                        "T-1" | "T-3" | "T-4" => vec![
                            json!({"id":"T-2","projectItems":{"nodes":[{"project":{"id":"P-2"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
                        ],
                        _ => vec![],
                    };
                    // The production document selects both connections, so the fixture answers
                    // both: T-2 is what the other three are blocked by, and so what it blocks.
                    let blocking = if id == "T-2" {
                        ["T-1", "T-3", "T-4"]
                        .into_iter()
                        .map(|id| json!({"id":id,"projectItems":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}))
                        .collect::<Vec<_>>()
                    } else {
                        vec![]
                    };
                    json!({"node":{"__typename":"Issue",
                    "blockedBy":{"nodes":blockers,"pageInfo":{"hasNextPage":false,"endCursor":null}},
                    "blocking":{"nodes":blocking,"pageInfo":{"hasNextPage":false,"endCursor":null}}}})
                }
            } else if query.contains("owner:repositoryOwner") {
                assert_eq!(variables["owner"], "fixture-owner");
                assert_eq!(variables["number"], 7);
                assert_eq!(variables["nestedFirst"], 50);
                github_project_page(
                    variables,
                    recorded.as_ref(),
                    &written.lock().unwrap(),
                    project_write.lock().unwrap().as_ref(),
                )
            } else {
                panic!("fixture received an unknown GraphQL operation")
            };
            let body = json!({"data":data}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("GitHub fixture response");
        }
    });
    json!({
        "owner": "fixture-owner",
        "project_number": 7,
        "token_env": "GITHUB_PROJECTS_FIXTURE_TOKEN",
        "endpoint": endpoint,
        "status_mapping": {"Doing":"in-progress", "Shipped":"done"}
    })
}

fn read_http_json(stream: &mut impl Read) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("fixture request");
        assert!(count > 0, "fixture request ended before its HTTP headers");
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP header terminator")
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
        .expect("Content-Length");
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("fixture request body");
        assert!(count > 0, "fixture request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).expect("request JSON")
}

fn github_project_page(
    variables: &Value,
    recorded: Option<&Value>,
    written: &[Value],
    project_write: Option<&Value>,
) -> Value {
    let tasks = [
        (
            "T-1",
            "Alpha engine",
            "the engine core",
            "Todo",
            "OPEN",
            vec![("L-1", "bug"), ("L-3", "core")],
        ),
        (
            "T-2",
            "Beta",
            "alpha in the body",
            "Shipped",
            "CLOSED",
            vec![("L-2", "chore")],
        ),
        (
            "T-3",
            "Gamma",
            "unrelated",
            "Todo",
            "OPEN",
            vec![("L-1", "bug")],
        ),
        (
            "T-4",
            "Delta docs",
            "documentation",
            "Doing",
            "OPEN",
            vec![("L-3", "core")],
        ),
    ];
    let offset = match variables.get("after") {
        Some(Value::Null) => 0,
        Some(Value::String(value)) => value.parse::<usize>().expect("numeric after cursor"),
        _ => panic!("GraphQL after must be null or a numeric string"),
    };
    let first = usize::try_from(
        variables
            .get("first")
            .and_then(Value::as_u64)
            .expect("GraphQL first must be an unsigned integer"),
    )
    .expect("GraphQL first fits usize");
    assert!(first > 0, "GraphQL first must be positive");
    assert!(
        offset <= tasks.len(),
        "GraphQL after cursor is out of range"
    );
    let end = (offset + first).min(tasks.len());
    let mut nodes = tasks[offset..end]
        .iter()
        .map(|(id, title, body, status, state, labels)| json!({
            "id": format!("ITEM-{id}"),
            "fieldValues":{"nodes":[{"name":status,"field":{"id":"FIELD-status","name":"Status","options":[
                {"id":"OPT-todo","name":"Todo"},{"id":"OPT-doing","name":"Doing"},{"id":"OPT-shipped","name":"Shipped"}]}},
                {"text": if *id == "T-1" {serde_json::to_string(&json!({"onepipeline.turn_budget":12,"caller.flags":[true,null],
                    "onetaskgraph.depends_on": recorded.cloned().unwrap_or_else(|| Value::Array(recorded_far_ends("task_dependencies",&json!("T-1"))))})).unwrap()} else {"{}".into()},"field":{"id":"FIELD-metadata","name":"onetaskgraph.metadata"}}],"pageInfo":{"hasNextPage":false}},
            "content":{
                "__typename":"Issue","id":id,"title":title,"body":body,"state":state,"repository":{"nameWithOwner":"nickderobertis/onetaskgraph"},
                "url":format!("https://example.invalid/{id}"),
                "labels":{"nodes":labels.iter().map(|(id, name)| json!({"id":id,"name":name})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false}}
            }
        }))
        .collect::<Vec<_>>();
    if end == tasks.len() {
        nodes.extend_from_slice(written);
    }
    let title = project_write
        .and_then(|value| value["title"].as_str())
        .unwrap_or("Engine");
    let description = project_write.and_then(|value| value["shortDescription"].as_str()).map(str::to_owned).unwrap_or_else(|| format!("alpha engine project\n\n<!-- onetaskgraph.metadata\n{}\n-->", serde_json::to_string(&json!({
        "onepipeline.publication":{"mode":"review"},
        "onetaskgraph.repositories":["github.com/nickderobertis/onetaskgraph"],
        "onetaskgraph.depends_on":recorded_far_ends("project_dependencies",&json!("P-1"))
    })).unwrap()));
    let closed = project_write
        .and_then(|value| value["closed"].as_bool())
        .unwrap_or(false);
    json!({
        "owner":{"projectV2":{
            "id":"P-1","title":title,"shortDescription":description,
            "url":"https://example.invalid/P-1","closed":closed,
            "fields":{"nodes":[{"__typename":"ProjectV2SingleSelectField","id":"FIELD-status","name":"Status","options":[{"id":"OPT-todo","name":"Todo"},{"id":"OPT-doing","name":"Doing"},{"id":"OPT-shipped","name":"Shipped"}]},{"__typename":"ProjectV2Field","id":"FIELD-metadata","name":"onetaskgraph.metadata"}],"pageInfo":{"hasNextPage":false}},
            "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < tasks.len(),"endCursor":end.to_string()}}
        }},
        "user":{"projectV2":null}
    })
}

/// A socket-level Linear GraphQL fixture used by the shared binary journeys.
pub fn linear_block(sandbox: &Sandbox) -> Value {
    linear_server(sandbox, None)
}

/// The same workspace, with the item a dependency read asks about recording `recorded`
/// under the reserved dependency key.
///
/// The counterpart of [`github_projects_recording`], and it exists for the same reason:
/// the shared row is the one every other journey reads, so a workspace holding a key it
/// must not cannot be that row.
pub fn linear_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    linear_server(sandbox, Some(recorded))
}

fn linear_server(sandbox: &Sandbox, recorded: Option<Value>) -> Value {
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(dataset()));
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > 8_192 {
                    break;
                }
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.parse::<usize>().ok());
                    if length.is_some_and(|length| bytes.len() >= split + 4 + length) {
                        break;
                    }
                }
            }
            if bytes.len() > 8_192 {
                let text = r#"{"errors":[{"message":"fixture request too large"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 413 Content Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let split = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|n| n + 4)
                .unwrap_or(bytes.len());
            let request_head = String::from_utf8_lossy(&bytes[..split]);
            let declared_length = request_head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse::<usize>().ok());
            if declared_length.is_none_or(|length| bytes.len() != split + length) {
                let text = r#"{"errors":[{"message":"invalid content length"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let request_line = request_head
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r');
            if request_line != "POST /graphql HTTP/1.1" {
                let text = r#"{"errors":[{"message":"expected POST /graphql"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let Ok(request) = serde_json::from_slice::<Value>(&bytes[split..]) else {
                let text = r#"{"errors":[{"message":"invalid fixture request"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            };
            if request.get("query").and_then(Value::as_str).is_none() {
                let text = r#"{"errors":[{"message":"missing GraphQL query"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let (status, response) =
                match linear_response(&request, recorded.as_ref(), &mut state.lock().unwrap()) {
                    Ok(body) => ("200 OK", json!({"data":body})),
                    Err(message) => ("400 Bad Request", json!({"errors":[{"message":message}]})),
                };
            let text = serde_json::to_string(&response).unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    json!({"endpoint":endpoint,"team":"FIX"})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinearRequest {
    query: String,
    variables: serde_json::Map<String, Value>,
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] These typed fixture-boundary variables mirror the accepted 2026-08-24 Linear documents; the authoritative variable/nullability contract is available only from Linear's authenticated unversioned explorer, while focused TCP tests prove malformed local requests are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoVariables {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemVariables {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
    filter: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationVariables {
    id: String,
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

fn validate_linear_variables(operation: &str, variables: &Value) -> Result<(), &'static str> {
    use onetaskgraph_linear::graphql;
    let valid = match operation {
        graphql::VIEWER => serde_json::from_value::<NoVariables>(variables.clone()).is_ok(),
        graphql::ISSUE | graphql::PROJECT => {
            serde_json::from_value::<ItemVariables>(variables.clone())
                .is_ok_and(|variables| !variables.id.is_empty())
        }
        graphql::LABELS => serde_json::from_value::<PageVariables>(variables.clone())
            .is_ok_and(|variables| variables.first > 0 && variables.after.as_deref() != Some("")),
        graphql::ISSUES | graphql::PROJECTS => {
            serde_json::from_value::<QueryVariables>(variables.clone()).is_ok_and(|variables| {
                variables.first > 0
                    && variables.after.as_deref() != Some("")
                    && valid_linear_filter(&Value::Object(variables.filter))
            })
        }
        graphql::ISSUE_RELATIONS | graphql::PROJECT_RELATIONS => {
            serde_json::from_value::<RelationVariables>(variables.clone()).is_ok_and(|variables| {
                !variables.id.is_empty()
                    && variables.first > 0
                    && variables.after.as_deref() != Some("")
            })
        }
        graphql::TEAM => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("key").is_some_and(|value| !value.is_empty())
                })
        }
        graphql::ISSUE_STATE => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 2
                        && ["name", "team"]
                            .iter()
                            .all(|key| values.get(*key).is_some_and(|value| !value.is_empty()))
                })
        }
        graphql::PROJECT_STATUS | graphql::ISSUE_LABEL | graphql::PROJECT_LABEL => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("name").is_some_and(|value| !value.is_empty())
                })
        }
        graphql::ISSUE_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamId", "title", "stateId", "labelIds"],
                    &["description", "projectId"],
                )
        }
        graphql::PROJECT_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamIds", "name", "statusId", "labelIds"],
                    &["description"],
                )
        }
        graphql::ISSUE_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["issueId", "relatedIssueId", "type"],
                    &[],
                )
        }
        graphql::PROJECT_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["projectId", "relatedProjectId", "type"],
                    &[],
                )
        }
        graphql::ISSUE_RELATION_DELETE | graphql::PROJECT_RELATION_DELETE => {
            exact_linear_variable_keys(variables, &["id"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
        }
        graphql::ISSUE_UPDATE | graphql::PROJECT_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(
                    variables.get("input"),
                    if operation == graphql::ISSUE_UPDATE {
                        &["title", "stateId", "labelIds"]
                    } else {
                        &["name", "statusId", "labelIds"]
                    },
                    if operation == graphql::ISSUE_UPDATE {
                        &["description", "projectId"]
                    } else {
                        &["description"]
                    },
                )
        }
        _ => false,
    };
    valid.then_some(()).ok_or("invalid operation variables")
}

fn exact_linear_variable_keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == expected.len() && expected.iter().all(|key| fields.contains_key(*key))
    })
}

fn valid_linear_write_input(value: Option<&Value>, required: &[&str], optional: &[&str]) -> bool {
    let Some(fields) = value.and_then(Value::as_object) else {
        return false;
    };
    if fields
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        || required.iter().any(|key| !fields.contains_key(*key))
    {
        return false;
    }
    fields.iter().all(|(key, value)| match key.as_str() {
        "labelIds" | "teamIds" => value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| value.as_str().is_some_and(|id| !id.is_empty()))
        }),
        "description" => value.is_null() || value.is_string(),
        "projectId" => value.is_null() || value.as_str().is_some_and(|id| !id.is_empty()),
        _ => value.as_str().is_some_and(|text| !text.is_empty()),
    })
}

fn valid_linear_filter(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().all(|(key, value)| match key.as_str() {
            "and" => value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|value| value.is_object() && valid_linear_filter(value))
            }),
            "team" | "key" | "labels" | "some" | "name" | "every" | "state" | "type"
            | "project" | "id" => value.is_object() && valid_linear_filter(value),
            "eqIgnoreCase" | "neqIgnoreCase" | "eq" => value.is_string(),
            "inIgnoreCase" | "in" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "null" => value.is_boolean(),
            _ => false,
        }),
        _ => false,
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

fn linear_response(
    request: &Value,
    recorded: Option<&Value>,
    data: &mut Value,
) -> Result<Value, &'static str> {
    let request: LinearRequest =
        serde_json::from_value(request.clone()).map_err(|_| "invalid GraphQL request")?;
    use onetaskgraph_linear::graphql;
    let operation = request.query.as_str();
    if ![
        graphql::VIEWER,
        graphql::ISSUE,
        graphql::PROJECT,
        graphql::ISSUES,
        graphql::PROJECTS,
        graphql::LABELS,
        graphql::ISSUE_RELATIONS,
        graphql::PROJECT_RELATIONS,
        graphql::TEAM,
        graphql::ISSUE_STATE,
        graphql::PROJECT_STATUS,
        graphql::ISSUE_LABEL,
        graphql::PROJECT_LABEL,
        graphql::ISSUE_CREATE,
        graphql::ISSUE_UPDATE,
        graphql::PROJECT_CREATE,
        graphql::PROJECT_UPDATE,
        graphql::ISSUE_RELATION_CREATE,
        graphql::PROJECT_RELATION_CREATE,
        graphql::ISSUE_RELATION_DELETE,
        graphql::PROJECT_RELATION_DELETE,
    ]
    .contains(&operation)
    {
        return Err("unknown GraphQL operation");
    }
    let vars = Value::Object(request.variables);
    validate_linear_variables(operation, &vars)?;
    if operation == graphql::TEAM {
        return Ok(json!({"teams":{"nodes":[{"id":"TEAM-1"}]}}));
    }
    if operation == graphql::ISSUE_STATE {
        return Ok(json!({"workflowStates":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::PROJECT_STATUS {
        return Ok(json!({"projectStatuses":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::ISSUE_LABEL {
        return Ok(json!({"issueLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::PROJECT_LABEL {
        return Ok(json!({"projectLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    if matches!(operation, graphql::ISSUE_CREATE | graphql::ISSUE_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::ISSUE_CREATE, false);
    }
    if matches!(operation, graphql::PROJECT_CREATE | graphql::PROJECT_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::PROJECT_CREATE, true);
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_CREATE | graphql::PROJECT_RELATION_CREATE
    ) {
        return linear_write_relation(data, &vars, operation == graphql::PROJECT_RELATION_CREATE);
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_DELETE | graphql::PROJECT_RELATION_DELETE
    ) {
        let project = operation == graphql::PROJECT_RELATION_DELETE;
        let index = vars["id"]
            .as_str()
            .and_then(|id| id.rsplit(':').next())
            .and_then(|id| id.parse::<usize>().ok())
            .ok_or("invalid relation fixture id")?;
        let edges = data[if project {
            "project_dependencies"
        } else {
            "task_dependencies"
        }]
        .as_array_mut()
        .ok_or("fixture edges are not an array")?;
        if index < edges.len() {
            edges.remove(index);
        }
        return Ok(if project {
            json!({"projectRelationDelete":{"success":true}})
        } else {
            json!({"issueRelationDelete":{"success":true}})
        });
    }
    if operation == graphql::LABELS {
        return Ok(
            json!({"issueLabels":linear_connection(data["labels"].as_array().unwrap().iter().map(linear_label).collect(),&vars)}),
        );
    }
    if operation == graphql::ISSUES {
        let mut rows: Vec<Value> = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_matches_fixture_subset(v, &vars))
            .map(|v| linear_task(v, data))
            .collect();
        return Ok(json!({"issues":linear_connection(std::mem::take(&mut rows),&vars)}));
    }
    if operation == graphql::PROJECTS {
        let rows = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_matches_fixture_subset(v, &vars))
            .map(|v| linear_project(v, data))
            .collect();
        return Ok(json!({"projects":linear_connection(rows,&vars)}));
    }
    if matches!(operation, graphql::ISSUE | graphql::ISSUE_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::ISSUE_RELATIONS {
            return Ok(
                json!({"issue":linear_relations(data,"task_dependencies",id,"Issue",recorded)}),
            );
        }
        return Ok(json!({"issue":item.map(|v|linear_task(v,data))}));
    }
    if matches!(operation, graphql::PROJECT | graphql::PROJECT_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::PROJECT_RELATIONS {
            return Ok(
                json!({"project":linear_relations(data,"project_dependencies",id,"Project",recorded)}),
            );
        }
        return Ok(json!({"project":item.map(|v|linear_project(v,data))}));
    }
    Ok(json!({"viewer":{"id":"fixture-user"}}))
}

fn linear_write_item(
    data: &mut Value,
    vars: &Value,
    create: bool,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("write input must be an object")?;
    let collection = if project { "projects" } else { "tasks" };
    let rows = data[collection]
        .as_array_mut()
        .ok_or("fixture collection is not an array")?;
    let id = if create {
        format!("{}-W{}", if project { "P" } else { "T" }, rows.len() + 1)
    } else {
        vars["id"]
            .as_str()
            .ok_or("update id must be a string")?
            .to_owned()
    };
    let existing = rows.iter().position(|row| row["id"] == id);
    if !create && existing.is_none() {
        return Err("update target does not exist");
    }
    let title_key = if project { "name" } else { "title" };
    let status_key = if project { "statusId" } else { "stateId" };
    let labels = input
        .get("labelIds")
        .and_then(Value::as_array)
        .ok_or("labelIds must be an array")?
        .iter()
        .map(|id| json!({"id":id,"name":id}))
        .collect::<Vec<_>>();
    let mut row = json!({
        "id": id,
        "title": input.get(title_key).and_then(Value::as_str).ok_or("title must be a string")?,
        "content": "",
        "status": {"name":input.get(status_key).and_then(Value::as_str).ok_or("status id must be a string")?,"category":"todo"},
        "labels": labels,
        "_linear_description": input.get("description").cloned().unwrap_or(Value::Null),
    });
    if !project && let Some(project_id) = input.get("projectId").filter(|v| !v.is_null()) {
        row["project"] = project_id.clone();
    }
    if let Some(index) = existing {
        rows[index] = row;
    } else {
        rows.push(row);
    }
    let payload = json!({"id":id});
    Ok(if project {
        if create {
            json!({"projectCreate":{"success":true,"project":payload}})
        } else {
            json!({"projectUpdate":{"success":true,"project":payload}})
        }
    } else if create {
        json!({"issueCreate":{"success":true,"issue":payload}})
    } else {
        json!({"issueUpdate":{"success":true,"issue":payload}})
    })
}

fn linear_write_relation(
    data: &mut Value,
    vars: &Value,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("relation input must be an object")?;
    let near_key = if project { "projectId" } else { "issueId" };
    let far_key = if project {
        "relatedProjectId"
    } else {
        "relatedIssueId"
    };
    let kind = input
        .get("type")
        .and_then(Value::as_str)
        .ok_or("relation type must be a string")?;
    if !matches!(kind, "blocks" | "related") {
        return Err("undocumented relation type");
    }
    let edge = json!({"from":input.get(near_key).ok_or("missing near id")?,"to":input.get(far_key).ok_or("missing far id")?,"kind":kind});
    data[if project {
        "project_dependencies"
    } else {
        "task_dependencies"
    }]
    .as_array_mut()
    .ok_or("fixture edges are not an array")?
    .push(edge);
    Ok(if project {
        json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"PR-W"}}})
    } else {
        json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"IR-W"}}})
    })
}

#[test]
// llmlint: ignore[tests_mirror_real_usage] This is a failure test for the fixture server's own untrusted HTTP boundary, which product CLI requests cannot malformedly exercise because the Linear client always emits valid typed requests; it intentionally sends raw TCP requests through the real socket rather than calling response logic directly.
fn linear_fixture_rejects_invalid_variables_and_unknown_operations() {
    let sandbox = Sandbox::new();
    let config = linear_block(&sandbox);
    let endpoint = config["endpoint"].as_str().unwrap();
    let address = endpoint
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/graphql")
        .unwrap();
    for body in [
        json!({"query":onetaskgraph_linear::graphql::VIEWER}),
        json!({"query":onetaskgraph_linear::graphql::VIEWER,"variables":[]}),
        json!({"query":"query { invented { id } }","variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":7}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":"i1","extra":true}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":0,"after":null,"filter":{}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":[]}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"invented":true}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"state":{"type":{"in":7}}}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"and":"invalid"}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"project":{"null":"true"}}}}),
    ] {
        let body = serde_json::to_string(&body).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        write!(
            stream,
            "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
    }
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(stream, "GET /graphql HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the invalid-method fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: 9\r\n\r\n{{}}"
    )
    .unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the short-body fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let oversized = "x".repeat(8_193);
    let _ = write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n{oversized}",
        oversized.len()
    );
    // The fixture can reject and close this oversized request before the client reaches
    // shutdown, so a half-close here would race with the expected server response.
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 413 Content Too Large"));
}
/// The far ends `id` records under the reserved key, for a source with no native way to
/// name one: every qualified endpoint the dataset gives that item at `key`.
fn recorded_far_ends(key: &str, id: &Value) -> Vec<Value> {
    dataset()[key]
        .as_array()
        .expect("the dataset lists edges")
        .iter()
        .filter(|edge| edge["from"].get("id") == Some(id))
        .map(|edge| edge["to"].clone())
        .collect()
}
fn linear_label(v: &Value) -> Value {
    json!({"id":v["id"],"name":v["name"],"color":null})
}
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This fixture-only mapping and matcher implement the finite shared journey dataset against the accepted 2026-08-24 contract; production parsing and real CLI row assertions independently verify the observable behavior without requiring live credentials.
fn linear_state(v: &Value) -> Value {
    let category = v["category"].as_str().unwrap_or("");
    json!({"name":v["name"],"type":match category{"todo"=>"unstarted","in-progress"=>"started","done"=>"completed","cancelled"=>"canceled",_=>"backlog"}})
}
fn linear_task(v: &Value, data: &Value) -> Value {
    json!({"id":v["id"],"title":v["title"],"description":linear_description(v,"task_dependencies",data),"state":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"project":v.get("project").map(|id|json!({"id":id})),"url":v.get("url"),"createdAt":null,"updatedAt":null})
}
fn linear_project(v: &Value, data: &Value) -> Value {
    json!({"id":v["id"],"name":v["title"],"description":linear_description(v,"project_dependencies",data),"status":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"url":v.get("url"),"createdAt":null,"updatedAt":null})
}
fn linear_description(v: &Value, edges: &str, data: &Value) -> String {
    if let Some(description) = v.get("_linear_description").and_then(Value::as_str) {
        return description.to_owned();
    }
    let mut metadata = v
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(repositories) = v.get("repositories") {
        metadata.insert("onetaskgraph.repositories".into(), repositories.clone());
    }
    // No Linear relation can name an item of another source, so this is the one slot a
    // far end like that can be in.
    let far = data[edges]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["from"].get("id") == Some(&v["id"])
                && edge["to"]["id"].as_str().is_some_and(|id| id.contains(':'))
        })
        .map(|edge| edge["to"].clone())
        .collect::<Vec<_>>();
    if !far.is_empty() {
        metadata.insert("onetaskgraph.depends_on".into(), Value::Array(far));
    }
    let content = v.get("content").and_then(Value::as_str).unwrap_or_default();
    if metadata.is_empty() {
        content.into()
    } else {
        linear_metadata_slot(content, &Value::Object(metadata))
    }
}

/// The one slot a Linear item keeps caller-defined metadata in: an HTML comment appended
/// to the description, which is what the source reads and what a person never sees.
fn linear_metadata_slot(content: &str, metadata: &Value) -> String {
    format!(
        "{content}\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        serde_json::to_string(metadata).unwrap()
    )
}
fn linear_connection(rows: Vec<Value>, vars: &Value) -> Value {
    let start = vars["after"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit = vars["first"].as_u64().unwrap_or(50) as usize;
    let nodes = rows
        .iter()
        .skip(start)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let end = start + nodes.len();
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":end<rows.len(),"endCursor":if end<rows.len(){Some(end.to_string())}else{None}}})
}
fn linear_matches_fixture_subset(v: &Value, vars: &Value) -> bool {
    let text = vars["filter"].to_string().to_ascii_lowercase();
    let labels = v["labels"].as_array().unwrap();
    for name in ["bug", "chore", "core"] {
        if text.contains(&format!("\"{name}\"")) {
            let present = labels.iter().any(|l| l["name"].as_str() == Some(name));
            let excluded = text.contains(&format!("neqignorecase\":\"{name}"));
            if (excluded && present) || (!excluded && !present) {
                return false;
            }
        }
    }
    let mut allowed = Vec::new();
    for (linear, category) in [
        ("completed", "done"),
        ("unstarted", "todo"),
        ("\"started\"", "in-progress"),
        ("backlog", "backlog"),
        ("canceled", "cancelled"),
    ] {
        if text.contains(linear) {
            allowed.push(category);
        }
    }
    if !allowed.is_empty() && !allowed.contains(&v["status"]["category"].as_str().unwrap_or("")) {
        return false;
    }
    if text.contains("\"null\":true") && v.get("project").is_some() {
        return false;
    }
    for id in ["p-1", "p-2"] {
        if text.contains(id)
            && v.get("project")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(id)
        {
            return false;
        }
    }
    true
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
fn linear_relations(
    data: &Value,
    key: &str,
    id: &str,
    suffix: &str,
    recorded: Option<&Value>,
) -> Value {
    let edges = data[key].as_array().unwrap();
    // A Linear relation names a Linear item, so only the edges whose ends are both plain
    // native ids are here. The rest are in the item's own description slot, which this
    // operation selects for exactly that reason.
    let forward = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["from"] == id && e["to"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":e["kind"],(format!("related{suffix}")):{"id":e["to"]}}))
        .collect::<Vec<_>>();
    let inverse = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["to"] == id && e["from"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":e["kind"],(suffix.to_ascii_lowercase()):{"id":e["from"]}}))
        .collect::<Vec<_>>();
    let items = if suffix == "Issue" {
        "tasks"
    } else {
        "projects"
    };
    let item = data[items]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id);
    // The description slot is where this source reads a recorded far end from, so a
    // workspace built to hold one puts it here and leaves every other operation alone.
    let description = match recorded {
        Some(recorded) => Some(linear_metadata_slot(
            "",
            &json!({"onetaskgraph.depends_on": recorded}),
        )),
        None => item.map(|item| linear_description(item, key, data)),
    };
    json!({"description":description,"relations":{"nodes":forward,"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":inverse,"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

fn local_md_block(sandbox: &Sandbox) -> Value {
    let root = sandbox.subdirectory("local-md");
    for (kind, id, front, body) in [
        (
            "tasks",
            "T-1",
            "title: Alpha engine\nstatus: Todo\nlabels: [{id: L-1, name: bug}, {id: L-3, name: core}]\nproject: P-1\nurl: https://example.invalid/T-1\nmetadata: {onepipeline.turn_budget: 12, caller.flags: [true, null]}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [T-2, {id: \"elsewhere:P-9\", item: project}]",
            "the engine core",
        ),
        (
            "tasks",
            "T-2",
            "title: Beta\nstatus: Shipped\nlabels: [{id: L-2, name: chore}]\nproject: P-1",
            "alpha in the body",
        ),
        (
            "tasks",
            "T-3",
            "title: Gamma\nstatus: Todo\nlabels: [{id: L-1, name: bug}]\ndepends_on: [T-2]",
            "unrelated",
        ),
        (
            "tasks",
            "T-4",
            "title: Delta docs\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nproject: P-2\ndepends_on:\n  - id: T-2\n    kind: related",
            "documentation",
        ),
        (
            "projects",
            "P-1",
            "title: Engine\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nurl: https://example.invalid/P-1\nmetadata: {onepipeline.publication: {mode: review}}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [P-2, {id: \"elsewhere:T-9\", item: task}]",
            "the engine",
        ),
        ("projects", "P-2", "title: Docs\nstatus: Todo", "alpha docs"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
        std::fs::write(path, format!("---\n{front}\n---\n{body}\n")).expect("Markdown fixture");
    }
    json!({ "root": root, "status_mapping": {"todo":"todo", "doing":"in-progress", "shipped":"done"} })
}

/// The `in-memory` row that applies every predicate itself.
fn native_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = json!({"max_page_size": 50});
    block
}

/// The row that runs the same dataset in a second process, over the stdio protocol.
///
/// This is journey 19 — every journey again, through a subprocess-wrapped source — and it
/// is a row rather than a suite of its own for the reason the whole table exists: a
/// transport proven by tests written for it is proven against its author's expectations,
/// and this one has to answer the same assertions every in-process source answers. It
/// declares everything native because the source behind the pipe does, which is the claim
/// worth making: what a source can do must not change because it is a process away.
fn hosted_block(_sandbox: &Sandbox) -> Value {
    let mut settings = dataset();
    settings["capabilities"] = json!({"max_page_size": 50});
    json!({
        "command": env!("CARGO_BIN_EXE_onetaskgraph"),
        "args": ["plugin-serve", "in-memory"],
        "settings": settings,
    })
}

/// The `in-memory` row that applies none of them, and pages two rows at a time.
///
/// A small page ceiling on purpose: compensation has to walk more than one page to find
/// the rows a filter keeps, and a ceiling of two is what makes a journey notice when it
/// stops doing so.
fn compensated_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = json!({
        "filter_by_label": "unsupported",
        "filter_by_status": "unsupported",
        "search_title": "unsupported",
        "search_content": "unsupported",
        "orphan_tasks": "unsupported",
        "task_dependencies": "forward-only",
        "project_dependencies": "forward-only",
        "max_page_size": 2
    });
    block
}

/// The work every row serves: four tasks, three of which are in a project, three labels,
/// two projects, and a dependency graph with a reverse answer worth checking.
///
/// Exactly one of the two projects carries a label, and the two sit in different status
/// categories, so every project filter has something to keep and something to drop —
/// a filter both projects satisfied would pass against a source that ignored it.
///
/// The dependency edges are listed in the order their `from` items are, which is what
/// makes the engine's emulated reverse scan — item by item, each item's forward edges in
/// order — produce the *same sequence* a source answering natively does, rather than the
/// same set in another order. A fixture that shuffled them would make the two answers
/// compare unequal for a reason that has nothing to do with the engine.
///
/// Two of the edges leave this source altogether — one from a task to a project, one from
/// a project to a task, both in a source called `elsewhere` that is not configured at all.
/// They are here rather than in a journey of their own because *where* such an edge is
/// held is each source's own business: a native relation that can name the far end, and
/// the reserved key on the near item where none can. Every row below encodes these two in
/// its own way, and one journey asserts that all of them report the same edge.
pub fn dataset() -> Value {
    json!({
        "tasks": [
            {"id": "T-1", "title": "Alpha engine", "content": "the engine core",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}, {"id": "L-3", "name": "core"}],
            "project": "P-1", "url": "https://example.invalid/T-1",
            "metadata": {"onepipeline.turn_budget": 12, "caller.flags": [true, null]},
            "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "T-2", "title": "Beta", "content": "alpha in the body",
             "status": {"category": "done", "name": "Shipped"},
             "labels": [{"id": "L-2", "name": "chore"}], "project": "P-1"},
            {"id": "T-3", "title": "Gamma", "content": "unrelated",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}]},
            {"id": "T-4", "title": "Delta docs", "content": "documentation",
             "status": {"category": "in-progress", "name": "Doing"},
             "labels": [{"id": "L-3", "name": "core"}], "project": "P-2"}
        ],
        "projects": [
            {"id": "P-1", "title": "Engine", "content": "the engine",
             "status": {"category": "in-progress", "name": "Doing"},
             "labels": [{"id": "L-3", "name": "core"}],
             "url": "https://example.invalid/P-1",
             "metadata": {"onepipeline.publication": {"mode": "review"}},
             "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "P-2", "title": "Docs", "content": "alpha docs",
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ],
        "labels": [
            {"id": "L-1", "name": "bug"},
            {"id": "L-2", "name": "chore"},
            {"id": "L-3", "name": "core"}
        ],
        "task_dependencies": [
            {"from": "T-1", "to": "T-2", "kind": "blocks"},
            {"from": {"id": "T-1", "kind": "task"},
             "to": {"id": "elsewhere:P-9", "kind": "project"}, "kind": "blocks"},
            {"from": "T-3", "to": "T-2", "kind": "blocks"},
            {"from": "T-4", "to": "T-2", "kind": "related"}
        ],
        "project_dependencies": [
            {"from": "P-1", "to": "P-2", "kind": "blocks"},
            {"from": {"id": "P-1", "kind": "project"},
             "to": {"id": "elsewhere:T-9", "kind": "task"}, "kind": "blocks"}
        ]
    })
}
