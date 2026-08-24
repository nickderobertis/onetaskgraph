//! The one table every journey is written against.
//!
//! A journey is written once and run against **every** source kind, so no plugin is ever
//! proven by a suite of its own writing. A row says which registry plugin it stands for,
//! how to configure one over the shared dataset below, and what that configuration
//! *declares* — which is what lets one journey assert both the rows and the plan against
//! a source that filters natively and one that does not.
//!
//! `scripts/check-journey-matrix.sh` fails, naming the plugin, when a plugin the registry
//! knows has no row here. A plugin whose source has not landed carries a
//! [`Fixture::Pending`] row rather than no row: that row is a journey too — it asserts
//! the plugin refuses with its own message — so a placeholder cannot sit here doing
//! nothing.

use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use crate::common::{Sandbox, SourceBoundary};

/// One row: a source kind, in one configuration, over the shared dataset.
pub struct Row {
    /// The registry plugin kind this row stands for.
    pub plugin: &'static str,
    /// This row's own name, unique across rows, used in failure messages.
    pub name: &'static str,
    /// How to build it, or why it cannot be built yet.
    pub fixture: Fixture,
}

/// A row that can be configured, or one whose plugin has not landed.
pub enum Fixture {
    /// A working source over the shared dataset.
    Ready(Ready),
    /// The plugin is registered and refuses to build. The journey for such a row asserts
    /// exactly that, so this is a test rather than a placeholder.
    Pending,
}

/// Everything a journey needs in order to drive one configured source.
pub struct Ready {
    /// The `config:` block, given a sandbox to write into if the source needs files.
    pub block: fn(&Sandbox) -> Value,
    /// What this configuration declares it applies itself.
    pub declared: Declared,
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
        let block = match &self.fixture {
            Fixture::Ready(ready) => (ready.block)(sandbox),
            Fixture::Pending => json!({}),
        };
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block}
        }))
    }

    /// What this row declares, or nothing when its plugin has not landed.
    pub fn declared(&self) -> Option<&Declared> {
        match &self.fixture {
            Fixture::Ready(ready) => Some(&ready.declared),
            Fixture::Pending => None,
        }
    }
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
        let Fixture::Ready(ready) = &row.fixture else {
            panic!("the first two rows are the configured `in-memory` pair");
        };
        sources.insert(
            name.to_owned(),
            boundary.source(row.plugin, (ready.block)(sandbox)),
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
        fixture: Fixture::Ready(Ready {
            block: native_block,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: true,
                search_content: true,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        }),
    },
    Row {
        plugin: "in-memory",
        name: "in-memory (declares nothing native, forward-only)",
        fixture: Fixture::Ready(Ready {
            block: compensated_block,
            declared: Declared {
                filter_by_label: false,
                filter_by_status: false,
                search_title: false,
                search_content: false,
                orphan_tasks: false,
                reverse_task_dependencies: false,
                reverse_project_dependencies: false,
            },
        }),
    },
    Row {
        plugin: "subprocess",
        name: "subprocess (the in-memory source over a real pipe)",
        fixture: Fixture::Ready(Ready {
            block: hosted_block,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: true,
                search_content: true,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        }),
    },
    Row {
        plugin: "local-md",
        name: "local-md",
        fixture: Fixture::Ready(Ready {
            block: local_md_block,
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
        }),
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Fixture::Ready(Ready {
            block: linear_block,
            declared: Declared {
                filter_by_label: true,
                filter_by_status: true,
                search_title: false,
                search_content: false,
                orphan_tasks: true,
                reverse_task_dependencies: true,
                reverse_project_dependencies: true,
            },
        }),
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Fixture::Pending,
    },
];

/// A socket-level Linear GraphQL fixture used by the shared binary journeys.
fn linear_block(sandbox: &Sandbox) -> Value {
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
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
            let (status, response) = match linear_response(&request) {
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
    json!({"endpoint":endpoint})
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
        _ => false,
    };
    valid.then_some(()).ok_or("invalid operation variables")
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

fn linear_response(request: &Value) -> Result<Value, &'static str> {
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
    ]
    .contains(&operation)
    {
        return Err("unknown GraphQL operation");
    }
    let vars = Value::Object(request.variables);
    validate_linear_variables(operation, &vars)?;
    let data = dataset();
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
            .map(linear_task)
            .collect();
        return Ok(json!({"issues":linear_connection(std::mem::take(&mut rows),&vars)}));
    }
    if operation == graphql::PROJECTS {
        let rows = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_matches_fixture_subset(v, &vars))
            .map(linear_project)
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
            return Ok(json!({"issue":linear_relations(&data,"task_dependencies",id,"Issue")}));
        }
        return Ok(json!({"issue":item.map(linear_task)}));
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
                json!({"project":linear_relations(&data,"project_dependencies",id,"Project")}),
            );
        }
        return Ok(json!({"project":item.map(linear_project)}));
    }
    Ok(json!({"viewer":{"id":"fixture-user"}}))
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
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: 9\r\n\r\n{{}}"
    )
    .unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
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
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 413 Content Too Large"));
}
fn linear_label(v: &Value) -> Value {
    json!({"id":v["id"],"name":v["name"],"color":null})
}
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This fixture-only mapping and matcher implement the finite shared journey dataset against the accepted 2026-08-24 contract; production parsing and real CLI row assertions independently verify the observable behavior without requiring live credentials.
fn linear_state(v: &Value) -> Value {
    let category = v["category"].as_str().unwrap_or("");
    json!({"name":v["name"],"type":match category{"todo"=>"unstarted","in-progress"=>"started","done"=>"completed","cancelled"=>"canceled",_=>"backlog"}})
}
fn linear_task(v: &Value) -> Value {
    json!({"id":v["id"],"title":v["title"],"description":v["content"],"state":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"project":v.get("project").map(|id|json!({"id":id})),"url":v.get("url"),"createdAt":null,"updatedAt":null})
}
fn linear_project(v: &Value) -> Value {
    json!({"id":v["id"],"name":v["title"],"description":v["content"],"status":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"url":v.get("url"),"createdAt":null,"updatedAt":null})
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
fn linear_relations(data: &Value, key: &str, id: &str, suffix: &str) -> Value {
    let edges = data[key].as_array().unwrap();
    let forward = edges
        .iter()
        .filter(|e| e["from"] == id)
        .map(|e| json!({"type":e["kind"],(format!("related{suffix}")):{"id":e["to"]}}))
        .collect::<Vec<_>>();
    let inverse = edges
        .iter()
        .filter(|e| e["to"] == id)
        .map(|e| json!({"type":e["kind"],(suffix.to_ascii_lowercase()):{"id":e["from"]}}))
        .collect::<Vec<_>>();
    json!({"relations":{"nodes":forward,"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":inverse,"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

fn local_md_block(sandbox: &Sandbox) -> Value {
    let root = sandbox.subdirectory("local-md");
    for (kind, id, front, body) in [
        (
            "tasks",
            "T-1",
            "title: Alpha engine\nstatus: Todo\nlabels: [{id: L-1, name: bug}, {id: L-3, name: core}]\nproject: P-1\nurl: https://example.invalid/T-1\ndepends_on: [T-2]",
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
            "title: Engine\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nurl: https://example.invalid/P-1\ndepends_on: [P-2]",
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
        "command": env!("CARGO_BIN_EXE_onetaskgraph-source"),
        "settings": {"kind": "in-memory", "config": settings},
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
pub fn dataset() -> Value {
    json!({
        "tasks": [
            {"id": "T-1", "title": "Alpha engine", "content": "the engine core",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}, {"id": "L-3", "name": "core"}],
             "project": "P-1", "url": "https://example.invalid/T-1"},
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
             "url": "https://example.invalid/P-1"},
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
            {"from": "T-3", "to": "T-2", "kind": "blocks"},
            {"from": "T-4", "to": "T-2", "kind": "related"}
        ],
        "project_dependencies": [{"from": "P-1", "to": "P-2", "kind": "blocks"}]
    })
}
