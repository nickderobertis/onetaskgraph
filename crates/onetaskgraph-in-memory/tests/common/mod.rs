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

/// The same work with a document table beside it, and the declaration that gives it one.
///
/// Separate from [`with_capabilities`] rather than folded into [`work`]: `documents`
/// declares whether a source has documents *at all*, so the fixture every other test uses
/// has to go on having none — a source that grew a document table would stop being the
/// document-free source those tests are about.
///
/// One document carries a link and one carries a path, which is the pair a consumer has to
/// be able to tell apart; the third carries neither, because "the source did not say" is
/// its own case and not the same as being nowhere.
#[must_use]
pub fn with_documents(capabilities: Value) -> InMemoryConfig {
    let mut config = work();
    config["capabilities"] = capabilities;
    config["capabilities"]["documents"] = json!("native");
    config["documents"] = json!([
        document(
            "D-1",
            "Contract review",
            Some("the two crates, one direction"),
            ["infra"],
            Some("P-1"),
            json!({"url": "https://example.invalid/D-1"})
        ),
        document(
            "D-2",
            "Plugin notes",
            Some("how a plugin is hosted"),
            ["p1"],
            Some("P-2"),
            json!({"path": "/srv/notes/D-2.md"})
        ),
        document("D-3", "Loose note", None, [], None, Value::Null),
    ]);
    serde_json::from_value(config).expect("the fixture is a valid configuration block")
}

fn document(
    id: &str,
    title: &str,
    content: Option<&str>,
    labels: impl IntoIterator<Item = &'static str>,
    project: Option<&str>,
    location: Value,
) -> Value {
    json!({
        "id": id,
        "title": title,
        "content": content,
        "project": project,
        "labels": labels.into_iter().map(label).collect::<Vec<_>>(),
        "url": null,
        "location": location,
        "created_at": null,
        "updated_at": null,
    })
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
