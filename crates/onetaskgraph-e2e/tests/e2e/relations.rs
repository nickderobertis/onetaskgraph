//! A task's parent and typed links between tasks, through the binary, over folders of Markdown
//! and an in-memory source: `task parent set-many`, `task link add-many`, `task link
//! remove-many` and `task relations`, each answered per item.
//!
//! Every folder journey reads what one command wrote back through a later command, so what is
//! asserted is what the folder holds. An in-memory source's work dies with its process, so its
//! journeys seed a parent in its configuration and assert each call's own answer.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};

struct Store {
    sandbox: Sandbox,
    /// The folder `work`, holding `t1` … `t4`.
    work: PathBuf,
    /// The folder `other`, holding `o1`.
    other: PathBuf,
}

impl Store {
    /// Folders `work` (four tasks) and `other` (one task), an empty folder `copies`, and an
    /// in-memory source `memory` whose `m2` is seeded under `m1`.
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let work = sandbox.subdirectory("work");
        let other = sandbox.subdirectory("other");
        sandbox.subdirectory("copies");
        for (root, native, title) in [
            (&work, "t1", "Root cause"),
            (&work, "t2", "Symptom"),
            (&work, "t3", "Client symptom"),
            (&work, "t4", "Another symptom"),
            (&other, "o1", "Elsewhere"),
        ] {
            std::fs::create_dir_all(root.join("tasks")).expect("a task folder");
            std::fs::write(
                root.join("tasks").join(format!("{native}.md")),
                format!("---\ntitle: {title}\nstatus: todo\n---\nThe {native} body.\n"),
            )
            .expect("a task file");
        }
        let store = Self {
            sandbox,
            work,
            other,
        };
        store.configure(&json!({}));
        store
    }

    /// Write the configuration, `work`'s `config` extended by `extra`.
    fn configure(&self, extra: &Value) {
        let mut work = json!({"root": self.work});
        if let (Some(work), Some(extra)) = (work.as_object_mut(), extra.as_object()) {
            work.extend(extra.clone());
        }
        let status = |category: &str| json!({"category": category, "name": category});
        let document = json!({
            "sources": {
                "work": {"plugin": "local-md", "config": work},
                "other": {"plugin": "local-md", "config": {"root": self.other}},
                "copies": {"plugin": "local-md",
                           "config": {"root": self.sandbox.project().join("copies")}},
                "memory": {"plugin": "in-memory", "config": {
                    "tasks": [
                        {"id": "m1", "title": "Memory root", "labels": [],
                         "status": status("todo")},
                        {"id": "m2", "title": "Memory child", "labels": [],
                         "status": status("todo")},
                        {"id": "m3", "title": "Memory other", "labels": [],
                         "content": "Unrelated.", "status": status("todo")},
                    ],
                    "parents": {"m2": "m1"},
                }},
            },
        });
        self.sandbox
            .project_document(&serde_json::to_string_pretty(&document).expect("JSON"));
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.arg("--json").args(arguments);
        command.write_stdin(input.unwrap_or_default());
        command.assert().get_output().clone()
    }

    /// `onetaskgraph <arguments>` exiting `code`, with `input` on standard input, as JSON.
    fn exited(&self, arguments: &[&str], input: Option<&str>, code: i32) -> Value {
        let output = self.run(arguments, input);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the run answers JSON")
    }

    fn parents(&self, edges: &Value, code: i32) -> Value {
        let batch = json!({"schema_version": 1, "edges": edges}).to_string();
        self.exited(
            &["task", "parent", "set-many", "--input", "-"],
            Some(&batch),
            code,
        )
    }

    fn links(&self, verb: &str, links: &Value, code: i32) -> Value {
        let batch = json!({"schema_version": 1, "links": links}).to_string();
        self.exited(&["task", "link", verb, "--input", "-"], Some(&batch), code)
    }

    /// `task relations` of `ids`, each item by its id.
    fn relations(&self, ids: &[&str]) -> Vec<Value> {
        let mut arguments = vec!["task", "relations"];
        arguments.extend(ids);
        self.exited(&arguments, None, 0)["items"]
            .as_array()
            .expect("items")
            .clone()
    }

    fn file(&self, root: &std::path::Path, native: &str) -> String {
        std::fs::read_to_string(root.join("tasks").join(format!("{native}.md")))
            .expect("a task file")
    }

    fn shown(&self, id: &str) -> Value {
        self.exited(&["task", "show", id], None, 0)["items"][0]["item"].clone()
    }
}

/// Each result's `outcome`, in order.
fn outcomes(answer: &Value) -> Vec<&str> {
    answer["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect()
}

/// One item's links as `(type, direction, other, other_end)`.
fn links_of(item: &Value) -> Vec<(String, String, String, String)> {
    item["links"]
        .as_array()
        .expect("links")
        .iter()
        .map(|link| {
            (
                link["type"].as_str().expect("a type").to_owned(),
                link["direction"].as_str().expect("a direction").to_owned(),
                link["other"].as_str().expect("an other").to_owned(),
                link["other_end"].as_str().expect("an other end").to_owned(),
            )
        })
        .collect()
}

fn link(kind: &str, direction: &str, other: &str, end: &str) -> (String, String, String, String) {
    (kind.into(), direction.into(), other.into(), end.into())
}

#[test]
fn a_two_level_chain_is_read_back_and_a_re_parent_moves_one_edge() {
    let store = Store::new();
    let written = store.parents(
        &json!([
            {"child": "work:t2", "parent": "work:t1"},
            {"child": "work:t3", "parent": "work:t2"},
            {"child": "work:t4", "parent": "work:t2"},
        ]),
        0,
    );
    assert_eq!(outcomes(&written), ["written", "written", "written"]);
    assert!(written["results"][0]["version"].is_string(), "{written:#}");
    // The file holds the parent beside everything it held, and nothing else moved.
    assert_eq!(
        store.file(&store.work, "t3"),
        "---\ntitle: Client symptom\nstatus: todo\nparent: \"t2\"\n---\nThe t3 body.\n"
    );

    // A later process reads the chain from either end.
    let items = store.relations(&["work:t1", "work:t2", "work:t3"]);
    assert_eq!(items[0]["parent"], Value::Null);
    assert_eq!(items[0]["children"], json!(["work:t2"]));
    assert_eq!(items[1]["parent"], json!("work:t1"));
    assert_eq!(items[1]["children"], json!(["work:t3", "work:t4"]));
    assert_eq!(items[2]["parent"], json!("work:t2"));
    assert_eq!(items[2]["children"], json!([]));
    assert!(items.iter().all(|item| item["children_complete"] == true));

    // The same parent again writes nothing; a re-parent moves t4 alone; a removal clears t3.
    let again = store.parents(
        &json!([
            {"child": "work:t2", "parent": "work:t1"},
            {"child": "work:t4", "parent": "work:t1"},
            {"child": "work:t3", "parent": null},
        ]),
        0,
    );
    assert_eq!(outcomes(&again), ["unchanged", "written", "written"]);
    let items = store.relations(&["work:t1", "work:t2", "work:t3", "work:t4"]);
    assert_eq!(items[0]["children"], json!(["work:t2", "work:t4"]));
    assert_eq!(items[1]["children"], json!([]));
    assert_eq!(items[2]["parent"], Value::Null);
    assert_eq!(items[3]["parent"], json!("work:t1"));
    // A removed parent leaves the file exactly as it was before it had one.
    assert_eq!(
        store.file(&store.work, "t3"),
        "---\ntitle: Client symptom\nstatus: todo\n---\nThe t3 body.\n"
    );
}

#[test]
fn a_cycle_and_a_parent_in_another_source_are_refused_for_their_edge_alone() {
    let store = Store::new();
    store.parents(&json!([{"child": "work:t2", "parent": "work:t1"}]), 0);
    let answer = store.parents(
        &json!([
            {"child": "work:t1", "parent": "work:t2"},
            {"child": "work:t3", "parent": "work:t1"},
            {"child": "work:t4", "parent": "other:o1"},
            {"child": "work:t4", "parent": "work:t4"},
        ]),
        4,
    );
    assert_eq!(
        outcomes(&answer),
        ["refused", "written", "refused", "refused"]
    );
    let reason = |at: usize| answer["results"][at]["reason"].as_str().unwrap_or_default();
    assert!(reason(0).contains("cycle"), "{answer:#}");
    assert!(
        reason(2).contains("work:t4") && reason(2).contains("other:o1"),
        "the refusal names both: {answer:#}"
    );
    assert!(reason(3).contains("cycle"), "{answer:#}");
    let items = store.relations(&["work:t1", "work:t3", "work:t4"]);
    assert_eq!(items[0]["parent"], Value::Null, "the cycle wrote nothing");
    assert_eq!(items[1]["parent"], json!("work:t1"));
    assert_eq!(items[2]["parent"], Value::Null);
}

#[test]
fn a_link_reads_from_both_ends_and_repeats_answer_exists_then_absent() {
    let store = Store::new();
    let added = store.links(
        "add-many",
        &json!([
            {"from": "work:t2", "to": "work:t1", "type": "caused-by"},
            {"from": "work:t3", "to": "work:t1", "type": "caused-by"},
        ]),
        0,
    );
    assert_eq!(outcomes(&added), ["written", "written"]);
    assert!(added["results"][0]["from_version"].is_string(), "{added:#}");

    let items = store.relations(&["work:t1", "work:t2", "work:t3"]);
    assert_eq!(
        links_of(&items[0]),
        [
            link("caused-by", "in", "work:t2", "present"),
            link("caused-by", "in", "work:t3", "present"),
        ]
    );
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "out", "work:t1", "present")]
    );
    assert!(items.iter().all(|item| item["links_complete"] == true));
    // Asked about one end alone, the other is not read.
    assert_eq!(
        links_of(&store.relations(&["work:t2"])[0]),
        [link("caused-by", "out", "work:t1", "unread")]
    );

    let again = store.links(
        "add-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    assert_eq!(outcomes(&again), ["exists"]);

    let removed = store.links(
        "remove-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    assert_eq!(outcomes(&removed), ["written"]);
    let gone = store.links(
        "remove-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    assert_eq!(outcomes(&gone), ["absent"]);
    let items = store.relations(&["work:t1", "work:t2"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "in", "work:t3", "unread")]
    );
    assert!(links_of(&items[1]).is_empty());
    // The end whose last link went is exactly the file it was before it had one.
    assert_eq!(
        store.file(&store.work, "t2"),
        "---\ntitle: Symptom\nstatus: todo\n---\nThe t2 body.\n"
    );
}

#[test]
fn the_links_section_is_in_the_file_and_never_in_the_content_a_read_answers() {
    let store = Store::new();
    store.links(
        "add-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    let file = store.file(&store.work, "t2");
    assert!(file.contains("\n## Links\n"), "{file}");
    assert!(file.contains("- **caused-by** → [Root cause]("), "{file}");
    assert!(file.contains("`work:t1`"), "{file}");
    assert!(file.contains("onetaskgraph.links"), "{file}");

    let shown = store.shown("work:t2");
    assert_eq!(shown["content"], json!("The t2 body."));
    assert_eq!(
        shown["metadata"]["onetaskgraph.links"],
        json!([{"type": "caused-by", "direction": "out", "other": "work:t1"}])
    );
    let target = store.shown("work:t1");
    assert_eq!(target["content"], json!("The t1 body."));
}

#[test]
fn a_link_across_two_sources_is_recorded_at_both() {
    let store = Store::new();
    let added = store.links(
        "add-many",
        &json!([{"from": "work:t1", "to": "other:o1", "type": "relates"}]),
        0,
    );
    assert_eq!(outcomes(&added), ["written"]);
    let items = store.relations(&["work:t1", "other:o1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("relates", "out", "other:o1", "present")]
    );
    assert_eq!(
        links_of(&items[1]),
        [link("relates", "in", "work:t1", "present")]
    );
    assert!(store.file(&store.other, "o1").contains("`work:t1`"));
}

#[test]
fn a_link_one_end_lost_reads_missing_from_the_other_and_a_repeated_add_restores_it() {
    let store = Store::new();
    store.links(
        "add-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    // A person rewrites t1's file whole, front matter and all.
    std::fs::write(
        store.work.join("tasks").join("t1.md"),
        "---\ntitle: Root cause\nstatus: todo\n---\nRewritten.\n",
    )
    .expect("the rewrite");
    let items = store.relations(&["work:t1", "work:t2"]);
    assert!(links_of(&items[0]).is_empty());
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "out", "work:t1", "missing")]
    );
    let restored = store.links(
        "add-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    assert_eq!(outcomes(&restored), ["written"]);
    let items = store.relations(&["work:t1", "work:t2"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "in", "work:t2", "present")]
    );
    assert_eq!(store.shown("work:t1")["content"], json!("Rewritten."));
}

#[test]
fn a_copy_of_a_linked_child_carries_neither_its_parent_nor_its_links() {
    let store = Store::new();
    store.parents(&json!([{"child": "work:t3", "parent": "work:t2"}]), 0);
    store.links(
        "add-many",
        &json!([{"from": "work:t3", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    let copied = store.exited(&["task", "copy", "work:t3", "--to", "copies"], None, 0);
    let landed = copied["items"][0]["destination"]
        .as_str()
        .or_else(|| copied["items"][0]["result"]["id"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the copy lands: {copied:#}"));
    let landed = if landed.contains(':') {
        landed
    } else {
        format!("copies:{landed}")
    };
    let shown = store.shown(&landed);
    assert_eq!(shown["content"], json!("The t3 body."));
    assert!(
        shown["metadata"].get("onetaskgraph.links").is_none(),
        "{shown:#}"
    );
    let native = landed.split_once(':').expect("qualified").1;
    let file = store.file(&store.sandbox.project().join("copies"), native);
    assert!(!file.contains("parent:"), "{file}");
    assert!(!file.contains("## Links"), "{file}");
    assert!(!file.contains("onetaskgraph.links"), "{file}");
    let items = store.relations(&[&landed]);
    assert_eq!(items[0]["parent"], Value::Null);
    assert!(links_of(&items[0]).is_empty());
}

#[test]
fn an_in_memory_source_reads_its_seeded_parent_and_answers_its_writes() {
    let store = Store::new();
    let items = store.relations(&["memory:m1", "memory:m2"]);
    assert_eq!(items[0]["children"], json!(["memory:m2"]));
    assert_eq!(items[1]["parent"], json!("memory:m1"));
    // Within one process: a cycle refused beside an edge written, and a link written at both
    // ends of an in-memory task and a folder's.
    let answer = store.parents(
        &json!([
            {"child": "memory:m1", "parent": "memory:m2"},
            {"child": "memory:m3", "parent": "memory:m2"},
        ]),
        4,
    );
    assert_eq!(outcomes(&answer), ["refused", "written"]);
    let added = store.links(
        "add-many",
        &json!([{"from": "memory:m3", "to": "work:t1", "type": "relates"}]),
        0,
    );
    assert_eq!(outcomes(&added), ["written"]);
    assert!(
        store
            .file(&store.work, "t1")
            .contains("{\"type\":\"relates\",\"direction\":\"in\",\"other\":\"memory:m3\"}"),
        "{}",
        store.file(&store.work, "t1")
    );
}

#[test]
fn a_native_mirror_named_on_a_source_with_none_is_refused_naming_the_source_type_and_word() {
    let store = Store::new();
    store.configure(&json!({"link_types": {"caused-by": {"native": "related"}}}));
    let output = store.run(&["task", "relations", "work:t1"], None);
    assert_ne!(output.status.code(), Some(0));
    let said = format!("{}{}", stdout(&output), stderr(&output));
    for named in ["work", "caused-by", "`related`"] {
        assert!(said.contains(named), "names {named}: {said}");
    }
    // A type kept in metadata alone is accepted.
    store.configure(&json!({"link_types": {"caused-by": {}}}));
    store.relations(&["work:t1"]);
}

#[test]
fn a_task_holding_no_relation_is_stored_and_read_exactly_as_before_relations_existed() {
    // The committed fixtures are the stored forms of a task holding no relation as the release
    // before relations wrote them: a folder's file after a copy and an update, and an in-memory
    // task as `task show` reads it, its change token included.
    let store = Store::new();
    store.exited(&["task", "copy", "memory:m3", "--to", "copies"], None, 0);
    let copies = store.sandbox.project().join("copies").join("tasks");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&copies)
        .expect("the copy's folder")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 1, "{files:?}");
    let native = files[0]
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("a stem")
        .to_owned();
    store.exited(
        &[
            "task",
            "update",
            &format!("copies:{native}"),
            "--title",
            "Memory other, updated",
        ],
        None,
        0,
    );
    let stored = std::fs::read_to_string(&files[0]).expect("the copy");
    assert_eq!(
        stored.replace(&native, "<native>"),
        include_str!("fixtures/relations/unrelated-local-md.md"),
    );
    let shown = store.shown("memory:m3");
    assert_eq!(
        serde_json::to_string_pretty(&shown).expect("JSON") + "\n",
        include_str!("fixtures/relations/unrelated-in-memory.json"),
    );
}

/// The `## Links` section of a task file — its heading and every line after it, up to the
/// stored answers block or the comments section when the file has one.
fn section(file: &str) -> &str {
    let Some(start) = file.find("## Links\n") else {
        return "";
    };
    let rest = &file[start..];
    let end = ["<!-- onetaskgraph:", "## Comments"]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
    rest[..end].trim_end()
}

#[test]
fn every_content_write_of_a_linked_task_keeps_its_links_section() {
    let store = Store::new();
    store.links(
        "add-many",
        &json!([{"from": "work:t2", "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    let rendered = section(&store.file(&store.work, "t2")).to_owned();
    assert!(rendered.contains("[Root cause]("), "{rendered}");

    // `task content set`: the new content, then the very section the link write rendered.
    let body = store.sandbox.project().join("body.md");
    std::fs::write(&body, "Rewritten by content set.").expect("a body");
    let path = body.to_str().expect("a path");
    store.exited(
        &["task", "content", "set", "work:t2", "--file", path],
        None,
        0,
    );
    let file = store.file(&store.work, "t2");
    assert!(
        file.contains("Rewritten by content set.\n\n## Links\n"),
        "{file}"
    );
    assert_eq!(section(&file), rendered);
    assert_eq!(
        store.shown("work:t2")["content"],
        json!("Rewritten by content set.")
    );

    // `task update --body-file`: the same.
    std::fs::write(&body, "Rewritten by update.").expect("a body");
    store.exited(&["task", "update", "work:t2", "--body-file", path], None, 0);
    let file = store.file(&store.work, "t2");
    assert_eq!(section(&file), rendered);
    assert_eq!(
        store.shown("work:t2")["content"],
        json!("Rewritten by update.")
    );
    // And the link still reads from both ends.
    let items = store.relations(&["work:t1", "work:t2"]);
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "out", "work:t1", "present")]
    );
}

#[test]
fn a_copy_over_a_parented_linked_counterpart_keeps_its_parent_and_its_section() {
    let store = Store::new();
    let first = store.exited(
        &["task", "copy", "work:t1", "work:t2", "--to", "copies"],
        None,
        0,
    );
    let landed: Vec<String> = first["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            item["destination"]
                .as_str()
                .unwrap_or_else(|| panic!("each lands: {first:#}"))
                .to_owned()
        })
        .collect();
    let (one, two) = (&landed[0], &landed[1]);
    store.parents(&json!([{"child": two, "parent": one}]), 0);
    store.links(
        "add-many",
        &json!([{"from": two, "to": one, "type": "caused-by"}]),
        0,
    );
    let copies = store.sandbox.project().join("copies");
    let native = two.split_once(':').expect("qualified").1;
    let rendered = section(&store.file(&copies, native)).to_owned();
    assert!(!rendered.is_empty());

    // The source changes and is copied again over its counterpart.
    store.exited(
        &["task", "update", "work:t2", "--title", "Symptom, revised"],
        None,
        0,
    );
    let again = store.exited(&["task", "copy", "work:t2", "--to", "copies"], None, 0);
    assert_eq!(again["items"][0]["destination"], json!(two), "{again:#}");
    let file = store.file(&copies, native);
    assert!(file.contains("title: Symptom, revised"), "{file}");
    assert_eq!(section(&file), rendered);
    let items = store.relations(&[two]);
    assert_eq!(items[0]["parent"], json!(one));
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", one, "unread")]
    );
    assert_eq!(store.shown(two)["content"], json!("The t2 body."));
}

#[test]
fn the_text_renderings_name_each_item_and_what_became_of_it() {
    let store = Store::new();
    let text = |arguments: &[&str], input: &str, code: i32| {
        let mut command = store.sandbox.command();
        command.args(arguments);
        command.write_stdin(input);
        let output = command.assert().get_output().clone();
        assert_eq!(output.status.code(), Some(code), "{}", stderr(&output));
        stdout(&output)
    };
    let parents = text(
        &["task", "parent", "set-many", "--input", "-"],
        &json!({"schema_version": 1, "edges": [
            {"child": "work:t2", "parent": "work:t1"},
            {"child": "work:t3", "parent": "other:o1"},
        ]})
        .to_string(),
        4,
    );
    let lines: Vec<&str> = parents.lines().collect();
    assert!(
        lines[0].starts_with("work:t2 → work:t1  written  version "),
        "{parents}"
    );
    assert!(
        lines[1].starts_with("work:t3 → other:o1  refused  "),
        "{parents}"
    );
    let links = text(
        &["task", "link", "add-many", "--input", "-"],
        &json!({"schema_version": 1, "links": [
            {"from": "work:t3", "to": "work:t1", "type": "caused-by"},
        ]})
        .to_string(),
        0,
    );
    assert_eq!(links.trim_end(), "work:t3 -[caused-by]-> work:t1  written");
    let relations = text(&["task", "relations", "work:t1", "work:t2"], "", 0);
    assert_eq!(
        relations,
        "work:t1\n  parent: (none)\n  children: work:t2\n  caused-by ← work:t3  (other end \
         unread)\nwork:t2\n  parent: work:t1\n  children: (none)\n"
    );
}

#[test]
fn every_malformed_input_is_refused_naming_it_and_the_rest_of_the_call_goes_on() {
    let store = Store::new();
    // A document of another schema version is refused whole, writing nothing.
    let refused = store.exited(
        &["task", "link", "add-many", "--input", "-"],
        Some(&json!({"schema_version": 2, "links": []}).to_string()),
        1,
    );
    assert_eq!(
        refused["failure"]["kind"],
        json!("invalid-batch"),
        "{refused:#}"
    );
    let refused = store.exited(
        &["task", "parent", "set-many", "--input", "-"],
        Some(&json!({"schema_version": 2, "edges": []}).to_string()),
        1,
    );
    assert_eq!(
        refused["failure"]["kind"],
        json!("invalid-batch"),
        "{refused:#}"
    );
    // An edge leaving `parent` out is refused rather than read as a removal: `null` removes.
    let refused = store.exited(
        &["task", "parent", "set-many", "--input", "-"],
        Some(&json!({"schema_version": 1, "edges": [{"child": "work:t2"}]}).to_string()),
        1,
    );
    assert_eq!(
        refused["failure"]["kind"],
        json!("invalid-batch"),
        "{refused:#}"
    );
    assert!(
        refused["failure"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("parent"),
        "{refused:#}"
    );

    // Each malformed edge and link is its own error, beside one that is written.
    let edges = store.parents(
        &json!([
            {"child": "t1", "parent": "work:t2"},
            {"child": "work:t1", "parent": "work"},
            {"child": "work:nowhere", "parent": "work:t1"},
            {"child": "work:t1", "parent": "work:nowhere"},
            {"child": "elsewhere:t1", "parent": "elsewhere:t2"},
            {"child": "work:t2", "parent": "work:t1"},
        ]),
        4,
    );
    assert_eq!(
        outcomes(&edges),
        ["error", "error", "error", "error", "error", "written"]
    );
    let reason = |answer: &Value, at: usize| {
        answer["results"][at]["reason"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert!(reason(&edges, 0).contains("\"t1\""), "{edges:#}");
    assert!(reason(&edges, 2).contains("work:nowhere"), "{edges:#}");
    assert!(reason(&edges, 3).contains("work:nowhere"), "{edges:#}");
    assert!(reason(&edges, 4).contains("elsewhere"), "{edges:#}");

    let links = store.links(
        "add-many",
        &json!([
            {"from": "work:t1", "to": "work:t1", "type": "x"},
            {"from": "work:t1", "to": "work:t2", "type": "  "},
            {"from": "work:t1", "to": "work:nowhere", "type": "x"},
            {"from": "elsewhere:t1", "to": "work:t2", "type": "x"},
            {"from": "t1", "to": "work:t2", "type": "x"},
            {"from": "work:t3", "to": "work:t4", "type": "x"},
        ]),
        4,
    );
    assert_eq!(
        outcomes(&links),
        ["error", "error", "error", "error", "error", "written"]
    );
    assert!(reason(&links, 0).contains("itself"), "{links:#}");
    assert!(reason(&links, 1).contains("names no type"), "{links:#}");
    assert!(reason(&links, 2).contains("work:nowhere"), "{links:#}");
    assert!(reason(&links, 3).contains("elsewhere"), "{links:#}");

    // Links a person spoiled in a task's front matter are refused by name, never read as links.
    std::fs::write(
        store.work.join("tasks").join("t4.md"),
        "---\ntitle: Another symptom\nstatus: todo\nmetadata:\n  \"onetaskgraph.links\": \
         [{\"type\": \"x\", \"direction\": \"in\", \"other\": \"t3\"}]\n---\nThe t4 body.\n",
    )
    .expect("a spoiled file");
    let spoiled = store.exited(&["task", "relations", "work:t4"], None, 1);
    let message = spoiled["failure"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("work:t4") && message.contains("\"t3\""),
        "{spoiled:#}"
    );
    let blocked = store.links(
        "remove-many",
        &json!([{"from": "work:t3", "to": "work:t4", "type": "x"}]),
        4,
    );
    assert_eq!(outcomes(&blocked), ["error"]);
    assert!(reason(&blocked, 0).contains("work:t4"), "{blocked:#}");
}

#[test]
fn an_in_memory_parents_map_naming_no_task_or_closing_a_cycle_is_refused_at_load() {
    let store = Store::new();
    let status = json!({"category": "todo", "name": "todo"});
    for (parents, named) in [
        (json!({"m1": "ghost"}), "ghost"),
        (json!({"m1": "m2", "m2": "m1"}), "cycle"),
    ] {
        let document = json!({"sources": {"memory": {"plugin": "in-memory", "config": {
            "tasks": [
                {"id": "m1", "title": "One", "labels": [], "status": status},
                {"id": "m2", "title": "Two", "labels": [], "status": status},
            ],
            "parents": parents,
        }}}});
        store
            .sandbox
            .project_document(&serde_json::to_string_pretty(&document).expect("JSON"));
        let output = store.run(&["task", "relations", "memory:m1"], None);
        assert_ne!(output.status.code(), Some(0));
        let said = format!("{}{}", stdout(&output), stderr(&output));
        assert!(said.contains("parents") && said.contains(named), "{said}");
    }
}

#[test]
fn a_render_of_a_linked_task_keeps_its_links_section() {
    let store = Store::new();
    let template = store.sandbox.project().join("task-template.md");
    std::fs::write(
        &template,
        "---\nonetaskgraph_template: 1\nvariables:\n  goal: {description: the goal}\n---\n\
         Aim: {{ goal }}\n",
    )
    .expect("a template");
    let template = template.to_str().expect("a path");
    std::fs::create_dir_all(store.work.join("projects")).expect("the project folder");
    std::fs::write(
        store.work.join("projects").join("P-1.md"),
        "---\ntitle: Plan\nstatus: todo\n---\n",
    )
    .expect("a project");
    let created = store.exited(
        &[
            "task",
            "create",
            "work",
            "--project",
            "P-1",
            "--title",
            "Rendered",
            "--template",
            template,
            "--var",
            "goal=first",
            "--no-interactive",
        ],
        None,
        0,
    );
    let id = created["items"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the create answers its id: {created:#}"))
        .to_owned();
    store.links(
        "add-many",
        &json!([{"from": id, "to": "work:t1", "type": "caused-by"}]),
        0,
    );
    let native = id.split_once(':').expect("qualified").1;
    let rendered = section(&store.file(&store.work, native)).to_owned();
    assert!(rendered.contains("[Root cause]("), "{rendered}");

    store.exited(
        &[
            "task",
            "render",
            &id,
            "--var",
            "goal=second",
            "--no-interactive",
        ],
        None,
        0,
    );
    let file = store.file(&store.work, native);
    assert!(file.contains("Aim: second"), "{file}");
    assert_eq!(section(&file), rendered);
    let shown = store.shown(&id);
    assert_eq!(shown["content"], json!("Aim: second\n"));
    // The rendering's digest is of the content a read answers, so the render is no hand edit.
    let digest = shown["metadata"]["onetaskgraph.template"]["body_digest"].clone();
    assert!(digest.is_string(), "{shown:#}");
    let again = store.exited(
        &[
            "task",
            "render",
            &id,
            "--var",
            "goal=second",
            "--no-interactive",
        ],
        None,
        0,
    );
    assert_eq!(store.file(&store.work, native), file, "{again:#}");
}

#[test]
fn a_parent_held_to_a_version_the_child_moved_past_answers_version_moved_writing_nothing() {
    let store = Store::new();
    let read = store.shown("work:t2")["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    // Another writer moves t2 after it was read.
    store.exited(
        &["task", "update", "work:t2", "--title", "Symptom, moved"],
        None,
        0,
    );
    let moved = store.file(&store.work, "t2");
    let now = store.shown("work:t2")["version"].clone();
    let answer = store.parents(
        &json!([
            {"child": "work:t2", "parent": "work:t1", "if_version": read},
            {"child": "work:t3", "parent": "work:t1", "if_version": store.shown("work:t3")["version"]},
        ]),
        4,
    );
    assert_eq!(outcomes(&answer), ["version-moved", "written"]);
    assert_eq!(answer["results"][0]["version"], now, "{answer:#}");
    assert_eq!(
        store.file(&store.work, "t2"),
        moved,
        "nothing was written to t2"
    );
    assert_eq!(store.relations(&["work:t2"])[0]["parent"], Value::Null);
}
