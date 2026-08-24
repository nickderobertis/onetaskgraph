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

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use serde_json::{Value, json};

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
        }),
    },
    Row {
        plugin: "in-memory",
        name: "in-memory (declares nothing native, forward-only)",
        fixture: Fixture::Ready(Ready {
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
        }),
    },
    Row {
        plugin: "subprocess",
        name: "subprocess (the in-memory source over a real pipe)",
        fixture: Fixture::Ready(Ready {
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
        }),
    },
    Row {
        plugin: "local-md",
        name: "local-md",
        fixture: Fixture::Ready(Ready {
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
        }),
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Fixture::Pending,
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Fixture::Ready(Ready {
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
        }),
    },
];

fn github_projects_block(sandbox: &Sandbox) -> Value {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("GitHub fixture connection");
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("GraphQL query string");
            let variables = &request["variables"];
            let data = if query.contains("node(id:$id)") {
                let id = variables["id"].as_str().expect("dependency id");
                let blockers = match id {
                    "T-1" | "T-3" | "T-4" => vec![json!({"id":"T-2","projectItems":{"nodes":[]}})],
                    _ => vec![],
                };
                json!({"node":{"__typename":"Issue","blockedBy":{"nodes":blockers,"pageInfo":{"hasNextPage":false,"endCursor":null}}}})
            } else {
                github_project_page(variables)
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

fn github_project_page(variables: &Value) -> Value {
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
    let end = (offset + first).min(tasks.len());
    let nodes = tasks[offset..end]
        .iter()
        .map(|(id, title, body, status, state, labels)| json!({
            "id": format!("ITEM-{id}"),
            "fieldValues":{"nodes":[{"name":status,"field":{"name":"Status"}}]},
            "content":{
                "id":id,"title":title,"body":body,"state":state,
                "url":format!("https://example.invalid/{id}"),
                "labels":{"nodes":labels.iter().map(|(id, name)| json!({"id":id,"name":name})).collect::<Vec<_>>()}
            }
        }))
        .collect::<Vec<_>>();
    json!({
        "organization":{"projectV2":{
            "id":"P-1","title":"Engine","shortDescription":"alpha engine project",
            "url":"https://example.invalid/P-1","closed":false,
            "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < tasks.len(),"endCursor":end.to_string()}}
        }},
        "user":{"projectV2":null}
    })
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
