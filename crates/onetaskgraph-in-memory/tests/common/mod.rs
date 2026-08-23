//! The work two differently-capable sources are configured over.
//!
//! Shared by every test below so the *only* difference between two sources is the
//! capability block — which is the whole reason this plugin takes one.

use onetaskgraph_in_memory::InMemoryConfig;
use serde_json::{Value, json};

/// A configuration block over a small graph: two projects, four tasks (one of
/// them an orphan), three labels, and dependency edges in both tables.
#[must_use]
pub fn work() -> Value {
    json!({
        "projects": [
            project("P-1", "Foundation", "backlog", "Planned", ["infra"]),
            project("P-2", "Plugins", "in-progress", "Building", []),
        ],
        "tasks": [
            task("T-1", "Land the contract", Some("two crates, one direction"),
                 "in-progress", "In Review", ["infra", "p1"], Some("P-1")),
            task("T-2", "Wire the graph", Some("nx affected is the gate"),
                 "todo", "Todo", ["infra"], Some("P-1")),
            task("T-3", "Write the linear plugin", None,
                 "backlog", "Someday", ["p1"], Some("P-2")),
            task("T-4", "Loose end", Some("belongs to nothing"),
                 "done", "Shipped", [], None),
        ],
        "labels": [
            { "id": "l-1", "name": "infra", "color": "#336699" },
            { "id": "l-2", "name": "p1", "color": null },
            { "id": "l-3", "name": "wontfix", "color": null },
        ],
        "task_dependencies": [
            { "from": "T-2", "to": "T-1", "kind": "blocks" },
            { "from": "T-3", "to": "T-1", "kind": "blocks" },
            { "from": "T-4", "to": "T-2", "kind": "related" },
        ],
        "project_dependencies": [
            { "from": "P-2", "to": "P-1", "kind": "blocks" },
        ],
    })
}

/// [`work`] with `capabilities` merged in, so two sources differ in exactly that.
#[must_use]
pub fn with_capabilities(capabilities: Value) -> InMemoryConfig {
    let mut config = work();
    config["capabilities"] = capabilities;
    serde_json::from_value(config).expect("the fixture is a valid configuration block")
}

fn task(
    id: &str,
    title: &str,
    content: Option<&str>,
    category: &str,
    name: &str,
    labels: impl IntoIterator<Item = &'static str>,
    project: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "title": title,
        "content": content,
        "status": { "category": category, "name": name },
        "labels": labels.into_iter().map(label).collect::<Vec<_>>(),
        "project": project,
        "url": null,
        "created_at": null,
        "updated_at": null,
    })
}

fn project(
    id: &str,
    title: &str,
    category: &str,
    name: &str,
    labels: impl IntoIterator<Item = &'static str>,
) -> Value {
    json!({
        "id": id,
        "title": title,
        "content": null,
        "status": { "category": category, "name": name },
        "labels": labels.into_iter().map(label).collect::<Vec<_>>(),
        "url": null,
        "created_at": null,
        "updated_at": null,
    })
}

fn label(name: &str) -> Value {
    json!({ "id": format!("l-{name}"), "name": name, "color": null })
}
