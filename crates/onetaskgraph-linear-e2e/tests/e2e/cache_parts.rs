//! A Linear task's comments and reverse edges, and a project's edges both ways, answered from
//! the store's cache, driven through the binary against the loopback Linear, doubling only
//! Linear.
//!
//! With the cache attached a task's `lin1` token carries the digests of its first page of
//! comments, relations and inverse relations, and a project's the digests of its first page of
//! relations and inverse relations, so a read of any of those pinned to a token the entry is at
//! or after sends nothing. Every change to them moves the token — a comment added, edited or
//! deleted, a relation added or removed from either end — and a read pinned to the new listing
//! token reads Linear and answers what an uncached read does. The one member served older than
//! its token is a comment author's display name, which no part covers. A copy filing under two
//! or more projects reads one page of them, which pins them, and no more.

use std::path::Path;

use onetaskgraph_linear::graphql;
use serde_json::{Value, json};

use crate::cache::{Cached, dataset, walk};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_failing};
use crate::linear::{eng_mapping, linear};

/// A change a person makes in Linear.
type Edit = Box<dyn Fn(&LinearWorkspace)>;

/// The workspace every journey here starts from: the shared one, with `L-1` carrying a comment,
/// a third project, and `P-1` depending on `P-2`.
fn held() -> Value {
    let mut held = dataset();
    held["projects"].as_array_mut().expect("projects").push(
        json!({"id": "P-3", "title": "Ops", "content": "About operations.",
                     "status": {"category": "todo", "name": "Todo"}, "labels": []}),
    );
    held["project_dependencies"] = json!([{"from": "P-1", "to": "P-2", "kind": "blocks"}]);
    held["comments"] = json!([{"task": "L-1", "comment": {
        "id": "C-1", "body": "The first word.", "author": "Ada",
        "created_at": "2026-08-24T00:00:00Z", "updated_at": "2026-08-24T00:00:00Z"}}]);
    held
}

/// One of the reads this file holds to the cache: what it is called, the kind of item it pins,
/// that item, and its arguments for a pin.
struct Read {
    name: &'static str,
    kind: &'static str,
    id: &'static str,
    arguments: fn(&str) -> Vec<String>,
}

impl Read {
    fn with(&self, pin: &str) -> Vec<String> {
        (self.arguments)(pin)
    }
}

fn words(line: &str, pin: &str) -> Vec<String> {
    let mut arguments: Vec<String> = line.split(' ').map(str::to_owned).collect();
    arguments.extend(["--pin".to_owned(), pin.to_owned(), "--json".to_owned()]);
    arguments
}

/// A task's comments, as `task comment list` reads them.
const COMMENTS: Read = Read {
    name: "comment list",
    kind: "task",
    id: "linear:L-1",
    arguments: |pin| words("task comment list linear:L-1", pin),
};

/// A task's fields with its comments, as `task show` reads them.
const DETAIL: Read = Read {
    name: "show with comments",
    kind: "task",
    id: "linear:L-1",
    arguments: |pin| words("task show linear:L-1", pin),
};

/// A project's forward edges.
const PROJECT_EDGES: Read = Read {
    name: "project deps",
    kind: "project",
    id: "linear:P-1",
    arguments: |pin| words("project deps linear:P-1", pin),
};

/// A project's reverse edges: `P-1` depends on `P-2`.
const PROJECT_REVERSE_EDGES: Read = Read {
    name: "reverse project deps",
    kind: "project",
    id: "linear:P-2",
    arguments: |pin| words("project deps linear:P-2 --direction depended-on-by", pin),
};

/// A task's reverse edges.
const REVERSE_EDGES: Read = Read {
    name: "reverse task deps",
    kind: "task",
    id: "linear:L-2",
    arguments: |pin| words("task deps linear:L-2 --direction depended-on-by", pin),
};

fn strs(arguments: &[String]) -> Vec<&str> {
    arguments.iter().map(String::as_str).collect()
}

/// The JSON a read printed, without its `cache` member and without any `version`, which the
/// cache-on and cache-off spellings of a token differ in.
fn bare(mut printed: Value) -> Value {
    if let Some(object) = printed.as_object_mut() {
        object.remove("cache");
    }
    strip_versions(&mut printed);
    printed
}

fn strip_versions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("version");
            object.values_mut().for_each(strip_versions);
        }
        Value::Array(values) => values.iter_mut().for_each(strip_versions),
        _ => {}
    }
}

/// Fill the entry `read` answers from at the token its item lists at, and hold the next read of
/// it to sending nothing: what the journeys below start from.
fn filled(cached: &Cached, read: &Read) -> String {
    let pin = cached.listed(read.kind, read.id).expect("a token");
    cached.ok(&strs(&read.with(&pin)));
    let (_, sent) = cached.ok(&strs(&read.with(&pin)));
    assert_eq!(sent, Vec::<String>::new(), "{}: filled", read.name);
    pin
}

#[test]
fn every_comment_and_edge_read_is_a_cold_load_a_hit_on_an_unchanged_pin_and_a_miss_on_a_newer_one()
{
    // (the read, the one document a cold read of it sends, how a person changes it)
    let reads: Vec<(Read, &str, Edit)> = vec![
        (
            COMMENTS,
            graphql::ISSUE_DETAIL_CACHED,
            Box::new(|w| {
                w.add_comment("L-1", "A second word.");
            }),
        ),
        (
            DETAIL,
            graphql::ISSUE_DETAIL_CACHED,
            Box::new(|w| w.edit_comment("C-1", "The first word, edited.")),
        ),
        (
            PROJECT_EDGES,
            graphql::PROJECT_RELATIONS_CACHED,
            Box::new(|w| w.relate_projects("P-1", "P-3")),
        ),
        (
            REVERSE_EDGES,
            graphql::ISSUE_RELATIONS_CACHED,
            Box::new(|w| w.relate("L-3", "L-2", "blocks")),
        ),
        (
            PROJECT_REVERSE_EDGES,
            graphql::PROJECT_RELATIONS_CACHED,
            Box::new(|w| w.relate_projects("P-3", "P-2")),
        ),
    ];
    for (read, cold, change) in reads {
        let cached = Cached::new(held());
        let pin = cached.listed(read.kind, read.id).expect("a token");
        let (loaded, sent) = cached.ok(&strs(&read.with(&pin)));
        assert_eq!(sent, [cold], "{}: a cold read is one request", read.name);
        let (hit, sent) = cached.ok(&strs(&read.with(&pin)));
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "{}: a hit sends nothing",
            read.name
        );
        assert_eq!(
            bare(hit),
            bare(loaded),
            "{}: the hit answers the load",
            read.name
        );

        change(&cached.workspace);
        let newer = cached.listed(read.kind, read.id).expect("a token");
        assert_ne!(newer, pin, "{}: the token moves", read.name);
        let (_, sent) = cached.ok(&strs(&read.with(&newer)));
        assert_eq!(sent, [cold], "{}: a newer pin misses", read.name);
        let [on, off] = cached.on_and_off(&strs(&read.with(&newer)));
        assert_eq!(on.0, off.0, "{}: what an uncached read answers", read.name);
        let (_, sent) = cached.ok(&strs(&read.with(&newer)));
        assert_eq!(sent, Vec::<String>::new(), "{}: refilled", read.name);
    }
}

#[test]
fn a_comments_copy_by_query_reads_only_the_task_whose_comments_moved() {
    let cached = Cached::new(held());
    let copy = [
        "task",
        "copy",
        "--to",
        "notes",
        "--source",
        "linear",
        "--comments",
        "--dry-run",
        "--no-link",
        "--json",
    ];
    let details = |sent: &[String]| {
        sent.iter()
            .filter(|document| *document == graphql::ISSUE_DETAIL_CACHED)
            .count()
    };
    let (cold, sent) = cached.ok(&copy);
    assert_eq!(details(&sent), 4, "a cold copy loads each task: {sent:?}");
    let (warm, sent) = cached.ok(&copy);
    assert!(
        !sent.iter().any(|document| {
            document == graphql::ISSUE_DETAIL_CACHED || document == graphql::ISSUE_RELATIONS_CACHED
        }),
        "a warm copy reads no task's comments or edges: {sent:?}"
    );
    assert_eq!(bare(warm), bare(cold.clone()));

    cached.workspace.add_comment("L-3", "A word on L-3.");
    let before = cached.workspace.served().len();
    let (moved, _) = cached.ok(&copy);
    let loaded: Vec<Value> = cached.workspace.served()[before..]
        .iter()
        .filter(|(document, _)| document == graphql::ISSUE_DETAIL_CACHED)
        .map(|(_, variables)| variables["id"].clone())
        .collect();
    assert_eq!(loaded, [json!("L-3")], "only the task whose comments moved");
    let [on, off] = cached.on_and_off(&copy);
    assert_eq!(on.0, off.0, "what an uncached copy reads");
    assert_eq!(bare(moved), on.0.1);
}

#[test]
fn a_comment_or_edge_read_its_token_cannot_vouch_for_is_never_kept_or_served() {
    // (the case, how it is made, the read, the entry directory nothing may be kept in, and the
    // connection whose first page reported another page)
    let cases: Vec<(&str, Edit, Read, &str, &str)> = vec![
        (
            "eleven comments",
            Box::new(|w| {
                for n in 0..10 {
                    w.add_comment("L-1", &format!("Word {n}."));
                }
            }),
            COMMENTS,
            "tasks",
            "/comments/pageInfo/hasNextPage",
        ),
        (
            "eleven comments, read beside the task",
            Box::new(|w| {
                for n in 0..10 {
                    w.add_comment("L-1", &format!("Word {n}."));
                }
            }),
            DETAIL,
            "tasks",
            "/comments/pageInfo/hasNextPage",
        ),
        (
            "eleven inverse relations",
            Box::new(|w| {
                for n in 0..10 {
                    w.relate(["L-3", "L-4"][n % 2], "L-2", "related");
                }
            }),
            REVERSE_EDGES,
            "reverse-edges-v2",
            "/inverseRelations/pageInfo/hasNextPage",
        ),
        (
            "eleven project relations",
            Box::new(|w| {
                for n in 0..10 {
                    w.relate_projects("P-1", ["P-2", "P-3"][n % 2]);
                }
            }),
            PROJECT_EDGES,
            "project-edges-v2",
            "/relations/pageInfo/hasNextPage",
        ),
        (
            "eleven projects depending on one",
            Box::new(|w| {
                for n in 0..10 {
                    w.relate_projects(["P-1", "P-3"][n % 2], "P-2");
                }
            }),
            PROJECT_REVERSE_EDGES,
            "project-reverse-edges-v2",
            "/inverseRelations/pageInfo/hasNextPage",
        ),
    ];
    for (case, make, read, directory, more) in cases {
        let cached = Cached::new(held());
        make(&cached.workspace);
        assert_eq!(
            cached.listed(read.kind, read.id),
            None,
            "{case}: no token from part of a connection"
        );
        // The listing's first page said there was more.
        let listing = cached
            .workspace
            .answered()
            .into_iter()
            .rev()
            .find(|answer| {
                answer.pointer("/data/issues").is_some()
                    || answer.pointer("/data/projects").is_some()
            })
            .expect("a listing answered");
        let native = read.id.strip_prefix("linear:").unwrap();
        let row = ["/data/issues/nodes", "/data/projects/nodes"]
            .iter()
            .filter_map(|pointer| listing.pointer(pointer).and_then(Value::as_array))
            .flatten()
            .find(|row| row["id"] == native)
            .expect("the item is listed");
        assert_eq!(row.pointer(more), Some(&json!(true)), "{case}: {row}");
        for _ in 0..2 {
            let [on, off] = cached.on_and_off(&strs(&read.with("lin1|anything")));
            assert!(on.1 >= 1, "{case}: always read from Linear");
            // The same answer: a smaller page walks there in one request more.
            assert_eq!(on.0.0, off.0.0, "{case}: exit code");
            assert_eq!(
                on.0.1["items"], off.0.1["items"],
                "{case}: what an uncached read answers"
            );
        }
        let kept = cached
            .dir()
            .map(|dir| walk(&dir.join(directory)))
            .unwrap_or_default();
        assert!(
            !kept.iter().any(|path| holds(path, native)),
            "{case}: nothing is kept without a token: {kept:?}"
        );
    }

    // A copy by query reads a task with eleven comments every time, and the rest once.
    let cached = Cached::new(held());
    for n in 0..10 {
        cached.workspace.add_comment("L-1", &format!("Word {n}."));
    }
    let copy = [
        "task",
        "copy",
        "--to",
        "notes",
        "--source",
        "linear",
        "--comments",
        "--dry-run",
        "--no-link",
        "--json",
    ];
    for pass in 0..3 {
        let before = cached.workspace.served().len();
        cached.ok(&copy);
        let mut loaded: Vec<String> = cached.workspace.served()[before..]
            .iter()
            .filter(|(document, _)| document == graphql::ISSUE_DETAIL_CACHED)
            .map(|(_, variables)| variables["id"].as_str().unwrap_or_default().to_owned())
            .collect();
        loaded.sort();
        let expected: &[&str] = if pass == 0 {
            &["L-1", "L-2", "L-3", "L-4"]
        } else {
            &["L-1"]
        };
        assert_eq!(
            loaded, expected,
            "copy {pass}: the task with no token, every time"
        );
    }

    // An unordered pin — another item's token, an earlier release's spelling of this one, and a
    // value that is no token at all — reads Linear every time, and the held pin is still a hit.
    for (read, other) in [
        (COMMENTS, "linear:L-3"),
        (DETAIL, "linear:L-3"),
        (REVERSE_EDGES, "linear:L-3"),
        (PROJECT_EDGES, "linear:P-3"),
        (PROJECT_REVERSE_EDGES, "linear:P-3"),
    ] {
        let cached = Cached::new(held());
        let pin = filled(&cached, &read);
        let other = cached.listed(read.kind, other).expect("a token");
        let earlier = earlier_spelling(read.kind, &pin);
        for unordered in [other.as_str(), earlier.as_str(), "not-a-token", "gh3|x"] {
            let (_, sent) = cached.ok(&strs(&read.with(unordered)));
            assert_eq!(sent.len(), 1, "{}: {unordered} reads Linear", read.name);
        }
        let (_, sent) = cached.ok(&strs(&read.with(&pin)));
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "{}: the held pin is a hit",
            read.name
        );
    }
}

/// `token`, of an item of `kind`, as the release before this one spelled an attached token: its
/// time and facets, and — for a task — a relations part alone, where a project had no part.
fn earlier_spelling(kind: &str, token: &str) -> String {
    let mut parts = token.split('|');
    let (prefix, updated, facets) = (
        parts.next().expect("lin1"),
        parts.next().expect("a time"),
        parts.next().expect("the facets"),
    );
    let base = format!("{prefix}|{updated}|{facets}");
    if kind == "task" {
        format!("{base}|r:{}", "0".repeat(64))
    } else {
        base
    }
}

/// Whether the entry file at `path` names the native id `native`.
fn holds(path: &Path, native: &str) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|entry| entry["id"] == native || entry["task"]["id"] == native)
}

#[test]
fn every_comment_and_relation_change_is_read_again_pinned_to_the_new_token() {
    // (the change, how a person makes it, the read it reaches, whether it moves the token)
    let changes: Vec<(&str, Edit, Read, bool)> = vec![
        (
            "a comment added",
            Box::new(|w| {
                w.add_comment("L-1", "A second word.");
            }),
            COMMENTS,
            true,
        ),
        (
            "a comment's body edited",
            Box::new(|w| w.edit_comment("C-1", "The first word, edited.")),
            COMMENTS,
            true,
        ),
        (
            "a comment deleted",
            Box::new(|w| w.delete_comment("C-1")),
            COMMENTS,
            true,
        ),
        (
            "a comment added, read beside the task",
            Box::new(|w| {
                w.add_comment("L-1", "A second word.");
            }),
            DETAIL,
            true,
        ),
        (
            "a comment deleted, read beside the task",
            Box::new(|w| w.delete_comment("C-1")),
            DETAIL,
            true,
        ),
        (
            "a project relation added on the near project",
            Box::new(|w| w.relate_projects("P-1", "P-3")),
            PROJECT_EDGES,
            true,
        ),
        (
            "a project relation removed on the near project",
            Box::new(|w| w.unrelate_projects("P-1", "P-2")),
            PROJECT_EDGES,
            true,
        ),
        (
            "a project far end deleted",
            Box::new(|w| w.remove_project("P-2")),
            PROJECT_EDGES,
            false,
        ),
        (
            "a blocks relation added by the far issue",
            Box::new(|w| w.relate("L-3", "L-2", "blocks")),
            REVERSE_EDGES,
            true,
        ),
        (
            "a related relation added by the far issue",
            Box::new(|w| w.relate("L-3", "L-2", "related")),
            REVERSE_EDGES,
            true,
        ),
        (
            "a blocks relation removed by the far issue",
            Box::new(|w| w.unrelate("L-1", "L-2")),
            REVERSE_EDGES,
            true,
        ),
        (
            "a related relation removed by the far issue",
            Box::new(|w| w.unrelate("L-4", "L-2")),
            REVERSE_EDGES,
            true,
        ),
        (
            "the far issue trashed",
            Box::new(|w| w.trash_issue("L-1")),
            REVERSE_EDGES,
            false,
        ),
        (
            "a project relation added by the far project",
            Box::new(|w| w.relate_projects("P-3", "P-2")),
            PROJECT_REVERSE_EDGES,
            true,
        ),
        (
            "a project relation removed by the far project",
            Box::new(|w| w.unrelate_projects("P-1", "P-2")),
            PROJECT_REVERSE_EDGES,
            true,
        ),
        (
            "the far project deleted",
            Box::new(|w| w.remove_project("P-1")),
            PROJECT_REVERSE_EDGES,
            false,
        ),
    ];
    for (change, make, read, moves) in changes {
        let mut workspace = held();
        if change == "a related relation removed by the far issue" {
            workspace["task_dependencies"]
                .as_array_mut()
                .expect("relations")
                .push(json!({"from": "L-4", "to": "L-2", "kind": "related"}));
        }
        let cached = Cached::new(workspace);
        let pin = filled(&cached, &read);
        make(&cached.workspace);
        let current = cached.listed(read.kind, read.id).expect("a token");
        assert_eq!(current != pin, moves, "{change}: whether the token moves");
        let (_, sent) = cached.ok(&strs(&read.with(&current)));
        assert_eq!(
            sent.len(),
            usize::from(moves),
            "{change}: a moved token is never the entry"
        );
        let [on, off] = cached.on_and_off(&strs(&read.with(&current)));
        assert_eq!(on.0, off.0, "{change}: what an uncached read answers");
    }
}

#[test]
fn a_comment_authors_renamed_display_name_is_the_one_member_served_older_than_its_token() {
    let cached = Cached::new(held());
    let pin = filled(&cached, &COMMENTS);
    cached
        .workspace
        .rename_comment_author("Ada", "Ada Lovelace");
    assert_eq!(
        cached.listed("task", "linear:L-1").as_deref(),
        Some(pin.as_str()),
        "a display name moves no part of the token"
    );
    for read in [COMMENTS, DETAIL] {
        let (served, sent) = cached.ok(&strs(&read.with(&pin)));
        let [_, off] = cached.on_and_off(&strs(&read.with(&pin)));
        if read.name == DETAIL.name {
            // The detail read fills on its first pinned read, here, after the rename.
            continue;
        }
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "{}: served from the entry",
            read.name
        );
        let comment = |printed: &Value| printed["comments"][0].clone();
        let (cached_comment, uncached_comment) = (comment(&served), comment(&off.0.1));
        assert_eq!(
            cached_comment["author"], "Ada",
            "the name recorded when it was filled"
        );
        assert_eq!(uncached_comment["author"], "Ada Lovelace");
        let without_author = |mut comment: Value| {
            comment.as_object_mut().expect("a comment").remove("author");
            comment
        };
        assert_eq!(
            without_author(cached_comment),
            without_author(uncached_comment),
            "every other member is the uncached read's"
        );
    }
}

/// Where the cache keeps the `directory` entry of the native id `native`, under `dir`.
fn entry(dir: &Path, directory: &str, native: &str) -> std::path::PathBuf {
    dir.join(directory).join(format!(
        "{}.json",
        onetaskgraph_plugin_api::asset_sha256(native.as_bytes())
    ))
}

/// A comment write's arguments, given the journey's workspace and how to write a body file.
type CommentWrite = fn(&Cached, &dyn Fn(&Cached, &str) -> String) -> Vec<String>;

#[test]
fn every_comment_write_takes_the_entry_its_comments_are_kept_in_out_of_service() {
    let body = |cached: &Cached, text: &str| {
        let path = cached.sandbox.project().join("comment.md");
        std::fs::write(&path, text).expect("a body");
        path.display().to_string()
    };
    // (the write, what the next read shows)
    let writes: Vec<(CommentWrite, &str)> = vec![
        (
            |cached, body| {
                vec![
                    "task".into(),
                    "comment".into(),
                    "add".into(),
                    "linear:L-1".into(),
                    "--body-file".into(),
                    body(cached, "Added through the binary."),
                ]
            },
            "Added through the binary.",
        ),
        (
            |cached, body| {
                vec![
                    "task".into(),
                    "comment".into(),
                    "edit".into(),
                    "linear:L-1".into(),
                    "C-1".into(),
                    "--body-file".into(),
                    body(cached, "Edited through the binary."),
                ]
            },
            "Edited through the binary.",
        ),
        (
            |_, _| {
                ["task", "comment", "delete", "linear:L-1", "C-1"]
                    .map(str::to_owned)
                    .to_vec()
            },
            "\"comments\":[]",
        ),
    ];
    for (write, shows) in writes {
        let cached = Cached::new(held());
        let pin = filled(&cached, &COMMENTS);
        let dir = cached.dir().expect("a cache");
        assert!(entry(&dir, "tasks", "L-1").exists());
        let mut arguments = write(&cached, &body);
        arguments.push("--json".into());
        cached.ok(&strs(&arguments));
        assert!(
            !entry(&dir, "tasks", "L-1").exists(),
            "`{}` takes the entry out of service",
            arguments.join(" ")
        );
        let (shown, sent) = cached.ok(&strs(&COMMENTS.with(&pin)));
        assert_eq!(
            sent.len(),
            1,
            "the read after `{}` is Linear's",
            arguments.join(" ")
        );
        assert!(shown.to_string().contains(shows), "{shown}");
    }
}

#[test]
fn every_relation_write_takes_the_edges_entries_it_can_change_out_of_service() {
    // A task's dependencies written: the relation lands on the far issue's inverse relations.
    for (write, shows) in [
        (
            [
                "task",
                "update",
                "linear:L-3",
                "--depends-on",
                "linear:L-2",
                "--json",
            ]
            .to_vec(),
            true,
        ),
        (
            ["task", "update", "linear:L-1", "--no-depends-on", "--json"].to_vec(),
            false,
        ),
    ] {
        let cached = Cached::new(held());
        let pin = filled(&cached, &REVERSE_EDGES);
        let dir = cached.dir().expect("a cache");
        assert!(entry(&dir, "reverse-edges-v2", "L-2").exists());
        cached.ok(&write);
        assert!(
            !entry(&dir, "reverse-edges-v2", "L-2").exists(),
            "`{}` takes L-2's reverse edges out of service",
            write.join(" ")
        );
        let (edges, sent) = cached.ok(&strs(&REVERSE_EDGES.with(&pin)));
        assert_eq!(
            sent.len(),
            1,
            "the read after `{}` is Linear's",
            write.join(" ")
        );
        let from_l3 = edges["items"]
            .as_array()
            .expect("edges")
            .iter()
            .any(|edge| edge["from"]["id"] == "linear:L-3");
        assert_eq!(from_l3, shows, "{edges}");
        let from_l1 = edges.to_string().contains("linear:L-1");
        assert_eq!(from_l1, shows, "{edges}");
    }

    // A project's dependencies written: a copy of P-1 out to the folder, with its dependency on
    // P-2 removed there, and back over P-1.
    let cached = Cached::new(held());
    let (copied, _) = cached.ok(&[
        "project",
        "copy",
        "linear:P-1",
        "--to",
        "notes",
        "--no-tasks",
        "--no-link",
        "--json",
    ]);
    let copy = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    let file = cached
        .sandbox
        .project()
        .join("notes/projects")
        .join(format!("{}.md", copy.split_once(':').expect("qualified").1));
    let text = std::fs::read_to_string(&file).expect("the copy");
    assert!(
        text.contains("linear:P-2"),
        "the copy records P-1's dependency: {text}"
    );
    std::fs::write(
        &file,
        text.lines()
            .filter(|line| !line.contains("linear:P-2") && !line.starts_with("depends_on"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("the edit");
    let pin = filled(&cached, &PROJECT_EDGES);
    let reverse_pin = filled(&cached, &PROJECT_REVERSE_EDGES);
    let dir = cached.dir().expect("a cache");
    assert!(entry(&dir, "project-edges-v2", "P-1").exists());
    assert!(entry(&dir, "project-reverse-edges-v2", "P-2").exists());
    let (back, _) = cached.ok(&[
        "project",
        "copy",
        &copy,
        "--to",
        "linear",
        "--no-tasks",
        "--json",
    ]);
    assert_eq!(back["items"][0]["destination"], "linear:P-1", "{back}");
    assert!(
        !entry(&dir, "project-edges-v2", "P-1").exists(),
        "the project write takes P-1's forward edges out of service"
    );
    let (edges, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(sent.len(), 1, "the read after the write is Linear's");
    assert_eq!(edges["items"], json!([]), "{edges}");
    assert!(
        !entry(&dir, "project-reverse-edges-v2", "P-2").exists(),
        "and the far project's reverse edges, which that relation was recorded on too"
    );
    let (depended, sent) = cached.ok(&strs(&PROJECT_REVERSE_EDGES.with(&reverse_pin)));
    assert_eq!(sent.len(), 1, "the far project's next read is Linear's");
    assert_eq!(depended["items"], json!([]), "{depended}");
}

#[test]
fn a_copy_taken_back_takes_every_edges_entry_its_deletes_can_change_out_of_service() {
    let sandbox = crate::common::Sandbox::new();
    let (config, workspace) =
        linear_workspace_failing(&sandbox, held(), &[graphql::ISSUE_RELATION_CREATE]);
    let mut source = linear(&config, json!({"status_mapping": eng_mapping()}));
    source["cache"] = json!({"enabled": true});
    let notes = sandbox.subdirectory("notes");
    for kind in ["tasks", "projects", "documents"] {
        std::fs::create_dir_all(notes.join(kind)).expect("a folder");
    }
    std::fs::write(
        notes.join("projects/NP.md"),
        "---\ntitle: A plan\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("a project");
    std::fs::write(
        notes.join("tasks/NT-1.md"),
        "---\ntitle: First\nstatus: todo\nproject: NP\n---\nFirst.\n",
    )
    .expect("a task");
    std::fs::write(
        notes.join("tasks/NT-2.md"),
        "---\ntitle: Second\nstatus: todo\nproject: NP\ndepends_on: [NT-1]\n---\nSecond.\n",
    )
    .expect("a task");
    sandbox.project_document(&document(&json!({
        "linear": source,
        "notes": {"plugin": "local-md", "config": {"root": notes}},
    })));
    let cached = Cached {
        cache_home: sandbox.subdirectory("cache-home"),
        sandbox,
        workspace,
    };
    filled(&cached, &REVERSE_EDGES);
    let project_pin = filled(&cached, &PROJECT_EDGES);
    let dir = cached.dir().expect("a cache");
    assert!(entry(&dir, "reverse-edges-v2", "L-2").exists());
    assert!(entry(&dir, "project-edges-v2", "P-1").exists());
    filled(&cached, &PROJECT_REVERSE_EDGES);
    assert!(entry(&dir, "project-reverse-edges-v2", "P-2").exists());

    let output = cached.run(&["project", "copy", "notes:NP", "--to", "linear", "--json"]);
    assert_ne!(
        output.status.code(),
        Some(0),
        "the relation write is refused"
    );
    let sent: Vec<String> = cached
        .workspace
        .served()
        .into_iter()
        .map(|(document, _)| document)
        .collect();
    for taken_back in [graphql::ISSUE_DELETE, graphql::PROJECT_DELETE] {
        assert!(
            sent.iter().any(|document| document == taken_back),
            "the copy takes back what it made: {taken_back} in {sent:#?}"
        );
    }
    assert!(
        !entry(&dir, "project-edges-v2", "P-1").exists(),
        "a project deleted can be another project's far end, so every project's forward edges \
         are taken out of service"
    );
    assert!(
        !entry(&dir, "reverse-edges-v2", "L-2").exists(),
        "and every task's reverse edges"
    );
    assert!(
        !entry(&dir, "project-reverse-edges-v2", "P-2").exists(),
        "and every project's reverse edges"
    );
    let (_, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&project_pin)));
    assert_eq!(sent.len(), 1, "the next read is Linear's");
}

#[test]
fn a_copy_reading_a_later_page_of_comments_answers_what_an_uncached_copy_does() {
    let cached = Cached::new(held());
    for n in 0..104 {
        cached.workspace.add_comment("L-1", &format!("Word {n}."));
    }
    let copy = [
        "task",
        "copy",
        "linear:L-1",
        "--to",
        "notes",
        "--comments",
        "--dry-run",
        "--no-link",
        "--json",
    ];
    for _ in 0..2 {
        let (_, sent) = cached.ok(&copy);
        let pages = sent
            .iter()
            .filter(|document| *document == graphql::ISSUE_DETAIL_CACHED)
            .count();
        assert!(pages >= 2, "every copy walks past the first page: {sent:?}");
    }
    let [on, off] = cached.on_and_off(&copy);
    assert_eq!(on.0, off.0, "what an uncached copy reads");
    let kept = cached
        .dir()
        .map(|dir| walk(&dir.join("tasks")))
        .unwrap_or_default();
    assert!(
        !kept.iter().any(|path| holds(path, "L-1")),
        "a task whose comments run past a page gives no token, and no page is kept: {kept:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_relation_write_whose_far_edges_entry_cannot_be_retired_is_refused_unsent() {
    use std::os::unix::fs::PermissionsExt as _;
    let cached = Cached::new(held());
    filled(&cached, &REVERSE_EDGES);
    let dir = cached.dir().expect("a cache").join("reverse-edges-v2");
    let path = entry(&cached.dir().expect("a cache"), "reverse-edges-v2", "L-2");
    // The entry as a symlink to its own bytes, in a directory that will not let it go: it can be
    // neither removed nor emptied, and every process still reads it.
    let held_bytes = dir.join("held-elsewhere");
    std::fs::rename(&path, &held_bytes).expect("moved aside");
    std::os::unix::fs::symlink(&held_bytes, &path).expect("a symlink");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("read-only");
    if std::fs::remove_file(&path).is_ok() {
        // Running as a user no directory mode stops — nothing to prove here.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).ok();
        return;
    }
    let write = [
        "task",
        "update",
        "linear:L-3",
        "--depends-on",
        "linear:L-2",
        "--json",
    ];
    let before = cached.workspace.served().len();
    let output = cached.run(&write);
    let sent: Vec<String> = cached.workspace.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("writable");
    assert_ne!(output.status.code(), Some(0), "the write is refused");
    let stderr = crate::common::stderr(&output);
    assert!(
        stderr.contains(&dir.parent().unwrap().display().to_string()),
        "{stderr}"
    );
    assert!(
        !sent.iter().any(|document| document.starts_with("mutation")),
        "nothing was written: {sent:?}"
    );
    // With the directory writable again, the same write lands and the entry goes.
    cached.ok(&write);
    assert!(
        std::fs::symlink_metadata(&path).is_err(),
        "the entry is retired"
    );
}

#[test]
fn a_project_edge_walk_is_kept_only_when_read_whole_in_one_command() {
    let mut workspace = held();
    workspace["project_dependencies"]
        .as_array_mut()
        .expect("project relations")
        .push(
            json!({"from": {"id": "P-1"}, "to": {"id": "notes:NP-1", "kind": "project"},
                     "kind": "blocks"}),
        );
    let relations = |sent: &[String]| {
        sent.iter()
            .filter(|document| *document == graphql::PROJECT_RELATIONS_CACHED)
            .count()
    };
    // Its relations page, then its recorded far end's page: kept, and served next time.
    let cached = Cached::new(workspace.clone());
    let pin = cached.listed("project", "linear:P-1").expect("a token");
    let (cold, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(relations(&sent), 2, "a two-page walk: {sent:?}");
    assert!(cold.to_string().contains("notes:NP-1"), "{cold}");
    let (hit, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(sent, Vec::<String>::new(), "kept whole");
    assert_eq!(hit["items"], cold["items"], "every edge of both pages");

    // The same walk a page a command resumes a walk the process never began: nothing is kept.
    let cached = Cached::new(workspace);
    let pin = cached.listed("project", "linear:P-1").expect("a token");
    let mut first = PROJECT_EDGES.with(&pin);
    first.extend(["--limit".to_owned(), "1".to_owned()]);
    let (page, _) = cached.ok(&strs(&first));
    let token = page["next"].as_str().expect("a second page").to_owned();
    let mut second = first.clone();
    second.extend(["--page".to_owned(), token]);
    let (rest, _) = cached.ok(&strs(&second));
    assert!(rest.to_string().contains("notes:NP-1"), "{rest}");
    let kept = cached
        .dir()
        .map(|dir| walk(&dir.join("project-edges-v2")))
        .unwrap_or_default();
    assert!(
        kept.is_empty(),
        "a walk split across commands is not kept: {kept:?}"
    );
    let (_, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(relations(&sent), 2, "the next command reads it from Linear");
}

#[test]
fn cache_clear_reclaims_reverse_and_project_edge_entries_and_counts_them() {
    let cached = Cached::new(held());
    filled(&cached, &REVERSE_EDGES);
    filled(&cached, &PROJECT_EDGES);
    filled(&cached, &PROJECT_REVERSE_EDGES);
    let dir = cached.dir().expect("a cache");
    let entries = |directory: &str| walk(&dir.join(directory)).len();
    assert_eq!(
        (
            entries("reverse-edges-v2"),
            entries("project-edges-v2"),
            entries("project-reverse-edges-v2")
        ),
        (1, 1, 1)
    );
    let foreign = dir.join("reverse-edges-v2/not-ours.json");
    std::fs::write(&foreign, b"{\"kind\": \"something else\"}").expect("a foreign file");
    let (cleared, _) = cached.ok(&["cache", "clear", "linear", "--json"]);
    assert!(
        cleared["cleared"][0]["entries"].as_u64() >= Some(3),
        "all three are counted: {cleared}"
    );
    assert_eq!(entries("project-edges-v2"), 0);
    assert_eq!(entries("project-reverse-edges-v2"), 0);
    assert_eq!(
        walk(&dir.join("reverse-edges-v2")),
        [foreign],
        "only this store's entries are removed"
    );
}

#[test]
fn a_reverse_edge_read_a_page_at_a_time_is_never_served_or_kept_as_the_whole() {
    let mut workspace = held();
    workspace["task_dependencies"]
        .as_array_mut()
        .expect("relations")
        .push(json!({"from": "L-3", "to": "L-2", "kind": "blocks"}));
    let cached = Cached::new(workspace);
    let pin = filled(&cached, &REVERSE_EDGES);
    let (whole, _) = cached.ok(&strs(&REVERSE_EDGES.with(&pin)));
    assert_eq!(whole["items"].as_array().map(Vec::len), Some(2), "{whole}");

    // A limit below what is kept reads Linear a page at a time, never the entry.
    let mut first = REVERSE_EDGES.with(&pin);
    first.extend(["--limit".to_owned(), "1".to_owned()]);
    let (page, sent) = cached.ok(&strs(&first));
    assert_eq!(
        sent,
        [graphql::ISSUE_RELATIONS_CACHED],
        "a smaller page is Linear's"
    );
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1), "{page}");
    let token = page["next"].as_str().expect("a second page").to_owned();
    let mut second = first.clone();
    second.extend(["--page".to_owned(), token]);
    let (rest, sent) = cached.ok(&strs(&second));
    assert_eq!(
        sent,
        [graphql::ISSUE_RELATIONS_CACHED],
        "a resumed page is Linear's"
    );
    let mut paged: Vec<Value> = page["items"]
        .as_array()
        .into_iter()
        .chain(rest["items"].as_array())
        .flatten()
        .cloned()
        .collect();
    let mut kept: Vec<Value> = whole["items"].as_array().expect("edges").clone();
    let key = |edge: &Value| edge.to_string();
    paged.sort_by_key(key);
    kept.sort_by_key(key);
    assert_eq!(paged, kept, "the pages hold exactly the edges");
    let [on, off] = cached.on_and_off(&strs(&second));
    assert_eq!(on.0, off.0, "what an uncached resumed page answers");

    // Neither page replaced the whole entry: the next whole read is still served from it.
    let (again, sent) = cached.ok(&strs(&REVERSE_EDGES.with(&pin)));
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(again["items"], whole["items"]);
}

#[test]
fn a_task_created_from_a_template_retires_the_far_ends_reverse_edges() {
    // A create carrying an image asset is `linear_assets.rs`'s, whose loopback serves uploads.
    let cached = Cached::new(held());
    let pin = filled(&cached, &REVERSE_EDGES);
    let dir = cached.dir().expect("a cache");
    assert!(entry(&dir, "reverse-edges-v2", "L-2").exists());
    let template = cached.sandbox.subdirectory("inputs").join("note.md");
    std::fs::write(&template, "A rendered body.\n").expect("a template");
    let template = template.display().to_string();
    let (created, _) = cached.ok(&[
        "task",
        "create",
        "linear",
        "--project",
        "P-1",
        "--title",
        "Depends on L-2",
        "--depends-on",
        "linear:L-2",
        "--template",
        &template,
        "--no-interactive",
        "--json",
    ]);
    assert!(
        !entry(&dir, "reverse-edges-v2", "L-2").exists(),
        "the rendered create takes L-2's reverse edges out of service: {created}"
    );
    let (edges, sent) = cached.ok(&strs(&REVERSE_EDGES.with(&pin)));
    assert_eq!(sent.len(), 1, "the next read is Linear's");
    assert_eq!(
        edges["items"].as_array().map(Vec::len),
        Some(2),
        "L-1's edge and the new task's: {edges}"
    );
}

#[test]
fn a_project_edge_walk_read_across_a_change_between_its_pages_is_never_kept() {
    let mut workspace = held();
    workspace["project_dependencies"]
        .as_array_mut()
        .expect("project relations")
        .push(
            json!({"from": {"id": "P-1"}, "to": {"id": "notes:NP-1", "kind": "project"},
                     "kind": "blocks"}),
        );
    let cached = Cached::new(workspace);
    let pin = cached.listed("project", "linear:P-1").expect("a token");
    // P-1 renamed after its relations page and before its recorded far end's page.
    cached.workspace.after_request(
        |document, variables| {
            document == graphql::PROJECT_RELATIONS_CACHED && variables["id"] == "P-1"
        },
        0,
        |workspace| workspace.retitle("projects", "P-1", "Renamed between the pages"),
    );
    let (_, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(
        sent.iter()
            .filter(|document| *document == graphql::PROJECT_RELATIONS_CACHED)
            .count(),
        2,
        "the walk read both pages: {sent:?}"
    );
    let kept = cached
        .dir()
        .map(|dir| walk(&dir.join("project-edges-v2")))
        .unwrap_or_default();
    assert!(
        !kept.iter().any(|path| holds(path, "P-1")),
        "a walk read at two versions is not kept: {kept:?}"
    );
    let (_, sent) = cached.ok(&strs(&PROJECT_EDGES.with(&pin)));
    assert_eq!(
        sent.len(),
        2,
        "the next command reads both pages from Linear again"
    );
}

/// Eighty projects — more than the seventy-four one page of them holds, so `PX-79` is past it —
/// and four tasks: three marked `many`, filed under `PX-0`, `PX-1` and `PX-79`, and one marked
/// `one`, filed under `PX-2`.
fn many_projects() -> Value {
    let projects: Vec<Value> = (0..80)
        .map(|n| {
            json!({"id": format!("PX-{n}"), "title": format!("Project {n}"),
                   "content": format!("About {n}."),
                   "status": {"category": "todo", "name": "Todo"}, "labels": []})
        })
        .collect();
    let task = |id: &str, project: &str, case: &str| {
        crate::linear::issue(
            id,
            "Todo",
            json!({"project": project, "metadata": {"pin.case": case}}),
        )
    };
    crate::linear::held(
        vec![
            task("TX-1", "PX-0", "many"),
            task("TX-2", "PX-1", "many"),
            task("TX-3", "PX-79", "many"),
            task("TX-4", "PX-2", "one"),
        ],
        projects,
        Vec::new(),
    )
}

/// A dry-run copy into `notes` of the tasks marked `case`.
fn filing_copy(case: &str) -> Vec<String> {
    [
        "task",
        "copy",
        "--to",
        "notes",
        "--source",
        "linear",
        "--metadata",
        &format!("pin.case={case}"),
        "--dry-run",
        "--no-link",
        "--json",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn a_copy_filing_tasks_under_several_projects_reads_one_page_of_them_and_no_more() {
    let workspace = many_projects();
    let copy = filing_copy;
    let count =
        |sent: &[String], document: &str| sent.iter().filter(|sent| *sent == document).count();
    let cached = Cached::new(workspace);

    // Three projects named, two on the first page: the cold copy reads that page and each
    // project, filling them; the warm one reads the page and only the project past it.
    let (_, cold) = cached.ok(&strs(&copy("many")));
    assert_eq!(
        count(&cold, graphql::PROJECTS_CACHED),
        1,
        "one page: {cold:?}"
    );
    assert_eq!(count(&cold, graphql::PROJECT_CACHED), 3, "{cold:?}");
    let before = cached.workspace.served().len();
    let (warm_answer, warm) = cached.ok(&strs(&copy("many")));
    assert_eq!(count(&warm, graphql::PROJECTS_CACHED), 1, "{warm:?}");
    let read: Vec<Value> = cached.workspace.served()[before..]
        .iter()
        .filter(|(document, _)| document == graphql::PROJECT_CACHED)
        .map(|(_, variables)| variables["id"].clone())
        .collect();
    assert_eq!(
        read,
        [json!("PX-79")],
        "only the project the page did not carry"
    );

    // The same copy with no cache reads each project, and answers the same.
    let [on, off] = cached.on_and_off(&strs(&copy("many")));
    assert_eq!(on.0, off.0, "what an uncached copy answers");
    assert_eq!(bare(warm_answer), on.0.1);

    // One project named: no page is read, and its project is read by id as before.
    let (_, one) = cached.ok(&strs(&copy("one")));
    assert_eq!(count(&one, graphql::PROJECTS_CACHED), 0, "{one:?}");
    assert_eq!(count(&one, graphql::PROJECT_CACHED), 1, "{one:?}");
}

#[test]
fn a_copy_whose_projects_page_is_refused_reads_each_project_by_id_and_answers_the_same() {
    let sandbox = crate::common::Sandbox::new();
    let (config, workspace) =
        linear_workspace_failing(&sandbox, many_projects(), &[graphql::PROJECTS_CACHED]);
    let mut source = linear(&config, json!({"status_mapping": eng_mapping()}));
    source["cache"] = json!({"enabled": true});
    let notes = sandbox.subdirectory("notes");
    for kind in ["tasks", "projects", "documents"] {
        std::fs::create_dir_all(notes.join(kind)).expect("a folder");
    }
    sandbox.project_document(&document(&json!({
        "linear": source,
        "notes": {"plugin": "local-md", "config": {"root": notes}},
    })));
    let cached = Cached {
        cache_home: sandbox.subdirectory("cache-home"),
        sandbox,
        workspace,
    };
    let (refused, sent) = cached.ok(&strs(&filing_copy("many")));
    assert_eq!(
        sent.iter()
            .filter(|document| *document == graphql::PROJECTS_CACHED)
            .count(),
        1,
        "the refused page is not sent again: {sent:?}"
    );
    assert_eq!(
        sent.iter()
            .filter(|document| *document == graphql::PROJECT_CACHED)
            .count(),
        3,
        "each project is read by id instead: {sent:?}"
    );
    let [_, off] = cached.on_and_off(&strs(&filing_copy("many")));
    assert_eq!(bare(refused), off.0.1, "what an uncached copy answers");
}

#[test]
fn a_relation_write_retires_only_this_stores_edges_entries_and_is_refused_when_it_cannot_list_them()
{
    let write = [
        "task",
        "update",
        "linear:L-3",
        "--depends-on",
        "linear:L-2",
        "--json",
    ];
    // A file in the directory that is not one of this store's entries stays where it is.
    let cached = Cached::new(held());
    filled(&cached, &REVERSE_EDGES);
    let directory = cached.dir().expect("a cache").join("reverse-edges-v2");
    let foreign = directory.join("not-ours.json");
    std::fs::write(&foreign, b"{\"kind\": \"something else\"}").expect("a foreign file");
    cached.ok(&write);
    assert_eq!(
        walk(&directory),
        [foreign],
        "this store's entry is retired, and nothing else"
    );

    // A directory this process cannot list refuses the write before anything is sent.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let cached = Cached::new(held());
        filled(&cached, &REVERSE_EDGES);
        let directory = cached.dir().expect("a cache").join("reverse-edges-v2");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o000))
            .expect("unlistable");
        if std::fs::read_dir(&directory).is_ok() {
            // Running as a user no directory mode stops — nothing to prove here.
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).ok();
            return;
        }
        let before = cached.workspace.served().len();
        let output = cached.run(&write);
        let sent: Vec<String> = cached.workspace.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
            .expect("listable");
        assert_ne!(output.status.code(), Some(0), "the write is refused");
        assert!(
            !sent.iter().any(|document| document.starts_with("mutation")),
            "nothing was written: {sent:?}"
        );
        cached.ok(&write);
    }
}

#[test]
fn an_edges_entry_under_another_kinds_path_is_never_served() {
    let cached = Cached::new(held());
    let pin = filled(&cached, &PROJECT_EDGES);
    let dir = cached.dir().expect("a cache");
    // P-1's forward edges, put where P-1's reverse edges would be kept.
    std::fs::create_dir_all(dir.join("project-reverse-edges-v2")).expect("a directory");
    std::fs::copy(
        entry(&dir, "project-edges-v2", "P-1"),
        entry(&dir, "project-reverse-edges-v2", "P-1"),
    )
    .expect("copied");
    let reverse = words("project deps linear:P-1 --direction depended-on-by", &pin);
    let (_, sent) = cached.ok(&strs(&reverse));
    assert_eq!(sent.len(), 1, "read from Linear, never the misplaced entry");
    let [on, off] = cached.on_and_off(&strs(&reverse));
    assert_eq!(on.0, off.0, "what an uncached read answers");
}
