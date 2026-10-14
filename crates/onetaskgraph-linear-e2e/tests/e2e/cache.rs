//! A Linear source with its local cache on, driven through the binary against the loopback
//! Linear, doubling only Linear.
//!
//! Linear's `lin1` token — an item's `updatedAt`, the digest of its facets and, with the cache
//! attached, the digest of a task's first page of relations — covers a task's fields and forward
//! edges, a project's fields and a document's fields, so a read pinned to a token the entry is at
//! or after sends nothing. Every change `spike-linear-coverage` measured `updatedAt` alone missing,
//! and every member the facets embed, moves the token: a read pinned to the new listing token
//! reads Linear and answers what an uncached read does. A comment, a project's forward edges and
//! every reverse edge are always read from Linear.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};
use crate::linear::{eng_mapping, held, issue, linear};

/// A loopback Linear workspace configured as the source `linear` with its cache on, beside an
/// empty folder of Markdown called `notes`.
pub(crate) struct Cached {
    pub(crate) sandbox: Sandbox,
    pub(crate) workspace: LinearWorkspace,
    pub(crate) cache_home: PathBuf,
}

/// What one command answered, as two answers are compared — its exit code, its JSON and its
/// stderr — and how many requests it sent.
pub(crate) type Answered = ((Option<i32>, Value, String), usize);

/// A change a person makes in Linear.
type Edit = Box<dyn Fn(&LinearWorkspace)>;

/// A label as a workspace row carries it.
fn label(id: &str, name: &str, color: &str) -> Value {
    json!({"id": id, "name": name, "color": color})
}

/// The workspace every journey here starts from: four issues — `L-1` labelled and depending on
/// `L-2`, `L-4` recording a far end of another source — two projects and two documents.
pub(crate) fn dataset() -> Value {
    let mut workspace = held(
        vec![
            issue(
                "L-1",
                "Todo",
                json!({"labels": [label("LB-1", "bug", "#ff0000")], "project": "P-1",
                       "priority": "high"}),
            ),
            issue("L-2", "In Progress", json!({"project": "P-1"})),
            issue("L-3", "Todo", json!({})),
            issue("L-4", "Todo", json!({"project": "P-1"})),
        ],
        vec![
            json!({"id": "P-1", "title": "Engine", "content": "About the engine.",
                   "status": {"category": "in-progress", "name": "In Progress"},
                   "labels": [label("LB-9", "core", "#00ff00")]}),
            json!({"id": "P-2", "title": "Docs", "content": "About the docs.",
                   "status": {"category": "todo", "name": "Todo"}, "labels": []}),
        ],
        vec![
            json!({"id": "D-1", "title": "Design", "content": "The engine's design.",
                   "project": "P-1"}),
            json!({"id": "D-2", "title": "Runbook", "content": "How to run it."}),
        ],
    );
    workspace["labels"] = json!([
        label("LB-1", "bug", "#ff0000"),
        label("LB-9", "core", "#00ff00")
    ]);
    workspace["task_dependencies"] = json!([
        {"from": "L-1", "to": "L-2", "kind": "blocks"},
        {"from": {"id": "L-4"}, "to": {"id": "notes:N-1", "kind": "task"}, "kind": "blocks"},
    ]);
    workspace
}

impl Cached {
    pub(crate) fn new(held: Value) -> Self {
        Self::with(held, &json!({"enabled": true}))
    }

    pub(crate) fn with(held: Value, cache: &Value) -> Self {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace(&sandbox, held);
        let mut source = linear(&config, json!({"status_mapping": eng_mapping()}));
        source["cache"] = cache.clone();
        let notes = sandbox.subdirectory("notes");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(notes.join(kind)).expect("a folder");
        }
        // The far end `L-4` records, so a graph naming it resolves.
        std::fs::write(
            notes.join("tasks/N-1.md"),
            "---\ntitle: A note\nstatus: todo\n---\nElsewhere.\n",
        )
        .expect("the far end");
        sandbox.project_document(&document(&json!({
            "linear": source,
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        Self {
            sandbox,
            workspace,
            cache_home,
        }
    }

    pub(crate) fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// One command that had to succeed: what it printed, and every document Linear answered for
    /// it.
    pub(crate) fn ok(&self, arguments: &[&str]) -> (Value, Vec<String>) {
        let before = self.workspace.served().len();
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let sent = self.workspace.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        (
            serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            sent,
        )
    }

    /// `arguments` answered with the cache on and again with it off for that command: each
    /// answer's exit code, JSON without its `cache` member and without any `version` — a token
    /// carries its relations part only with the cache attached — and stderr, and how many
    /// requests each sent.
    pub(crate) fn on_and_off(&self, arguments: &[&str]) -> [Answered; 2] {
        let answer = |arguments: &[&str]| {
            let before = self.workspace.served().len();
            let output = self.run(arguments);
            let mut printed: Value = serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null);
            if let Some(object) = printed.as_object_mut() {
                object.remove("cache");
            }
            unversioned(&mut printed);
            (
                (output.status.code(), printed, stderr(&output)),
                self.workspace.served().len() - before,
            )
        };
        let on = answer(arguments);
        let mut off = vec!["--set", "sources.linear.cache.enabled=false"];
        off.extend_from_slice(arguments);
        [on, answer(&off)]
    }

    /// The token `<kind> list` carries for `id` — what a caller takes from a listing.
    pub(crate) fn listed(&self, kind: &str, id: &str) -> Option<String> {
        let (listing, _) = self.ok(&[
            kind, "list", "--source", "linear", "--json", "--limit", "100",
        ]);
        listing["items"]
            .as_array()
            .expect("a listing")
            .iter()
            .find(|item| item["id"] == id)
            .unwrap_or_else(|| panic!("{kind} list carries {id}: {listing}"))["item"]["version"]
            .as_str()
            .map(str::to_owned)
    }

    /// The one cache directory under the cache home, when anything made one.
    pub(crate) fn dir(&self) -> Option<PathBuf> {
        std::fs::read_dir(self.cache_home.join("onetaskgraph"))
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .next()
    }
}

/// `value` with every `version` member taken out, however deep.
fn unversioned(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("version");
            object.values_mut().for_each(unversioned);
        }
        Value::Array(values) => values.iter_mut().for_each(unversioned),
        _ => {}
    }
}

/// `<kind> show <id> --json`, pinned to `pin`, reading a task without its comments — the read
/// whose every part a Linear token covers.
pub(crate) fn pinned(kind: &str, id: &str, pin: &str) -> Vec<String> {
    let mut arguments = vec![kind.to_owned(), "show".to_owned(), id.to_owned()];
    if kind == "task" {
        arguments.push("--no-comments".to_owned());
    }
    arguments.extend(["--pin".to_owned(), pin.to_owned(), "--json".to_owned()]);
    arguments
}

fn strs(arguments: &[String]) -> Vec<&str> {
    arguments.iter().map(String::as_str).collect()
}

/// Every file under `root`.
pub(crate) fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn every_cached_linear_read_is_a_cold_load_a_hit_on_an_unchanged_pin_and_a_miss_on_a_newer_one() {
    let cached = Cached::new(dataset());
    // (kind, id, how a person changes it, what the changed read shows)
    let reads: Vec<(&str, &str, Edit, &str)> = vec![
        (
            "task",
            "linear:L-1",
            Box::new(|workspace| workspace.retitle("tasks", "L-1", "Renamed issue")),
            "Renamed issue",
        ),
        (
            "project",
            "linear:P-1",
            Box::new(|workspace| workspace.retitle("projects", "P-1", "Renamed project")),
            "Renamed project",
        ),
        (
            "document",
            "linear:D-1",
            Box::new(|workspace| workspace.retitle("documents", "D-1", "Renamed document")),
            "Renamed document",
        ),
    ];
    for (kind, id, change, shows) in reads {
        let held = cached.listed(kind, id).expect("a token");
        assert!(held.starts_with("lin1|"), "{held}");
        let read = pinned(kind, id, &held);
        let (cold, sent) = cached.ok(&strs(&read));
        assert_eq!(
            sent.len(),
            1,
            "{kind}: a cold read is one request: {sent:?}"
        );
        let (hit, sent) = cached.ok(&strs(&read));
        assert_eq!(sent, Vec::<String>::new(), "{kind}: a hit sends nothing");
        assert_eq!(hit["items"], cold["items"], "{kind}");

        change(&cached.workspace);
        let newer = cached.listed(kind, id).expect("a token");
        assert_ne!(newer, held, "{kind}: the token moves");
        let read = pinned(kind, id, &newer);
        let (current, sent) = cached.ok(&strs(&read));
        assert_eq!(sent.len(), 1, "{kind}: a newer pin misses");
        assert_eq!(current["items"][0]["item"]["title"], shows, "{kind}");
    }

    // Many tasks without their comments, each pinned: the same read, the same entries.
    let pin = cached.listed("task", "linear:L-1").expect("a token");
    let (_, sent) = cached.ok(&[
        "task",
        "show-many",
        "linear:L-1",
        "--no-comments",
        "--pin",
        &format!("linear:L-1={pin}"),
        "--json",
    ]);
    assert_eq!(
        sent,
        Vec::<String>::new(),
        "show-many of a held task sends nothing"
    );
}

#[test]
fn a_projects_tasks_listed_then_their_forward_edges_are_answered_from_the_cache_in_the_next_command()
 {
    let cached = Cached::new(dataset());
    let graph = ["project", "graph", "linear:P-1", "--format", "json"];
    let relations = |sent: &[String]| {
        sent.iter()
            .filter(|document| *document == onetaskgraph_linear::graphql::ISSUE_RELATIONS_CACHED)
            .count()
    };
    let (cold, sent) = cached.ok(&graph);
    // L-1's relations; L-2's; L-4's, then its recorded far end's page.
    assert_eq!(
        relations(&sent),
        4,
        "a cold graph reads each task's edges: {sent:?}"
    );
    let (warm, sent) = cached.ok(&graph);
    assert_eq!(
        relations(&sent),
        0,
        "the next command's listing pins each task to a token its kept edges are at: {sent:?}"
    );
    assert_eq!(warm, cold, "what the cache answers is what Linear answered");

    // A relation added moves nothing Linear's `updatedAt` can see — but the relations digest:
    // L-1's listed token is newer, so L-1's edges alone are read again.
    cached.workspace.relate("L-1", "L-4", "related");
    let (changed, sent) = cached.ok(&graph);
    assert_eq!(
        relations(&sent),
        1,
        "only the task whose relations moved: {sent:?}"
    );
    let [on, off] = cached.on_and_off(&graph);
    assert_eq!(on.0, off.0, "what an uncached graph answers");
    assert_eq!(on.0.1, changed);
    assert_ne!(changed, cold);
}

#[test]
fn a_token_linear_cannot_vouch_for_is_never_kept_or_served() {
    // (what makes the token absent or unordered, the issue, how)
    let cases: Vec<(&str, Edit)> = vec![
        (
            "fifty labels, Linear's default page",
            Box::new(|workspace| workspace.crowd_labels("tasks", "L-3", 50)),
        ),
        (
            "eleven relations, more than the token's page",
            Box::new(|workspace| {
                for far in 0..11 {
                    workspace.relate("L-3", &format!("L-{}", [1, 2, 4][far % 3]), "related");
                }
            }),
        ),
    ];
    for (case, make) in cases {
        let cached = Cached::new(dataset());
        make(&cached.workspace);
        assert_eq!(
            cached.listed("task", "linear:L-3"),
            None,
            "{case}: no token from part of a connection"
        );
        for _ in 0..2 {
            let (_, sent) = cached.ok(&strs(&pinned("task", "linear:L-3", "lin1|anything")));
            assert_eq!(sent.len(), 1, "{case}: always read from Linear");
        }
        let dir = cached.dir();
        assert!(
            dir.as_deref()
                .is_none_or(|dir| walk(&dir.join("tasks")).is_empty()),
            "{case}: nothing is kept without a token"
        );
    }

    // An unordered pin — another token's facets — and a value that is no token at all.
    let cached = Cached::new(dataset());
    let held = cached.listed("task", "linear:L-1").expect("a token");
    cached.ok(&strs(&pinned("task", "linear:L-1", &held)));
    let other = cached.listed("task", "linear:L-3").expect("a token");
    for pin in [other.as_str(), "not-a-token", "gh3|x"] {
        let (_, sent) = cached.ok(&strs(&pinned("task", "linear:L-1", pin)));
        assert_eq!(sent.len(), 1, "{pin}: read from Linear");
    }
    let (_, sent) = cached.ok(&strs(&pinned("task", "linear:L-1", &held)));
    assert_eq!(sent, Vec::<String>::new(), "the held pin is still a hit");
}

/// One change a person makes in Linear, named, with what the changed read shows of it.
type Change = (&'static str, Edit, &'static str);

#[test]
fn every_change_measured_or_covered_by_construction_is_read_again_pinned_to_the_new_token() {
    let slot = |text: &str, key: &str| {
        format!(
            "{text}\n\n<!-- onetaskgraph.metadata\n{}\n-->",
            json!({key: "noted"})
        )
    };
    let task: Vec<Change> = vec![
        (
            "title",
            Box::new(|w| w.retitle("tasks", "L-1", "Retitled")),
            "Retitled",
        ),
        (
            "description",
            Box::new(|w| w.edit_description("L-1", "A new body.")),
            "A new body.",
        ),
        (
            "metadata slot",
            Box::new(move |w| w.edit_description("L-1", &slot("The body of L-1.", "caller.note"))),
            "noted",
        ),
        (
            "state change",
            Box::new(|w| w.move_to_state("L-1", "In Progress", "started")),
            "in-progress",
        ),
        (
            "workflow-state rename",
            Box::new(|w| w.rename_state("Todo", "To Do")),
            "To Do",
        ),
        (
            "priority",
            Box::new(|w| w.set_priority("L-1", "low")),
            "low",
        ),
        (
            "label added",
            Box::new(|w| w.set_labels("tasks", "L-1", &[("LB-1", "bug"), ("LB-2", "ui")])),
            "\"ui\"",
        ),
        (
            "label renamed",
            Box::new(|w| w.rename_label("LB-1", "defect")),
            "defect",
        ),
        (
            "label recoloured",
            Box::new(|w| w.recolour_label("LB-1", "#000000")),
            "#000000",
        ),
        (
            "label removed",
            Box::new(|w| w.set_labels("tasks", "L-1", &[])),
            "\"labels\":[]",
        ),
        (
            "project removed",
            Box::new(|w| w.file_issue("L-1", None)),
            "\"project\":null",
        ),
        (
            "project added",
            Box::new(|w| w.file_issue("L-1", Some("P-2"))),
            "\"P-2\"",
        ),
        (
            "team move",
            Box::new(|w| w.move_team("L-1", "OPS-7")),
            "OPS-7",
        ),
    ];
    let project: Vec<Change> = vec![
        (
            "name",
            Box::new(|w| w.retitle("projects", "P-1", "Kernel")),
            "Kernel",
        ),
        (
            "content",
            Box::new(|w| w.edit_project_content("P-1", "About the kernel.")),
            "About the kernel.",
        ),
        (
            "status change",
            Box::new(|w| w.set_project_status("P-1", "Done", "completed")),
            "Done",
        ),
        (
            "project-status rename",
            Box::new(|w| w.rename_project_status("In Progress", "Underway")),
            "Underway",
        ),
        (
            "label added",
            Box::new(|w| w.set_labels("projects", "P-1", &[("LB-9", "core"), ("LB-2", "ui")])),
            "\"ui\"",
        ),
        (
            "label removed",
            Box::new(|w| w.set_labels("projects", "P-1", &[])),
            "\"labels\":[]",
        ),
        (
            "label renamed",
            Box::new(|w| w.rename_label("LB-9", "kernel")),
            "kernel",
        ),
        (
            "label recoloured",
            Box::new(|w| w.recolour_label("LB-9", "#0000ff")),
            "#0000ff",
        ),
    ];
    let document: Vec<Change> = vec![
        (
            "title",
            Box::new(|w| w.retitle("documents", "D-1", "Design, again")),
            "Design, again",
        ),
        (
            "content",
            Box::new(|w| w.edit_document_content("D-1", "The design, revised.")),
            "The design, revised.",
        ),
        (
            "an asset reference in the slot",
            Box::new(move |w| {
                w.edit_document_content(
                    "D-1",
                    "The engine's design.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.assets\":{\"shot.png\":{\"sha256\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"url\":\"https://uploads.linear.app/x/shot.png\"}}}\n-->",
                );
            }),
            "shot.png",
        ),
        (
            "refiled under another project",
            Box::new(|w| w.refile_document("D-1", Some("P-2"))),
            "\"P-2\"",
        ),
    ];
    for (kind, id, changes) in [
        ("task", "linear:L-1", task),
        ("project", "linear:P-1", project),
        ("document", "linear:D-1", document),
    ] {
        for (change, make, shows) in changes {
            let cached = Cached::new(dataset());
            let held = cached.listed(kind, id).expect("a token");
            cached.ok(&strs(&pinned(kind, id, &held)));
            let (_, sent) = cached.ok(&strs(&pinned(kind, id, &held)));
            assert_eq!(sent, Vec::<String>::new(), "{kind} {change}: filled");

            make(&cached.workspace);
            let current = cached.listed(kind, id).expect("a token");
            assert_ne!(current, held, "{kind} {change}: the token moves");
            let read = pinned(kind, id, &current);
            let (shown, sent) = cached.ok(&strs(&read));
            assert_eq!(sent.len(), 1, "{kind} {change}: never the entry");
            assert!(
                shown.to_string().contains(shows),
                "{kind} {change}: the changed value, {shows}: {shown}"
            );
            let [on, off] = cached.on_and_off(&strs(&read));
            assert_eq!(
                on.0, off.0,
                "{kind} {change}: what an uncached read answers"
            );
        }
    }
}

#[test]
fn a_relation_added_removed_or_whose_far_end_is_trashed_reads_as_an_uncached_read_does() {
    let changes: Vec<(&str, Edit)> = vec![
        (
            "blocks added",
            Box::new(|w| w.relate("L-1", "L-3", "blocks")),
        ),
        (
            "related added",
            Box::new(|w| w.relate("L-1", "L-3", "related")),
        ),
        ("blocks removed", Box::new(|w| w.unrelate("L-1", "L-2"))),
        ("related removed", Box::new(|w| w.unrelate("L-1", "L-4"))),
        ("far end trashed", Box::new(|w| w.trash_issue("L-2"))),
    ];
    let graph = ["project", "graph", "linear:P-1", "--format", "json"];
    let edges = ["task", "deps", "linear:L-1", "--json"];
    for (change, make) in changes {
        let mut held = dataset();
        if change == "related removed" {
            held["task_dependencies"]
                .as_array_mut()
                .expect("relations")
                .push(json!({"from": "L-1", "to": "L-4", "kind": "related"}));
        }
        let cached = Cached::new(held);
        cached.ok(&graph);
        let (_, sent) = cached.ok(&graph);
        assert!(
            !sent.contains(&onetaskgraph_linear::graphql::ISSUE_RELATIONS_CACHED.to_owned()),
            "{change}: the edges are kept"
        );
        make(&cached.workspace);
        let [on, off] = cached.on_and_off(&graph);
        assert_eq!(on.0, off.0, "{change}: the graph an uncached read draws");
        let [on, off] = cached.on_and_off(&edges);
        assert_eq!(on.0, off.0, "{change}: the edges an uncached read answers");
        assert_eq!(
            on.1, off.1,
            "{change}: a reverse-or-unpinned read costs what it did"
        );
    }
}

#[test]
fn a_listing_alone_fills_no_entry() {
    let cached = Cached::new(dataset());
    for kind in ["task", "project", "document"] {
        cached.ok(&[kind, "list", "--source", "linear", "--json"]);
    }
    assert!(
        cached.dir().is_none_or(|dir| walk(&dir)
            .iter()
            .all(|path| !path.to_string_lossy().ends_with(".json"))),
        "a listing pins and fills nothing"
    );
}

#[test]
fn every_linear_read_kept_remote_reaches_linear_every_time_and_costs_what_it_did() {
    let cached = Cached::new(dataset());
    let comment = cached
        .workspace
        .add_comment("L-1", "a comment only Linear holds");
    // Fill every entry a read can fill, so each remote read below has one present beside it.
    let pins: Vec<(&str, &str, String)> = [
        ("task", "linear:L-1"),
        ("project", "linear:P-1"),
        ("document", "linear:D-1"),
    ]
    .iter()
    .map(|(kind, id)| (*kind, *id, cached.listed(kind, id).expect("a token")))
    .collect();
    for (kind, id, pin) in &pins {
        cached.ok(&strs(&pinned(kind, id, pin)));
    }
    cached.ok(&["project", "graph", "linear:P-1", "--format", "json"]);
    let task_pin = pins[0].2.clone();
    let remote: Vec<Vec<&str>> = vec![
        vec![
            "task",
            "comment",
            "list",
            "linear:L-1",
            "--pin",
            &task_pin,
            "--json",
        ],
        vec!["task", "show", "linear:L-1", "--pin", &task_pin, "--json"],
        vec![
            "task",
            "copy",
            "linear:L-1",
            "--to",
            "notes",
            "--comments",
            "--no-link",
            "--json",
        ],
        vec!["project", "deps", "linear:P-1", "--json"],
        vec![
            "project",
            "deps",
            "linear:P-2",
            "--direction",
            "depended-on-by",
            "--json",
        ],
        vec![
            "task",
            "deps",
            "linear:L-2",
            "--direction",
            "depended-on-by",
            "--json",
        ],
        vec!["task", "list", "--source", "linear", "--json"],
        vec!["project", "list", "--source", "linear", "--json"],
        vec!["document", "list", "--source", "linear", "--json"],
        vec!["label", "list", "--source", "linear", "--json"],
        vec!["sources", "fields", "linear", "--json"],
    ];
    for arguments in &remote {
        for _ in 0..2 {
            let [on, off] = cached.on_and_off(arguments);
            assert!(
                on.1 > 0,
                "`{}` reaches Linear every time",
                arguments.join(" ")
            );
            assert_eq!(
                on.1,
                off.1,
                "`{}` sends exactly what it sends with no cache",
                arguments.join(" ")
            );
        }
    }
    // No comment and no author's name is ever written where an entry is kept.
    let dir = cached.dir().expect("a cache");
    for path in walk(&dir) {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !text.contains("a comment only Linear holds") && !text.contains("a person"),
            "{} keeps a comment",
            path.display()
        );
    }
    assert!(!comment.is_empty());
}

/// Where the cache keeps the `directory` entry of the native id `native`, under `dir`.
fn entry(dir: &Path, directory: &str, native: &str) -> PathBuf {
    dir.join(directory).join(format!(
        "{}.json",
        onetaskgraph_plugin_api::asset_sha256(native.as_bytes())
    ))
}

#[test]
fn every_linear_write_takes_the_entries_naming_its_item_out_of_service() {
    let body = |cached: &Cached| {
        let path = cached.sandbox.project().join("body.md");
        std::fs::write(&path, "A body written whole.").expect("a body");
        path.display().to_string()
    };
    // (kind, id, its entries' directory, the write, what the next read shows)
    type Write = (
        &'static str,
        &'static str,
        &'static str,
        Vec<String>,
        &'static str,
    );
    let writes = |cached: &Cached| -> Vec<Write> {
        let words = |line: &str| line.split(' ').map(str::to_owned).collect::<Vec<_>>();
        vec![
            (
                "task",
                "linear:L-1",
                "tasks",
                words("task status set linear:L-1 done"),
                "\"done\"",
            ),
            (
                "task",
                "linear:L-1",
                "tasks",
                words("task priority set linear:L-1 low"),
                "\"low\"",
            ),
            (
                "task",
                "linear:L-1",
                "tasks",
                words("task update linear:L-1 --title Updated"),
                "Updated",
            ),
            (
                "task",
                "linear:L-1",
                "tasks",
                words("task metadata set linear:L-1 caller.note \"noted\""),
                "noted",
            ),
            (
                "task",
                "linear:L-1",
                "tasks",
                vec![
                    "task".to_owned(),
                    "content".to_owned(),
                    "set".to_owned(),
                    "linear:L-1".to_owned(),
                    "--file".to_owned(),
                    body(cached),
                ],
                "A body written whole.",
            ),
            (
                "project",
                "linear:P-1",
                "projects-v1",
                words("project metadata set linear:P-1 caller.note \"noted\""),
                "noted",
            ),
            (
                "document",
                "linear:D-1",
                "documents-v1",
                words("document metadata set linear:D-1 caller.note \"noted\""),
                "noted",
            ),
        ]
    };
    let probe = Cached::new(dataset());
    for (kind, id, directory, write, shows) in writes(&probe) {
        let cached = Cached::new(dataset());
        let held = cached.listed(kind, id).expect("a token");
        cached.ok(&strs(&pinned(kind, id, &held)));
        let (_, sent) = cached.ok(&strs(&pinned(kind, id, &held)));
        assert_eq!(sent, Vec::<String>::new(), "{id} is filled");
        let native = id.strip_prefix("linear:").unwrap();
        let dir = cached.dir().expect("a cache");
        assert!(entry(&dir, directory, native).exists());
        let mut arguments = strs(&write);
        arguments.push("--json");
        cached.ok(&arguments);
        assert!(
            !entry(&dir, directory, native).exists(),
            "`{}` takes {id}'s entry out of service",
            write.join(" ")
        );
        let (shown, sent) = cached.ok(&strs(&pinned(kind, id, &held)));
        assert_eq!(
            sent.len(),
            1,
            "{id}: the read after `{}` is Linear's",
            write.join(" ")
        );
        assert!(
            shown.to_string().contains(shows),
            "{id} answers what `{}` left: {shown}",
            write.join(" ")
        );
    }

    // A copy into Linear over an issue it copied before takes that issue's entry out of service.
    let cached = Cached::new(dataset());
    let notes = cached.sandbox.project().join("notes");
    std::fs::write(
        notes.join("tasks/N-2.md"),
        "---\ntitle: Copied in\nstatus: todo\n---\nFrom the folder.\n",
    )
    .expect("a task");
    let (copied, _) = cached.ok(&["task", "copy", "notes:N-2", "--to", "linear", "--json"]);
    let landed = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    let held = cached.listed("task", &landed).expect("a token");
    cached.ok(&strs(&pinned("task", &landed, &held)));
    std::fs::write(
        notes.join("tasks/N-2.md"),
        std::fs::read_to_string(notes.join("tasks/N-2.md"))
            .unwrap()
            .replace("title: Copied in", "title: Copied in again"),
    )
    .expect("a task");
    cached.ok(&["task", "copy", "notes:N-2", "--to", "linear", "--json"]);
    let (shown, sent) = cached.ok(&strs(&pinned("task", &landed, &held)));
    assert_eq!(sent.len(), 1, "the copied-over issue is read from Linear");
    assert_eq!(shown["items"][0]["item"]["title"], "Copied in again");
}
