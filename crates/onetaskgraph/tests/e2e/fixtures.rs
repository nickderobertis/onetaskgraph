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

/// The two `in-memory` rows configured side by side over the same dataset.
///
/// This is the pair the whole capability mechanism exists for: one query, two sources of
/// deliberately different declared capability, one correct answer and two different
/// plans — and, for dependencies, one reverse answer the source gives and one the engine
/// scans for, which must match edge for edge.
pub fn pair(sandbox: &Sandbox) -> String {
    pair_at(sandbox, SourceBoundary::Direct)
}

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
        fixture: Fixture::Pending,
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Fixture::Pending,
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Fixture::Pending,
    },
];

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
