//! Replacement on every screened write, driven the way a user drives it: a store whose
//! `write_policy` names the released onevcs `boundary sanitize` command, against an onevcs home
//! this run owns holding the invented private identities of [`super::registered`].
//!
//! The term every journey here declares a replacement for is `quietharbor`, the name of the
//! invented private repository `hiddenco/quietharbor`: onevcs derives it, and rejects it unless a
//! replacement says otherwise. A store's own `replacements`, the environment and `--replace`
//! each supply one, and what lands in the public folder is held to carrying none of it.

use serde_json::{Value, json};

use super::{PLAIN, Store, files_under, registered, tree};
use crate::common::{stderr, stdout};
use crate::onevcs::{BoundaryCommand, BoundaryRecord, OnevcsHome};

/// The invented private term every journey here replaces.
const TERM: &str = "quietharbor";
/// What the store's own replacement rewrites it to.
const WITH: &str = "harbor-x";

/// A store of `plan` (private) and `site` (public) whose policy runs the released onevcs's four
/// boundary commands through a recorder, with `replacements` as its own replacement layer.
fn sanitizing(replacements: Value) -> (Store, BoundaryRecord) {
    let mut record = None;
    let store = Store::laid(Some(registered), |_| json!({}));
    let home = store.onevcs.as_ref().expect("an onevcs home");
    let recorded = home.recorded_sanitizing(&store.sandbox, replacements);
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"] = recorded.write_policy().clone();
    store.sandbox.project_document(&document.to_string());
    record.get_or_insert(recorded);
    store.public_project_p();
    (store, record.expect("a record"))
}

/// The store's own replacement of [`TERM`] by [`WITH`].
fn replaced() -> Value {
    json!([{ "term": TERM, "with": WITH }])
}

#[test]
fn a_priority_carrying_a_declared_term_is_refused_without_rewriting_it() {
    let (store, _) = sanitizing(json!([{ "term": "high", "with": "medium" }]));
    store.record(
        "site",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let (kind, _) = store.refused(&["task", "update", "site:clean", "--priority", "high"]);
    assert_eq!(kind, "boundary-refused");
    assert_eq!(tree(&store.folder("site")), before);
}

#[test]
fn stored_priorities_and_statuses_are_fixed_and_reported_as_their_own_fields() {
    for (field, value) in [("priority", "high"), ("status", "todo")] {
        let (store, _) = sanitizing(json!([{ "term": value, "with": "clean" }]));
        store.record(
            "site",
            "tasks",
            "clean",
            &format!(
                "title: Clean\nstatus: {}\npriority: {}",
                if field == "status" { value } else { "todo" },
                if field == "priority" { value } else { "low" }
            ),
            PLAIN,
        );
        let before = tree(&store.folder("site"));
        let (code, report, out, err) = sweep(&store, "site", &["--complete"]);
        assert_eq!(code, Some(1), "{out}{err}");
        let item = report["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["id"] == "site:clean")
            .expect("matched item");
        assert!(
            item["fields"]
                .as_array()
                .expect("fields")
                .iter()
                .any(|entry| entry["field"] == field && entry["outcome"] == "unfixable"),
            "{report}"
        );
        assert_eq!(tree(&store.folder("site")), before);
    }
}

#[test]
fn a_terms_failure_screens_the_source_name_even_when_no_item_was_read() {
    for missing in [false, true] {
        let (store, _) = sanitizing(replaced());
        let mut document: Value = serde_json::from_str(
            &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
                .expect("config"),
        )
        .expect("a document");
        document["sources"][TERM] = document["sources"]["site"].clone();
        if missing {
            document["write_policy"]
                .as_object_mut()
                .expect("policy")
                .remove("terms_command");
        } else {
            document["write_policy"]["terms_command"] = json!([
                crate::document_store::interpreter().to_string_lossy(),
                "-c",
                "import sys; sys.exit(2)"
            ]);
        }
        store.sandbox.project_document(&document.to_string());
        let (code, _, out, err) = sweep(&store, TERM, &[]);
        assert_eq!(code, Some(if missing { 1 } else { 2 }), "{out}{err}");
        assert!(err.contains("<redacted source 1>"), "{err}");
        spelled_nowhere(&store, &[&out, &err]);
    }
}

#[test]
fn the_editing_peers_vocabularies_and_task_fields_are_the_emitted_contract() {
    let shared = Shared::new(&[("t1", PLAIN)], json!({}), json!({}));
    let bundle: Value = serde_json::from_str(&shared.store.ok(&["schema"])).expect("schema");
    // Any change to a retained field's type, requiredness, nullability or referenced
    // definition requires reconciling the peer's validators and this fixture together.
    let frozen: serde_json::Map<String, Value> =
        serde_json::from_str(include_str!("../edit_source_schema.json"))
            .expect("the peer's schema fixture");
    let roots = bundle["roots"].as_object().expect("roots");
    for (name, expected) in frozen {
        let actual = roots
            .get(&name)
            .or_else(|| roots.values().find_map(|root| root["$defs"].get(&name)))
            .expect("a retained definition");
        assert_eq!(actual, &expected, "reconcile the peer's {name} validator");
    }
    let output = std::process::Command::new("python3")
        .args(["-c", "import json, runpy, sys, typing; p=runpy.run_path(sys.argv[1]); print(json.dumps({**{n:list(typing.get_args(p[n])) for n in ['StatusCategory','Priority','ItemKind','DependencyKind','TextFields','Direction','Classification']}, 'Task':sorted(p['Task'].__annotations__)}))"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/edit_source.py"))
        .output().expect("peer vocabulary");
    assert!(output.status.success(), "{}", stderr(&output));
    let declared: Value = serde_json::from_str(&stdout(&output)).expect("category values");
    for name in [
        "StatusCategory",
        "Priority",
        "ItemKind",
        "DependencyKind",
        "TextFields",
        "Classification",
    ] {
        let roots = bundle["roots"].as_object().expect("roots");
        let schema = roots
            .get(name)
            .or_else(|| roots.values().find_map(|root| root["$defs"].get(name)))
            .expect("a declared schema");
        let expected: Vec<Value> = schema["oneOf"]
            .as_array()
            .expect("alternatives")
            .iter()
            .map(|variant| variant["const"].clone())
            .collect();
        assert_eq!(declared[name], json!(expected), "{name}");
    }
    // Exhaustive matching makes a new API variant
    // fail compilation until this peer's accepted vocabulary is deliberately reconciled.
    use onetaskgraph_plugin_api::Direction;
    let direction_word = |direction| match direction {
        Direction::DependsOn => "depends-on",
        Direction::DependedOnBy => "depended-on-by",
    };
    assert_eq!(
        declared["Direction"],
        json!([
            direction_word(Direction::DependsOn),
            direction_word(Direction::DependedOnBy)
        ])
    );
    let schema = bundle["roots"]
        .as_object()
        .expect("roots")
        .values()
        .find_map(|root| root["$defs"].get("Task"))
        .expect("Task schema");
    let mut fields: Vec<_> = schema["properties"]
        .as_object()
        .expect("task properties")
        .keys()
        .collect();
    fields.sort();
    assert_eq!(declared["Task"], json!(fields));
}

#[test]
fn the_editing_peer_refuses_malformed_pages_targets_and_tasks_without_changing_the_record() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let shared = Shared::new(&[("t1", PLAIN)], json!({}), json!({}));
    let mut state = shared.state();
    state["later_comments_fail"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the peer's state");
    let before = shared.state()["tasks"].clone();
    let mut requests: Vec<Value> = [json!([1]), json!({"bad": 1}), json!(7)]
        .into_iter()
        .map(|target| {
            json!({"id":"bad-target", "method":"write_task", "params":{"write":{
                "target":target, "item":shared.task("t1"), "depends_on":[]
            }}})
        })
        .collect();
    for method in ["get_task", "set_task_priority"] {
        for id in [Value::Null, json!(7), json!([])] {
            requests.push(
                json!({"id":"bad-id", "method":method, "params":{"id":id,"priority":"high"}}),
            );
        }
    }
    requests.push(json!({"id":"bad-page", "method":"task_comments", "params":{
        "task":"t1", "page":null
    }}));
    for limit in [json!(0), json!(-1), json!(true)] {
        requests.push(
            json!({"id":"bad-limit", "method":"task_comments", "params":{
                "task":"t1", "page":{"cursor":null,"limit":limit}
            }}),
        );
    }
    requests.push(json!({"id":"bad-query", "method":"query_tasks", "params":{
        "query":{"text":{"terms":7,"fields":"title-or-content"}}
    }}));
    requests.push(json!({"id":"bad-fields", "method":"query_tasks", "params":{
        "query":{"text":{"terms":"plain","fields":"elsewhere"}}, "page":{"cursor":null,"limit":100}
    }}));
    requests.push(
        json!({"id":"bad-comment", "method":"edit_comment", "params":{
            "task":"t1", "comment":[], "body":"Plain."
        }}),
    );
    for body in [""] {
        requests.push(
            json!({"id":"empty-comment", "method":"edit_comment", "params":{
                "task":"t1", "comment":"c1", "body":body
            }}),
        );
    }
    for page in [
        Value::Null,
        json!({"cursor":"bad","limit":1}),
        json!({"cursor":null,"limit":0}),
    ] {
        requests.push(json!({"id":"bad-label-page","method":"labels","params":{"page":page}}));
    }
    requests.push(
        json!({"id":"bad-direction", "method":"task_dependencies", "params":{
            "id":"t1", "direction":"sideways", "page":{"cursor":null,"limit":100}
        }}),
    );
    requests.push(
        json!({"id":"bad-priority", "method":"set_task_priority", "params":{
            "id":"t1", "priority":"eventually"
        }}),
    );
    requests.push(
        json!({"id":"bad-edge", "method":"write_task", "params":{"write":{
            "target":"t1", "item":shared.task("t1"), "depends_on":[{"from":7}]
        }}}),
    );
    for designator in ["", "alias/short", "linear/not-a-digest"] {
        assert!(onetaskgraph_plugin_api::Designator::try_from(designator.to_owned()).is_err());
        requests.push(json!({"id":"bad-designator", "method":"write_task", "params":{"write":{
            "target":"t1", "item":shared.task("t1"), "depends_on":[{
                "from":"t1", "to":{"id":"t2","kind":"task","designator":designator}, "kind":"blocks"
            }]
        }}}));
    }
    let mut incomplete = shared.task("t1");
    incomplete.as_object_mut().expect("task").remove("content");
    requests.push(
        json!({"id":"bad-task", "method":"write_task", "params":{"write":{
            "target":"t1", "item":incomplete, "depends_on":[]
        }}}),
    );
    for (field, value) in [
        ("repositories", json!([7])),
        ("classification", json!("internal")),
        ("project", json!(7)),
        ("delivers", json!([7])),
        ("delivered_by", json!(false)),
        ("location", json!({"path":7})),
        ("location", json!({"path":"relative/record"})),
        ("created_at", json!("yesterday")),
        ("updated_at", json!(7)),
        ("key", json!(false)),
        ("url", json!([])),
    ] {
        let mut item = shared.task("t1");
        item[field] = value;
        requests.push(json!({"id":field,"method":"write_task","params":{"write":{
            "target":"t1", "item":item, "depends_on":[]
        }}}));
    }
    for method in ["health", "initialize"] {
        requests.push(json!({"id":"bad-params","method":method,"params":[1]}));
    }
    for request in requests {
        let mut child = Command::new(crate::document_store::interpreter())
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/edit_source.py"))
            .arg(&shared.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the editing peer");
        writeln!(child.stdin.take().expect("stdin"), "{request}").expect("a request");
        let output = child.wait_with_output().expect("a response");
        assert!(output.status.success(), "{}", stderr(&output));
        let answer: Value = serde_json::from_str(&stdout(&output)).expect("a response document");
        assert_eq!(answer["error"]["kind"], json!("malformed"), "{answer}");
        assert_eq!(shared.state()["tasks"], before);
    }
}

#[test]
fn the_editing_peer_refuses_malformed_stored_versions_and_controls() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let shared = Shared::new(&[("t1", PLAIN)], json!({}), json!({}));
    let base = shared.state();
    for (key, value) in [
        ("version", json!("²")),
        ("repositories", json!([7])),
        ("classification", json!("internal")),
        ("project", json!(7)),
        ("delivers", json!([7])),
        ("delivered_by", json!(false)),
        ("location", json!({"path":7})),
        ("location", json!({"path":"relative/record"})),
        ("created_at", json!("yesterday")),
        ("updated_at", json!(7)),
        ("gone", json!(7)),
        ("answer_ids", json!([])),
        ("log", json!([7])),
        ("unversioned", json!("yes")),
        ("reads", json!({"t1":true})),
        ("moves", json!({"t1":[0]})),
        (
            "comments",
            json!({"t1":[{"id":"c1","body":"Plain.","author":7}]}),
        ),
        (
            "comments",
            json!({"t1":[{"id":"c1","body":"Plain.","created_at":"yesterday"}]}),
        ),
        (
            "comments",
            json!({"t1":[{"id":"c1","body":"Plain.","updated_at":7}]}),
        ),
        (
            "comments",
            json!({"t1":[{"id":"c1","body":"Plain.","url":[]}]}),
        ),
        (
            "comments",
            json!({"t1":[{"id":"c1","body":"Plain.","origin":false}]}),
        ),
    ] {
        let mut state = base.clone();
        if [
            "version",
            "repositories",
            "classification",
            "project",
            "delivers",
            "delivered_by",
            "location",
            "created_at",
            "updated_at",
        ]
        .contains(&key)
        {
            state["tasks"][0][key] = value;
        } else {
            state[key] = value;
        }
        let before = state.to_string();
        std::fs::write(&shared.state, &before).expect("state");
        let mut child = Command::new(crate::document_store::interpreter())
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/edit_source.py"))
            .arg(&shared.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("peer");
        writeln!(
            child.stdin.take().expect("stdin"),
            "{}",
            json!({"id":"health","method":"health"})
        )
        .expect("request");
        let output = child.wait_with_output().expect("response");
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("not this peer's state"),
            "{}",
            stderr(&output)
        );
        assert_eq!(
            std::fs::read_to_string(&shared.state).expect("state"),
            before
        );
    }
}

#[test]
fn explicit_document_and_project_ids_are_fixed_entries_of_a_public_create() {
    let (store, record) = sanitizing(replaced());
    for kind in ["document", "project"] {
        let before = tree(&store.folder("site"));
        let body = store.body(PLAIN);
        let mut args = vec![
            kind,
            "create",
            "site",
            "--id",
            TERM,
            "--title",
            "Notes",
            "--body-file",
            &body,
        ];
        if kind == "document" {
            args.extend(["--project", "p"]);
        }
        let output = store.run(&args);
        assert!(!stdout(&output).contains(TERM), "{}", stdout(&output));
        assert!(!stderr(&output).contains(TERM), "{}", stderr(&output));
        let (failure, _) = store.refused(&args);
        assert_eq!(failure, "boundary-refused");
        assert_eq!(tree(&store.folder("site")), before);
        let calls = record.calls();
        let sent = &calls.last().expect("a screening call").input;
        assert!(
            sent["fixed"]["metadata"]
                .as_array()
                .expect("fixed metadata")
                .contains(&json!(TERM)),
            "{sent}"
        );
    }
}

#[test]
fn a_copy_whose_replacement_empties_a_carried_comment_writes_nothing() {
    let (store, _) = sanitizing(replaced());
    let python = crate::document_store::interpreter();
    repolicy(&store, |policy| {
        policy["sanitize_command"] = json!([python.to_string_lossy(), "-c", EMPTIES]);
    });
    store.record(
        "plan",
        "tasks",
        "carried",
        "title: Carried\nstatus: todo",
        &format!(
            "Plain body.\n\n## Comments\n\n<!-- onetaskgraph:comment \
          id=\"20260913T151107Z-1\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" \
          updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\n\
          {TERM}\n\n<!-- /onetaskgraph:comment -->"
        ),
    );
    let before = tree(&store.folder("site"));
    let output = store.run(&["task", "copy", "plan:carried", "--to", "site", "--comments"]);
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("leaves a comment it carries empty"),
        "{}",
        stderr(&output)
    );
    assert_eq!(tree(&store.folder("site")), before);
}

/// How many times the store ran `command` since `before` calls had been recorded.
fn ran(record: &BoundaryRecord, before: usize, command: BoundaryCommand) -> usize {
    record.calls()[before..]
        .iter()
        .filter(|call| call.command == command)
        .count()
}

/// Every file under `folder`, read as text, joined.
fn text_of(store: &Store, folder: &str) -> String {
    tree(&store.folder(folder))
        .into_values()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A template declaring a `goal` it renders and `notes` it keeps without rendering.
fn template(store: &Store) -> String {
    let path = store.sandbox.config_home().join("sanitize-template.md");
    std::fs::write(
        &path,
        "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is for\n  \
         notes:\n    description: Kept, never rendered\n    required: false\n---\n# {{ goal }}\n",
    )
    .expect("a template");
    path.display().to_string()
}

#[test]
fn every_rewritable_field_a_public_write_carries_lands_with_the_term_replaced_by_one_call() {
    let (store, record) = sanitizing(replaced());
    let body = store.body(&format!("Retry the way {TERM} does.\n"));

    // A create: body, title, label and a metadata value, rewritten by one sanitisation.
    let before = record.calls().len();
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        &format!("Port the {TERM} retry"),
        "--label",
        &format!("{TERM}-ops"),
        "--metadata",
        &format!("myapp.note=\"seen in {TERM}\""),
        "--body-file",
        &body,
        "--json",
    ]))
    .expect("the created task");
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    assert_eq!(ran(&record, before, BoundaryCommand::Check), 0);
    let sent = &record.calls()[record.calls().len() - 1].input;
    assert!(
        sent["replacements"] == replaced(),
        "the store's replacement layer is sent: {sent}"
    );
    assert_eq!(
        created["sanitization"]["replaced"],
        json!(4),
        "the create says how many spans were replaced: {created}"
    );
    assert_eq!(created["sanitization"]["side_effect"], json!([]));
    let task = &created["items"][0]["item"];
    assert_eq!(task["title"], json!(format!("Port the {WITH} retry")));
    assert_eq!(task["labels"][0]["name"], json!(format!("{WITH}-ops")));
    assert_eq!(
        task["metadata"]["myapp.note"],
        json!(format!("seen in {WITH}"))
    );
    assert!(
        task["content"]
            .as_str()
            .is_some_and(|content| content.contains(&format!("the way {WITH} does"))),
        "{task}"
    );
    let id = created["items"][0]["id"]
        .as_str()
        .expect("the created task's id")
        .to_owned();

    // An update and a comment: one sanitisation each, and what it answered is what is stored.
    let before = record.calls().len();
    store.ok(&["task", "update", &id, "--title", &format!("{TERM} again")]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    let comment = store.body(&format!("Asked the {TERM} team.\n"));
    let before = record.calls().len();
    store.ok(&["task", "comment", "add", &id, "--body-file", &comment]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    let answered = record.calls().last().expect("a call").clone();
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", &id, "--json"])).expect("a task");
    assert_eq!(
        shown["items"][0]["item"]["title"],
        json!(format!("{WITH} again"))
    );
    let comment_body = shown["comments"][0]["body"].as_str().expect("a comment");
    assert_eq!(comment_body, format!("Asked the {WITH} team.\n"));
    assert!(
        answered.input["text"]
            .as_array()
            .is_some_and(|text| text.contains(&json!(format!("Asked the {TERM} team.\n")))),
        "the comment's body was what the call was handed: {}",
        answered.input
    );

    let before = record.calls().len();
    store.ok(&[
        "document",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        &format!("{TERM} notes"),
        "--body-file",
        &store.body(PLAIN),
    ]);
    store.ok(&[
        "project",
        "create",
        "site",
        "--id",
        "q",
        "--title",
        &format!("The {TERM} port"),
        "--body-file",
        &store.body(PLAIN),
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 2);

    // A rendering's answers: the answer the template renders, and the one it only keeps.
    let template = template(&store);
    let before = record.calls().len();
    store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        &format!("goal=Match {TERM}"),
        "--var",
        &format!("notes=Background from {TERM}"),
        "--no-interactive",
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);

    // A copy out of the private folder: every field and the comment it carries, one call.
    store.record(
        "plan",
        "tasks",
        "carried",
        &format!(
            "title: Carry the {TERM} fix\nstatus: todo\nlabels: [{TERM}-carried]\nmetadata:\n  \
             why: from {TERM}"
        ),
        &format!(
            "Copied out of {TERM}.\n\n## Comments\n\n<!-- onetaskgraph:comment \
             id=\"20260913T151107Z-1\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" \
             updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\n\
             Ask {TERM} first.\n\n<!-- /onetaskgraph:comment -->"
        ),
    );
    let before = record.calls().len();
    store.ok(&["task", "copy", "plan:carried", "--to", "site", "--comments"]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);

    let public = text_of(&store, "site");
    assert!(
        !public.to_lowercase().contains(TERM),
        "no public file carries the term:\n{public}"
    );
    for written in [
        format!("{WITH} again"),
        format!("{WITH}-ops"),
        format!("seen in {WITH}"),
        format!("Asked the {WITH} team."),
        format!("{WITH} notes"),
        format!("The {WITH} port"),
        format!("Match {WITH}"),
        format!("Background from {WITH}"),
        format!("Carry the {WITH} fix"),
        format!("{WITH}-carried"),
        format!("Ask {WITH} first."),
    ] {
        assert!(
            public.contains(&written),
            "the public folder holds {written:?}:\n{public}"
        );
    }
}

#[test]
fn a_term_in_a_field_replacement_never_rewrites_is_refused_and_nothing_is_written() {
    let (store, record) = sanitizing(replaced());
    let before_tree = tree(&store.folder("site"));
    let create = |extra: &[&str]| -> Vec<String> {
        let mut arguments: Vec<String> = [
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "Plain",
            "--body-file",
        ]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
        arguments.push(store.body(PLAIN));
        arguments.extend(extra.iter().map(|word| (*word).to_owned()));
        arguments
    };
    let asset = store.sandbox.config_home().join(format!("{TERM}.png"));
    std::fs::write(&asset, onetaskgraph_e2e_support::images::png(7, 60_000)).expect("an image");
    let asset_body = store.body(&format!("![shot](./{TERM}.png)\n"));
    for (what, arguments) in [
        (
            "a metadata key",
            create(&["--metadata", &format!("{TERM}.key=\"x\"")]),
        ),
        (
            "a dependency id",
            create(&["--depends-on", &format!("site:{TERM}-dep")]),
        ),
        (
            "an asset name",
            vec![
                "task".to_owned(),
                "create".to_owned(),
                "site".to_owned(),
                "--project".to_owned(),
                "p".to_owned(),
                "--title".to_owned(),
                "Shot".to_owned(),
                "--body-file".to_owned(),
                asset_body.clone(),
                "--asset".to_owned(),
                asset.display().to_string(),
            ],
        ),
        (
            "a repository",
            create(&["--repository", "github.com/hiddenco/quietharbor"]),
        ),
    ] {
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let (kind, said) = store.refused(&arguments);
        assert!(
            [
                "boundary-refused",
                "not-private-destination",
                "private-member"
            ]
            .contains(&kind.as_str()),
            "{what}: {kind}: {said}"
        );
        assert!(
            !said.contains(TERM),
            "{what}: the refusal is neutral: {said}"
        );
        assert_eq!(
            tree(&store.folder("site")),
            before_tree,
            "{what}: nothing was written"
        );
    }
    // None of them was answered rewritten.
    for call in record.calls() {
        if call.command == BoundaryCommand::Sanitize {
            assert_ne!(
                call.exit,
                Some(0),
                "a refused write was not passed: {call:?}"
            );
        }
    }
}

#[test]
fn a_destination_declared_private_receives_the_original_text() {
    let (store, record) = sanitizing(replaced());
    store.record("plan", "projects", "p", "title: P\nstatus: todo", PLAIN);
    let before = record.calls().len();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "p",
        "--title",
        &format!("Keep {TERM}"),
        "--body-file",
        &store.body(&format!("Exactly as {TERM} says.\n")),
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 0);
    let private = text_of(&store, "plan");
    assert!(private.contains(&format!("Keep {TERM}")), "{private}");
    assert!(
        private.contains(&format!("Exactly as {TERM} says.")),
        "{private}"
    );
}

/// `quietharbor` as the store's own layer, the environment and `--replace` each set it.
#[test]
fn a_replacement_set_in_the_file_the_environment_or_the_flag_takes_effect_flag_over_environment_over_file()
 {
    let (store, _record) = sanitizing(json!([{ "term": TERM, "with": "file-word" }]));
    let title = |store: &Store, environment: &[(&str, &str)], flags: &[&str]| -> String {
        let mut command = store.sandbox.command();
        command.args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            &format!("From {TERM}"),
            "--body-file",
            &store.body(PLAIN),
            "--json",
        ]);
        command.args(flags);
        command.env("ONEVCS_HOME", store.onevcs.as_ref().expect("a home").path());
        for (name, value) in environment {
            command.env(name, value);
        }
        let output = command.assert().get_output().clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let created: Value = serde_json::from_str(&stdout(&output)).expect("a task");
        created["items"][0]["item"]["title"]
            .as_str()
            .expect("a title")
            .to_owned()
    };
    let environment = [
        ("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__TERM", TERM),
        (
            "ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__WITH",
            "env-word",
        ),
    ];
    assert_eq!(title(&store, &[], &[]), "From file-word");
    assert_eq!(title(&store, &environment, &[]), "From env-word");
    // `--set` addresses an entry by its index, as the environment does, and wins over it.
    assert_eq!(
        title(
            &store,
            &environment,
            &[
                "--set",
                &format!("write_policy.replacements.0.term={TERM}"),
                "--set",
                "write_policy.replacements.0.with=set-word",
            ]
        ),
        "From set-word"
    );
    assert_eq!(
        title(
            &store,
            &environment,
            &["--replace", &format!("{TERM}=flag-word")]
        ),
        "From flag-word"
    );
    // `config show` names the layer the replacement came from.
    let mut command = store.sandbox.command();
    command.args(["config", "show", "--json"]);
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.assert().get_output().clone();
    let shown: Value = serde_json::from_str(&stdout(&output)).expect("the configuration");
    let settings = shown["settings"].as_array().expect("settings");
    let layer_of = |key: &str| {
        settings
            .iter()
            .find(|setting| setting["key"] == json!(key))
            .map(|setting| setting["origin"].clone())
            .unwrap_or_else(|| panic!("config show reports {key}: {shown}"))
    };
    assert_eq!(
        layer_of("write_policy.replacements.0.with")["variable"],
        json!("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__WITH"),
        "{shown}"
    );
    // A `--reject` after the store's replacement refuses the term again.
    let mut command = store.sandbox.command();
    command
        .args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            &format!("From {TERM}"),
            "--body-file",
            &store.body(PLAIN),
            "--reject",
            TERM,
        ])
        .env("ONEVCS_HOME", store.onevcs.as_ref().expect("a home").path());
    let output = command.assert().get_output().clone();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
}

#[test]
fn a_caller_replacement_overrides_every_declaration_level_in_both_directions() {
    let (store, _record) = sanitizing(json!([]));
    let home: &OnevcsHome = store.onevcs.as_ref().expect("a home");
    // Three invented terms, one declared at each level onevcs reads.
    home.declare_repository_terms(
        "hiddenco",
        "quietharbor",
        "schema_version = 2\nterms = [\"emberline\"]\n[[replacements]]\nterm = \"emberline\"\n\
         with = \"repo-word\"\n",
    );
    home.declare_directory_terms(
        "schema_version = 2\nterms = [\"duskfield\"]\n[[replacements]]\nterm = \"duskfield\"\n\
         reject = true\n",
    );
    home.declare_fallback_terms(
        "schema_version = 2\nterms = [\"pinecrest\"]\n[[replacements]]\nterm = \"pinecrest\"\n\
         with = \"fallback-word\"\n",
    );
    let write = |flags: &[&str]| -> (Option<i32>, String) {
        let mut command = store.sandbox.command();
        command.args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "Levels",
            "--body-file",
            &store.body("emberline duskfield pinecrest quietharbor\n"),
            "--json",
            "--replace",
            "quietharbor=caller-q",
        ]);
        command.args(flags);
        command.env("ONEVCS_HOME", home.path());
        let output = command.assert().get_output().clone();
        let created: Value = serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null);
        (
            output.status.code(),
            created["items"][0]["item"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    };
    // The directory level rejects one, so without the caller nothing is written.
    assert_eq!(write(&[]).0, Some(1));
    // A caller replacement over that level's reject, and over the repository's own word.
    let (code, content) = write(&[
        "--replace",
        "duskfield=caller-d",
        "--replace",
        "emberline=caller-e",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(content, "caller-e caller-d fallback-word caller-q\n");
    // A caller reject over the fallback's replacement.
    let (code, _) = write(&["--replace", "duskfield=caller-d", "--reject", "pinecrest"]);
    assert_eq!(code, Some(1));
    assert!(
        !text_of(&store, "site").contains("pinecrest"),
        "the refused write left nothing"
    );
}

#[test]
fn replacements_without_a_sanitize_command_are_refused_at_load_naming_the_key() {
    let store = Store::new(Some(registered), json!({}));
    let home = store.onevcs.as_ref().expect("a home");
    let mut policy = home.write_policy();
    policy["replacements"] = replaced();
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"] = policy;
    store.sandbox.project_document(&document.to_string());
    let output = store.run(&["task", "list"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("write_policy.sanitize_command"),
        "{}",
        stderr(&output)
    );
    // And `--replace` on a store with no sanitize command at all.
    let plain = Store::new(Some(registered), json!({}));
    let output = plain.run(&["task", "list", "--replace", "a=b"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("write_policy.sanitize_command"));
    assert!(files_under(&plain, "site").is_empty());
}

/// A linking caller's policy written before this store could sanitise: it implements only the
/// two methods that existed then, over the released onevcs, and records every input it checks.
struct OldOnly {
    inner: super::LinkedOnevcs,
    checked: std::sync::Mutex<Vec<onetaskgraph_core::boundary::PublicWriteInput>>,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for OldOnly {
    async fn repository_visibility(
        &self,
        repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        self.inner.repository_visibility(repository).await
    }

    async fn check_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        self.checked.lock().expect("the record").push(input.clone());
        self.inner.check_public_write(input).await
    }
}

fn old_only(store: &Store) -> std::sync::Arc<OldOnly> {
    std::sync::Arc::new(OldOnly {
        inner: super::LinkedOnevcs {
            home: store
                .onevcs
                .as_ref()
                .expect("an onevcs home")
                .path()
                .to_path_buf(),
        },
        checked: std::sync::Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn an_old_only_linking_policy_answers_as_before_with_one_check_per_write_holding_every_locator()
 {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let policy = old_only(&store);
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    let created = engine
        .create_task(&super::task_on_site(PLAIN, &[]))
        .await
        .expect("a term-free create is written as before");
    let id = created.task.id.clone();
    assert_eq!(policy.checked.lock().expect("the record").len(), 1);
    engine
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect("a term-free narrow write is written as before");
    let checked = policy.checked.lock().expect("the record").clone();
    assert_eq!(checked.len(), 2, "one check per write: {checked:?}");
    // Every locator the write's diagnostics would print is in the one check's input.
    assert!(
        checked[0].metadata.contains(&"site".to_owned()),
        "{checked:?}"
    );
    assert!(
        checked[1].metadata.contains(&"site".to_owned())
            && checked[1].metadata.contains(&id.to_string()),
        "{checked:?}"
    );
    assert!(engine.take_sanitization().is_empty());
}

#[tokio::test]
async fn an_old_only_linking_policy_refuses_an_item_whose_id_carries_a_term_redacting_every_locator()
 {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "site",
        "tasks",
        &format!("{TERM}-note"),
        "title: Note\nstatus: todo\nproject: p",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let id: onetaskgraph_core::GlobalId = format!("site:{TERM}-note").parse().expect("an id");
    let engine = super::linked_engine(&store).with_write_policy(old_only(&store));
    let refused = engine
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect_err("the id carries a term");
    let said = refused.to_string();
    assert_eq!(super::kind_of(&refused), "boundary-refused", "{said}");
    assert!(
        said.contains("<redacted item 2>") && said.contains("<redacted source 1>"),
        "{said}"
    );
    assert!(!said.contains(TERM), "{said}");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");

    // A check that cannot be asked redacts every locator too.
    let unreachable =
        super::linked_engine(&store).with_write_policy(std::sync::Arc::new(super::Unreachable));
    let refused = unreachable
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect_err("an unreachable check never passes");
    let said = refused.to_string();
    assert_eq!(super::kind_of(&refused), "boundary-unavailable", "{said}");
    assert!(
        said.contains("<redacted item 2>") && !said.contains(TERM),
        "{said}"
    );
}

/// A linking caller's policy that sanitises through the released onevcs itself, counting calls.
struct Sanitizing {
    home: std::path::PathBuf,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Sanitizing {
    async fn repository_visibility(
        &self,
        _repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Ok(onetaskgraph_core::boundary::RepositoryVisibility::Unknown)
    }

    async fn check_public_write(
        &self,
        _input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        panic!("a sanitising policy is never asked to check");
    }

    async fn sanitize_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicSanitizeInput,
    ) -> Result<
        onetaskgraph_core::boundary::SanitizeVerdict,
        onetaskgraph_core::boundary::PolicyError,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let output = std::process::Command::new(crate::onevcs::onevcs())
            .args(["boundary", "sanitize", "--input", "-"])
            .env("ONEVCS_HOME", &self.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child
                    .stdin
                    .take()
                    .expect("stdin")
                    .write_all(&serde_json::to_vec(input).expect("serializes"))?;
                child.wait_with_output()
            })
            .expect("onevcs runs");
        Ok(onetaskgraph_core::boundary::sanitize_verdict(
            input,
            output.status.code(),
            &output.stdout,
        ))
    }
}

#[tokio::test]
async fn a_linking_caller_s_sanitising_policy_writes_what_its_one_call_answered() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(Sanitizing {
        home: store.onevcs.as_ref().expect("a home").path().to_path_buf(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let engine = super::linked_engine(&store)
        .with_write_policy(policy.clone())
        .with_replacements(vec![onetaskgraph_core::boundary::Replacement::Replace {
            term: TERM.to_owned(),
            with: WITH.to_owned(),
        }]);
    let mut task = super::task_on_site(&format!("Retry as {TERM} does."), &[]);
    task.title = format!("Port {TERM}");
    let created = engine.create_task(&task).await.expect("written replaced");
    assert_eq!(policy.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let shown = engine
        .task(&created.task.id)
        .await
        .expect("the task reads back");
    let item = &shown.items[0].item;
    assert_eq!(item.title, format!("Port {WITH}"));
    assert_eq!(
        item.content.as_deref(),
        Some(&*format!("Retry as {WITH} does."))
    );
    assert_eq!(engine.take_sanitization().replaced, 2);
    assert!(!text_of(&store, "site").contains(TERM));
}

/// One comment in a folder's own comments section, as an older store wrote it.
fn comment(id: &str, said: &str) -> String {
    format!(
        "<!-- onetaskgraph:comment id=\"{id}\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" \
         updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\n{said}\n\n\
         <!-- /onetaskgraph:comment -->"
    )
}

/// Stage the task `older` on `site` as a store before replacement could have written it: the
/// term in its body, title, a label, a metadata value and two comments.
fn stage_older(store: &Store) {
    store.record(
        "site",
        "tasks",
        "older",
        &format!(
            "title: Port the {TERM} retry\nstatus: todo\nproject: p\nlabels: [{TERM}-ops]\n\
             metadata:\n  myapp.note: seen in {TERM}"
        ),
        &format!(
            "Retry the way {TERM} does.\n\n## Comments\n\n{}\n\n{}",
            comment("20260913T151107Z-1", &format!("Asked {TERM} first.")),
            comment("20260913T151107Z-2", &format!("And {TERM} again."))
        ),
    );
}

const SIDE_EFFECTS: [&str; 6] = [
    "content",
    "title",
    "label",
    "metadata:myapp.note",
    "comment:20260913T151107Z-1",
    "comment:20260913T151107Z-2",
];

#[test]
fn a_narrow_write_to_an_item_already_carrying_a_term_sanitises_it_first_and_says_so() {
    let (store, record) = sanitizing(replaced());
    stage_older(&store);
    let before = record.calls().len();
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--json",
    ]))
    .expect("the answer");
    // One call screened the write with the task and its comments as they stood.
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    assert_eq!(answer["priority"], json!("high"), "{answer}");
    let fields: Vec<String> = answer["sanitization"]["side_effect"]
        .as_array()
        .expect("side effects")
        .iter()
        .map(|effect| {
            assert_eq!(effect["item"], json!("site:older"), "{answer}");
            effect["field"].as_str().expect("a field").to_owned()
        })
        .collect();
    let mut expected: Vec<String> = SIDE_EFFECTS
        .iter()
        .map(|field| (*field).to_owned())
        .collect();
    expected.sort();
    let mut fields_sorted = fields.clone();
    fields_sorted.sort();
    assert_eq!(fields_sorted, expected, "{answer}");
    assert_eq!(answer["sanitization"]["dry_run"], json!(false));
    let public = text_of(&store, "site");
    assert!(!public.contains(TERM), "{public}");
    for written in [
        format!("Retry the way {WITH} does."),
        format!("Port the {WITH} retry"),
        format!("{WITH}-ops"),
        format!("seen in {WITH}"),
        format!("Asked {WITH} first."),
        format!("And {WITH} again."),
    ] {
        assert!(public.contains(&written), "{written:?}:\n{public}");
    }
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", "site:older", "--json"])).expect("a task");
    assert_eq!(shown["items"][0]["item"]["priority"], json!("high"));
}

#[test]
fn a_sanitising_write_s_dry_run_writes_nothing_and_says_what_it_would_change() {
    let (store, _record) = sanitizing(replaced());
    stage_older(&store);
    let before = tree(&store.folder("site"));
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--json",
        "--sanitize-dry-run",
    ]))
    .expect("the answer");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
    assert!(answer.get("written").is_none(), "{answer}");
    assert_eq!(answer["would_pass"], json!(true), "{answer}");
    assert_eq!(answer["sanitization"]["dry_run"], json!(true), "{answer}");
    assert_eq!(
        answer["sanitization"]["side_effect"]
            .as_array()
            .expect("side effects")
            .len(),
        SIDE_EFFECTS.len(),
        "{answer}"
    );
    assert!(!answer.to_string().contains(TERM));
}

#[test]
fn a_sanitising_write_s_human_answer_is_today_s_line_then_one_sanitised_line_per_field() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo\nproject: p",
        PLAIN,
    );
    let today = store.ok(&["task", "priority", "set", "site:clean", "high"]);
    stage_older(&store);
    let said = store.ok(&["task", "priority", "set", "site:older", "high"]);
    let lines: Vec<&str> = said.trim_end().lines().collect();
    let success: Vec<String> = today
        .trim_end()
        .lines()
        .map(|line| line.replace("site:clean", "site:older"))
        .collect();
    assert_eq!(&lines[..success.len()], success.as_slice(), "{said}");
    let mut rest: Vec<&str> = lines[success.len()..].to_vec();
    rest.sort_unstable();
    let mut expected: Vec<String> = SIDE_EFFECTS
        .iter()
        .map(|field| format!("sanitised site:older {field}"))
        .collect();
    expected.sort();
    assert_eq!(rest, expected, "{said}");
    assert!(!said.contains(TERM));
}

#[test]
fn a_refused_write_reports_the_fields_it_sanitised_before_refusing() {
    for json_output in [true, false] {
        let (store, _) = sanitizing(replaced());
        stage_older(&store);
        let path = store.folder("site").join("tasks/older.md");
        let before = std::fs::read_to_string(&path).expect("task");
        std::fs::write(
            &path,
            before.replace(
                &format!("seen in {TERM}"),
                &format!("seen in {TERM} for hiddenco"),
            ),
        )
        .expect("older task");
        let mut args = vec!["task", "priority", "set", "site:older", "high"];
        if json_output {
            args.push("--json");
        }
        let output = store.run(&args);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        let out = stdout(&output);
        if json_output {
            let answer: Value = serde_json::from_str(&out).expect("failure");
            assert_eq!(answer["predates_write"], json!(true));
            assert_eq!(
                answer["sanitization"]["side_effect"]
                    .as_array()
                    .map(Vec::len),
                Some(SIDE_EFFECTS.len()),
                "{answer}"
            );
        } else {
            for field in SIDE_EFFECTS {
                assert!(
                    out.contains(&format!("sanitised site:older {field}")),
                    "{out}"
                );
            }
        }
        assert!(!out.contains(TERM) && !stderr(&output).contains(TERM));
        assert!(!out.contains("hiddenco") && !stderr(&output).contains("hiddenco"));
        let stored = std::fs::read_to_string(&path).expect("task");
        assert!(!stored.contains(TERM), "{stored}");
        assert!(stored.contains("hiddenco"));
        assert!(
            !stored.contains("priority: high"),
            "the requested write stayed refused"
        );
    }
}

#[test]
fn an_explicitly_private_item_with_no_repositories_keeps_the_classification_refusal() {
    let (store, _) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "private",
        "title: Private\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let output = store.run(&["task", "priority", "set", "site:private", "high", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("not-private-destination"));
    assert!(failure.get("predates_write").is_none(), "{failure}");
    assert!(!stderr(&output).contains("already named a private repository"));
    assert_eq!(tree(&store.folder("site")), before);
}

#[test]
fn an_item_whose_stored_repositories_already_name_a_private_repository_is_refused_saying_so() {
    let (store, record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "leaked",
        "title: Leaked\nstatus: todo\nproject: p\nrepositories: [github.com/hiddenco/quietharbor]",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let calls = record.calls().len();
    let output = store.run(&["task", "priority", "set", "site:leaked", "high", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("not-private-destination"));
    assert_eq!(failure["predates_write"], json!(true), "{failure}");
    let said = stderr(&output);
    for words in [
        "is private, and source site is not declared private",
        "already named a private repository before this write",
        "replacement cannot fix `repositories`",
        "remove that repository from the item's `repositories`",
        "move the item to a source declared private",
    ] {
        assert!(said.contains(words), "{words:?}: {said}");
    }
    assert!(!said.contains(TERM) && !said.contains("hiddenco"), "{said}");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
    // No sanitisation was attempted: the one call screened the refusal's locators alone.
    let after = &record.calls()[calls..];
    assert!(
        after
            .iter()
            .filter(|call| call.command == BoundaryCommand::Sanitize)
            .all(|call| call.input["text"].as_array().is_none_or(Vec::is_empty)),
        "{after:?}"
    );

    // A write that itself adds a private repository is refused as before, with no clause.
    store.record(
        "plan",
        "tasks",
        "adds",
        "title: Adds\nstatus: todo\nrepositories: [github.com/hiddenco/quietharbor]",
        PLAIN,
    );
    let output = store.run(&["task", "copy", "plan:adds", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("not-private-destination"));
    assert!(failure.get("predates_write").is_none(), "{failure}");
    assert!(!stderr(&output).contains("before this write"));
}

/// Stage on `folder` one task per hit class a whole read covers, as an older store wrote them,
/// beside one carrying no term at all.
fn stage_sweep(store: &Store, folder: &str) {
    store.record(
        folder,
        "tasks",
        "body",
        "title: Body\nstatus: todo\nproject: p",
        &format!("Retry the way {TERM} does."),
    );
    store.record(
        folder,
        "tasks",
        "said",
        "title: Said\nstatus: todo\nproject: p",
        &format!(
            "{PLAIN}\n\n## Comments\n\n{}",
            comment("20260913T151107Z-1", &format!("Asked {TERM} first."))
        ),
    );
    store.record(
        folder,
        "tasks",
        "origin",
        &format!(
            "title: Origin\nstatus: todo\nproject: p\nmetadata:\n  onetaskgraph.origin: plan:{TERM}-retry"
        ),
        PLAIN,
    );
    store.record(
        folder,
        "tasks",
        "clean",
        "title: Clean\nstatus: todo\nproject: p",
        PLAIN,
    );
}

/// Stage the two records whose own locators carry the term: an id, and a metadata key.
fn stage_locators(store: &Store, folder: &str) {
    store.record(
        folder,
        "tasks",
        &format!("{TERM}-named"),
        "title: Named\nstatus: todo\nproject: p",
        PLAIN,
    );
    store.record(
        folder,
        "tasks",
        "keyed",
        &format!("title: Keyed\nstatus: todo\nproject: p\nmetadata:\n  {TERM}.key: plain"),
        PLAIN,
    );
}

/// `sources sanitize` of `source` with `flags`, its exit code, parsed report, stdout and
/// stderr.
fn sweep(store: &Store, source: &str, flags: &[&str]) -> (Option<i32>, Value, String, String) {
    let mut arguments = vec!["sources", "sanitize", source, "--json"];
    arguments.extend_from_slice(flags);
    let output = store.run(&arguments);
    let out = stdout(&output);
    (
        output.status.code(),
        serde_json::from_str(&out).unwrap_or(Value::Null),
        out,
        stderr(&output),
    )
}

/// The entry of `report` for the item whose id is `id`.
fn entry<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == json!(id))
        .unwrap_or_else(|| panic!("an entry for {id}: {report}"))
}

/// Assert no spelling of the term — any case — is in `said`, nor in the store's cache.
fn spelled_nowhere(store: &Store, said: &[&str]) {
    for text in said {
        assert!(
            !text.to_lowercase().contains(TERM),
            "the term is printed: {text}"
        );
    }
    // Everything the store could have written beside the folders: what the test itself staged
    // — the folders, the bodies, the recorder's own log and the onevcs home — is left out.
    let root = store
        .sandbox
        .project()
        .parent()
        .expect("the sandbox")
        .to_path_buf();
    for (path, bytes) in tree(&root) {
        let staged = [
            "site",
            "named",
            "plan",
            "vault",
            "boundary-record",
            "onevcs-",
            "body-",
            "onetaskgraph.yaml",
            // The peer's own store: the source being swept, whose work the term is in.
            "edit-source.json",
        ]
        .iter()
        .any(|part| path.contains(part));
        assert!(
            staged
                || !String::from_utf8_lossy(&bytes)
                    .to_lowercase()
                    .contains(TERM),
            "a file the store wrote holds the term: {path}"
        );
    }
}

#[test]
fn a_sweep_of_a_folder_reads_it_whole_and_rewrites_every_hit_class_reporting_locators_alone() {
    let (store, _record) = sanitizing(replaced());
    stage_sweep(&store, "site");
    stage_locators(&store, "site");

    // A dry run first: every search and read, no write.
    let before = tree(&store.folder("site"));
    let (code, report, out, err) = sweep(&store, "site", &["--dry-run"]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert_eq!(
        tree(&store.folder("site")),
        before,
        "a dry run writes nothing"
    );
    assert_eq!(report["dry_run"], json!(true));
    for id in ["site:body", "site:said", "site:origin"] {
        assert_eq!(
            entry(&report, id)["outcome"],
            json!("would-rewrite"),
            "{report}"
        );
    }
    spelled_nowhere(&store, &[&out, &err]);

    let (code, report, out, err) = sweep(&store, "site", &[]);
    // Unfixable locators make it exit 1, though the folder was covered whole.
    assert_eq!(code, Some(1), "{out}{err}");
    assert_eq!(report["complete"], json!(true), "{report}");
    assert_eq!(
        report["coverage"],
        json!({"complete": true, "uncovered": []}),
        "{report}"
    );
    assert_eq!(report["source"], json!("site"));
    assert_eq!(
        report["requests"]["total"], 0,
        "a folder spends no source requests: {report}"
    );
    for operation in ["page_operations", "detail_operations", "rewrite_operations"] {
        assert!(
            report["requests"][operation]
                .as_u64()
                .is_some_and(|count| count > 0),
            "local work still counts as operations: {report}"
        );
    }
    let fields = |id: &str| -> Vec<(String, String)> {
        entry(&report, id)["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .map(|field| {
                (
                    field["field"].as_str().unwrap_or("<redacted>").to_owned(),
                    field["outcome"].as_str().expect("an outcome").to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(entry(&report, "site:body")["outcome"], json!("rewritten"));
    assert_eq!(
        fields("site:body"),
        [("content".to_owned(), "rewritten".to_owned())]
    );
    assert_eq!(
        fields("site:said"),
        [(
            "comment:20260913T151107Z-1".to_owned(),
            "rewritten".to_owned()
        )]
    );
    // A term only in the store's own link value is no locator: the value is rewritten.
    assert_eq!(
        fields("site:origin"),
        [(
            "metadata:onetaskgraph.origin".to_owned(),
            "rewritten".to_owned()
        )]
    );
    // An item read whole that carries no term has no entry.
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["id"] != json!("site:clean")),
        "{report}"
    );
    // The two records whose own locators carry the term: unfixable, redacted, numbered from 1
    // in the order the report emits them.
    let redacted: Vec<&Value> = report["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["outcome"] == json!("unfixable"))
        .collect();
    assert_eq!(redacted.len(), 2, "{report}");
    let mut ordinals = Vec::new();
    for item in &redacted {
        assert_eq!(
            item["fields"][0]["why"],
            json!("identifier carries a private term")
        );
        for locator in [&item["id"], &item["fields"][0]["field"]] {
            if locator["redacted"] == json!(true) {
                ordinals.push(locator["ordinal"].as_u64().expect("an ordinal"));
            }
        }
    }
    let named = redacted
        .iter()
        .find(|item| item["id"]["redacted"] == json!(true))
        .expect("the item whose id carries the term");
    assert_eq!(named["id"]["kind"], json!("item"));
    assert_eq!(
        ordinals.first(),
        Some(&1),
        "ordinals count from 1: {report}"
    );
    assert!(
        ordinals.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{ordinals:?}"
    );
    spelled_nowhere(&store, &[&out, &err]);
    let public = text_of(&store, "site");
    assert!(
        public.contains(&format!("Retry the way {WITH} does.")),
        "{public}"
    );
    assert!(public.contains(&format!("Asked {WITH} first.")), "{public}");
    assert!(public.contains(&format!("plan:{WITH}-retry")), "{public}");

    // The human rendering names the same records, by the same screened locators.
    let human = store.run(&["sources", "sanitize", "site"]);
    let said = stdout(&human);
    assert_eq!(human.status.code(), Some(1), "{said}");
    assert!(
        said.contains("<redacted metadata-key 1>") && said.contains("<redacted item 2>"),
        "{said}"
    );
    spelled_nowhere(&store, &[&said, &stderr(&human)]);

    // A rerun writes nothing to what is already sanitised: only the unfixable records remain.
    let before = tree(&store.folder("site"));
    let (code, report, _, _) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(1));
    assert_eq!(tree(&store.folder("site")), before, "{report}");
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["outcome"] == json!("unfixable")),
        "{report}"
    );
}

#[test]
fn a_sweep_of_a_clean_folder_exits_zero_with_complete_coverage() {
    let (store, _record) = sanitizing(replaced());
    stage_sweep(&store, "site");
    let (code, report, out, err) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(report["coverage"]["complete"], json!(true));
    assert_eq!(
        report["items"].as_array().map(Vec::len),
        Some(3),
        "{report}"
    );
    let (code, report, _, _) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(report["items"], json!([]), "{report}");
}

#[test]
fn a_sweep_of_a_source_whose_name_carries_a_term_redacts_it_and_an_undecided_policy_exits_two() {
    let (store, _record) = sanitizing(replaced());
    let named = format!("{TERM}-site");
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["sources"][&named] = json!({
        "plugin": "local-md",
        "config": {"root": store.sandbox.subdirectory("named")},
        "visibility": "public",
    });
    store.sandbox.project_document(&document.to_string());
    store.record(
        "named",
        "tasks",
        "body",
        "title: Body\nstatus: todo",
        &format!("{TERM} here"),
    );
    let (code, report, out, err) = sweep(&store, &named, &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert_eq!(
        report["source"],
        json!({"redacted": true, "kind": "source", "ordinal": 1}),
        "{report}"
    );
    spelled_nowhere(&store, &[&out, &err]);

    // A policy that cannot decide redacts every locator and exits 2.
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"]["sanitize_command"] = json!([
        "python3",
        "-c",
        "import sys; sys.stdin.read(); print('{\"verdict\": \"unavailable\", \"reason\": \"registry\"}'); sys.exit(2)"
    ]);
    store.sandbox.project_document(&document.to_string());
    stage_sweep(&store, "site");
    let (code, _report, out, err) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(2), "{out}{err}");
    assert!(err.contains("<redacted item"), "{err}");
    assert!(
        !err.contains("site:body") && !out.contains("site:body"),
        "{out}{err}"
    );
    spelled_nowhere(&store, &[&out, &err]);
}

#[test]
fn a_sweep_without_a_terms_command_is_refused_naming_it() {
    let (store, _record) = sanitizing(replaced());
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"]
        .as_object_mut()
        .expect("a policy")
        .remove("terms_command");
    store.sandbox.project_document(&document.to_string());
    let (code, _, out, err) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(err.contains("terms_command"), "{err}");
}

#[test]
fn replacement_flags_and_settings_are_read_in_order_and_refused_by_name_when_malformed() {
    let (store, record) = sanitizing(json!([]));
    let home = store.onevcs.as_ref().expect("a home").path().to_path_buf();
    let create = |flags: &[&str], environment: &[(&str, &str)]| {
        let mut command = store.sandbox.command();
        command
            .args([
                "task",
                "create",
                "site",
                "--project",
                "p",
                "--title",
                "Flags",
                "--body-file",
                &store.body(PLAIN),
            ])
            .args(flags)
            .env("ONEVCS_HOME", &home);
        for (name, value) in environment {
            command.env(name, value);
        }
        command.assert().get_output().clone()
    };
    // A `--replace` with no `=`, or naming no term, is a usage error naming the flag.
    for flags in [
        &["--replace", "emberline"][..],
        &["--replace", "=x"],
        &["--reject", ""],
    ] {
        let output = create(flags, &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{flags:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("--replace") || stderr(&output).contains("--reject"),
            "{}",
            stderr(&output)
        );
    }
    // Interleaved, the entries reach the policy in the order typed, split at the first `=`.
    let before = record.calls().len();
    let output = create(
        &[
            "--reject",
            "emberline",
            "--replace",
            "duskfield=dusk=x",
            "--replace",
            "emberline=ember-x",
        ],
        &[],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let calls = record.calls();
    let sent = &calls[before..]
        .iter()
        .find(|call| call.command == BoundaryCommand::Sanitize)
        .expect("one sanitisation")
        .input;
    assert_eq!(
        sent["replacements"],
        json!([
            {"term": "emberline", "reject": true},
            {"term": "duskfield", "with": "dusk=x"},
            {"term": "emberline", "with": "ember-x"},
        ]),
        "{sent}"
    );
    // An environment entry that skips an index is refused at load, naming the gap.
    let output = create(
        &[],
        &[
            (
                "ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__1__TERM",
                "emberline",
            ),
            ("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__1__REJECT", "true"),
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("write_policy") && stderr(&output).contains("entry 0"),
        "{}",
        stderr(&output)
    );
}

/// A store with the public source `shared` beside the folders: `edit_source.py`, whose fixture
/// lands another writer's `edits` at the read counts `moves` names, per task.
struct Shared {
    store: Store,
    state: std::path::PathBuf,
}

impl Shared {
    fn new(tasks: &[(&str, &str)], moves: Value, edits: Value) -> Self {
        Self::named("shared", tasks, moves, edits)
    }

    /// [`Shared::new`], the peer configured under `name`.
    fn named(name: &str, tasks: &[(&str, &str)], moves: Value, edits: Value) -> Self {
        let (store, _record) = sanitizing(replaced());
        let state = store.sandbox.config_home().join("edit-source.json");
        let held: Vec<Value> = tasks
            .iter()
            .map(|(id, content)| {
                json!({"id": id, "title": format!("{id} task"), "content": content,
                       "status": {"category": "todo", "name": "Todo"}, "priority": "none",
                       "labels": [], "project": null, "url": null, "location": null,
                       "created_at": null, "updated_at": null, "metadata": {}, "version": "1"})
            })
            .collect();
        let index: serde_json::Map<String, Value> = tasks
            .iter()
            .map(|(id, content)| ((*id).to_owned(), json!(format!("{id} task {content}"))))
            .collect();
        std::fs::write(
            &state,
            json!({"tasks": held, "comments": {}, "reads": {}, "moves": moves, "edits": edits,
                   "index": index, "edges": {}, "log": []})
            .to_string(),
        )
        .expect("the peer's state");
        let mut document: Value = serde_json::from_str(
            &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
                .expect("the project document"),
        )
        .expect("JSON");
        document["sources"][name] = json!({
            "plugin": "subprocess",
            "visibility": "public",
            "config": {
                "command": crate::document_store::interpreter().to_string_lossy(),
                "args": [
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/e2e/edit_source.py")
                        .to_string_lossy(),
                    state.to_string_lossy(),
                ],
            },
        });
        store.sandbox.project_document(&document.to_string());
        Self { store, state }
    }

    fn state(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.state).expect("the state"))
            .expect("JSON")
    }

    fn task(&self, id: &str) -> Value {
        self.state()["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|task| task["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} is held"))
    }

    /// Lay `fields` over the held task `id`, as another store could have written it.
    fn patch(&self, id: &str, fields: Value) {
        let mut state = self.state();
        for task in state["tasks"].as_array_mut().expect("tasks") {
            if task["id"] == json!(id) {
                for (key, value) in fields.as_object().expect("fields") {
                    task[key] = value.clone();
                }
            }
        }
        std::fs::write(&self.state, state.to_string()).expect("the state");
    }

    /// Put a comment `comment` saying `body` on the held task `task`.
    fn comment(&self, task: &str, comment: &str, body: &str) {
        let mut state = self.state();
        state["comments"][task] = json!([{"id": comment, "author": "ada", "created_at": null,
            "updated_at": null, "body": body, "url": null, "origin": null}]);
        std::fs::write(&self.state, state.to_string()).expect("the state");
    }

    fn writes(&self) -> usize {
        self.state()["log"]
            .as_array()
            .expect("a log")
            .iter()
            .filter(|method| *method == "write_task")
            .count()
    }
}

/// The body every interleaving journey's task starts with: the term in its first paragraph.
fn with_term() -> String {
    format!("Retry the way {TERM} does.")
}

fn labels(task: &Value) -> Vec<String> {
    task["labels"]
        .as_array()
        .expect("labels")
        .iter()
        .map(|label| label["name"].as_str().expect("a name").to_owned())
        .collect()
}

#[test]
fn a_record_moved_once_before_the_sanitising_write_is_sanitised_from_its_fresh_read() {
    for edit in [
        json!({"label": "person-label"}),
        json!({"paragraph": "A person's paragraph."}),
    ] {
        let shared = Shared::new(
            &[("t1", &with_term())],
            json!({"t1": [2]}),
            json!({ "t1": edit.clone() }),
        );
        let answer: Value = serde_json::from_str(&shared.store.ok(&[
            "task",
            "priority",
            "set",
            "shared:t1",
            "high",
            "--json",
        ]))
        .expect("the answer");
        let task = shared.task("t1");
        assert_eq!(task["priority"], json!("high"), "{edit}: {answer}");
        let content = task["content"].as_str().expect("content");
        assert!(
            content.contains(&format!("the way {WITH} does")),
            "{edit}: {content}"
        );
        assert!(!content.contains(TERM), "{edit}: {content}");
        if edit.get("label").is_some() {
            assert_eq!(
                labels(&task),
                ["person-label"],
                "{edit}: the person's label is kept"
            );
        } else {
            assert!(
                content.contains("A person's paragraph."),
                "{edit}: {content}"
            );
        }
        assert_eq!(
            answer["sanitization"]["side_effect"][0]["field"],
            json!("content")
        );
    }
}

#[test]
fn a_record_moved_twice_is_a_conflict_and_the_other_writer_s_version_stands() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2, 3]}),
        json!({"t1": {"label": "person-label"}}),
    );
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "sanitize-conflict", "{said}");
    assert!(
        said.contains("changed twice") && !said.contains(TERM),
        "{said}"
    );
    let task = shared.task("t1");
    assert_eq!(shared.writes(), 0, "nothing was overwritten");
    assert_eq!(
        task["version"],
        json!("3"),
        "both of the other writer's edits stand"
    );
    assert_eq!(task["content"], json!(with_term()));
    assert_eq!(
        task["priority"],
        json!("none"),
        "the write it was made for was not made"
    );
}

#[test]
fn an_edit_between_the_sanitising_write_and_the_write_it_was_for_is_kept_or_refused() {
    // Read 3 is the read back after the sanitising write: the edit lands there once — kept,
    // and the priority written over it — or again at read 4, a conflict, the priority unmade.
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [3]}),
        json!({"t1": {"paragraph": "A person's paragraph."}}),
    );
    shared
        .store
        .ok(&["task", "priority", "set", "shared:t1", "high"]);
    let task = shared.task("t1");
    assert_eq!(task["priority"], json!("high"));
    let content = task["content"].as_str().expect("content");
    assert!(
        content.contains(&format!("the way {WITH} does"))
            && content.contains("A person's paragraph."),
        "{content}"
    );

    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [3, 4]}),
        json!({"t1": {"paragraph": "A person's paragraph."}}),
    );
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "sanitize-conflict", "{said}");
    let task = shared.task("t1");
    assert_eq!(
        task["priority"],
        json!("none"),
        "the write it was for was not made"
    );
    assert!(
        task["content"]
            .as_str()
            .is_some_and(|content| content.ends_with("A person's paragraph.")),
        "the person's edits are not overwritten: {task}"
    );
}

#[test]
fn a_sweep_keeps_a_move_rewrites_after_it_reports_a_conflict_and_a_rerun_fixes_only_that() {
    let shared = Shared::new(
        &[
            ("t1", &with_term()),
            ("t2", &with_term()),
            ("t3", "Nothing private."),
        ],
        json!({"t1": [2], "t2": [2, 3]}),
        json!({"t1": {"label": "person-label"}, "t2": {"label": "person-label"}}),
    );
    let (code, report, out, err) = sweep(&shared.store, "shared", &[]);
    assert_eq!(code, Some(1), "a conflict is left: {out}{err}");
    assert_eq!(
        entry(&report, "shared:t1")["outcome"],
        json!("rewritten"),
        "{report}"
    );
    assert_eq!(
        entry(&report, "shared:t2")["outcome"],
        json!("conflict"),
        "{report}"
    );
    let first = shared.task("t1");
    assert_eq!(
        labels(&first),
        ["person-label"],
        "the other writer's change is kept"
    );
    assert!(!first["content"].as_str().expect("content").contains(TERM));
    let conflicted = shared.task("t2");
    assert_eq!(
        conflicted["content"],
        json!(with_term()),
        "nothing overwritten"
    );
    assert_eq!(shared.writes(), 1);

    // The rerun: the index still returns both hits; the one already sanitised is unchanged
    // and written nothing, and the conflicted one is rewritten keeping both edits.
    let (code, report, out, err) = sweep(&shared.store, "shared", &[]);
    assert_eq!(
        code,
        Some(3),
        "only coverage is left incomplete: {out}{err}"
    );
    assert_eq!(
        entry(&report, "shared:t1")["outcome"],
        json!("unchanged"),
        "{report}"
    );
    assert_eq!(
        entry(&report, "shared:t2")["outcome"],
        json!("rewritten"),
        "{report}"
    );
    assert_eq!(
        shared.writes(),
        2,
        "one write, to the conflicted item alone"
    );
    let fixed = shared.task("t2");
    assert!(!fixed["content"].as_str().expect("content").contains(TERM));
    assert_eq!(labels(&fixed), ["person-label", "person-label"]);
}

#[test]
fn a_recopy_of_an_item_whose_id_carries_a_replaced_term_updates_the_item_it_copied_before() {
    let (store, _record) = sanitizing(replaced());
    // A second public folder the copy lands in, beside `site`, which it is copied out of.
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["sources"]["mirror"] = json!({
        "plugin": "local-md",
        "config": {"root": store.sandbox.subdirectory("mirror")},
        "visibility": "public",
    });
    store.sandbox.project_document(&document.to_string());
    let source_id = format!("{TERM}-retry");
    let source = format!("site:{source_id}");
    let lay = |title: &str| {
        let path = store
            .folder("site")
            .join("tasks")
            .join(format!("{source_id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(
            &path,
            format!("---\ntitle: {title}\nstatus: todo\n---\n{PLAIN}\n"),
        )
        .expect("the source task");
    };
    lay("Retry");
    let first: Value =
        serde_json::from_str(&store.ok(&["task", "copy", &source, "--to", "mirror", "--json"]))
            .expect("the copy");
    let landed = first["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("a destination: {first}"))
        .to_owned();
    assert!(
        !landed.contains(TERM),
        "the id it landed under is as passed: {landed}"
    );
    let mirrored = text_of(&store, "mirror");
    assert!(!mirrored.contains(TERM), "{mirrored}");
    assert!(
        files_under(&store, "mirror")
            .iter()
            .all(|path| !path.contains(TERM)),
        "no public file is named after the term"
    );
    // The origin it records names the source item in its replaced form.
    assert!(
        mirrored.contains(&format!("site:{WITH}-retry")),
        "{mirrored}"
    );

    // A second copy after an edit at the source reaches that same item.
    lay("Retry again");
    let before = files_under(&store, "mirror");
    let second: Value =
        serde_json::from_str(&store.ok(&["task", "copy", &source, "--to", "mirror", "--json"]))
            .expect("the copy");
    assert_eq!(second["items"][0]["destination"], json!(landed), "{second}");
    assert_eq!(files_under(&store, "mirror"), before, "no second item");
    assert!(text_of(&store, "mirror").contains("Retry again"));
}

#[test]
fn a_sweep_reports_a_term_carrying_identifier_redacted_and_numbered_beside_records_that_pass() {
    let named = format!("{TERM}-x");
    let shared = Shared::new(
        &[
            ("PVTI_x", &with_term()),
            ("t2", &with_term()),
            (&named, "Plain."),
            ("t3", &with_term()),
        ],
        // Each dry run reads t2 once; the sweep that writes reads it at 3, again before its
        // write at 4 — moved — and once more at 5 — moved again: a conflict.
        json!({"t2": [4, 5]}),
        json!({"t2": {"label": "person-label"}}),
    );
    shared.comment("t3", &format!("{TERM}-c1"), "Plain.");
    let redacted_in = |report: &Value| -> Vec<(String, u64)> {
        let mut found = Vec::new();
        for item in report["items"].as_array().expect("items") {
            for locator in std::iter::once(&item["id"]).chain(
                item["fields"]
                    .as_array()
                    .expect("fields")
                    .iter()
                    .map(|field| &field["field"]),
            ) {
                if locator["redacted"] == json!(true) {
                    found.push((
                        locator["kind"].as_str().expect("a kind").to_owned(),
                        locator["ordinal"].as_u64().expect("an ordinal"),
                    ));
                }
            }
        }
        found
    };
    for flags in [&["--dry-run"][..], &["--complete", "--dry-run"], &[]] {
        let (code, report, out, err) = sweep(&shared.store, "shared", flags);
        assert_eq!(code, Some(1), "{flags:?}: {out}{err}");
        assert_eq!(
            redacted_in(&report),
            [("item".to_owned(), 1), ("comment".to_owned(), 2)],
            "{flags:?}: ordinals count from 1 in the order the report emits them: {report}"
        );
        // A passing locator prints as it is.
        let passing = entry(&report, "shared:PVTI_x");
        assert!(
            ["would-rewrite", "rewritten"]
                .contains(&passing["outcome"].as_str().expect("an outcome")),
            "{flags:?}: {report}"
        );
        for item in report["items"].as_array().expect("items") {
            if item["id"]["redacted"] == json!(true)
                || item["fields"][0]["field"]["redacted"] == json!(true)
            {
                assert_eq!(item["outcome"], json!("unfixable"), "{report}");
                assert_eq!(
                    item["fields"][0]["why"],
                    json!("identifier carries a private term")
                );
            }
        }
        if flags.is_empty() {
            assert_eq!(
                entry(&report, "shared:t2")["outcome"],
                json!("conflict"),
                "{report}"
            );
        }
        spelled_nowhere(&shared.store, &[&out, &err]);
    }
    // The human rendering prints the same records by the same screened locators.
    let human = shared
        .store
        .run(&["sources", "sanitize", "shared", "--dry-run"]);
    let said = stdout(&human);
    assert!(
        said.contains("<redacted item 1>") && said.contains("<redacted comment 2>"),
        "{said}"
    );
    spelled_nowhere(&shared.store, &[&said, &stderr(&human)]);
}

#[test]
fn both_predates_refusals_name_a_source_whose_name_carries_a_term_only_redacted() {
    let source = format!("{TERM}-shared");
    let shared = Shared::named(
        &source,
        &[
            ("t1", "Plain."),
            ("t2", "Mirror hiddenco/quietharbor here."),
        ],
        json!({}),
        json!({}),
    );
    shared.patch(
        "t1",
        json!({"repositories": ["github.com/hiddenco/quietharbor"]}),
    );
    for (task, kind) in [
        ("t1", "not-private-destination"),
        ("t2", "boundary-refused"),
    ] {
        let before = shared.state()["tasks"].clone();
        let output = shared.store.run(&[
            "task",
            "priority",
            "set",
            &format!("{source}:{task}"),
            "high",
            "--json",
        ]);
        assert_eq!(output.status.code(), Some(1), "{task}: {}", stderr(&output));
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        assert_eq!(failure["failure"]["kind"], json!(kind), "{task}: {failure}");
        assert_eq!(failure["predates_write"], json!(true), "{task}: {failure}");
        let said = stderr(&output);
        assert!(said.contains("<redacted source 1>"), "{task}: {said}");
        assert!(said.contains("before this write"), "{task}: {said}");
        spelled_nowhere(&shared.store, &[&said, &stdout(&output)]);
        assert_eq!(
            shared.state()["tasks"],
            before,
            "{task}: nothing was written"
        );
    }
}

/// A sanitise command answering one malformed way, named by its argument.
const MALFORMED: &str = r#"
import json, sys
sent = json.loads(sys.stdin.read())
answer = {"verdict": "pass", "text": sent.get("text", []), "paths": sent.get("paths", []),
          "metadata": sent.get("metadata", []), "replaced": 0, "unfixed": [],
          "locators": [locator["value"] for locator in sent.get("locators", [])]}
how = sys.argv[1]
if how == "lengths":
    answer["text"] = []
elif how == "locators":
    answer["locators"] = ["another-source" for _ in sent.get("locators", [])]
elif how == "passed-unfixed":
    answer["unfixed"] = [{"surface": "text", "index": 0, "fixed": False}]
elif how == "passed-redacted":
    answer["locators"] = [{"redacted": True, "kind": locator["kind"], "ordinal": index + 1}
                          for index, locator in enumerate(sent.get("locators", []))]
elif how == "two-ways":
    seen = set()
    for index, value in enumerate(answer["metadata"]):
        if value in seen:
            answer["metadata"][index] = value + " again"
        seen.add(value)
elif how == "wrong-kind":
    answer.update(verdict="refuse", surface="locators",
                  unfixed=[{"surface": "locators", "index": 0, "fixed": True}])
    answer["locators"][0] = {"redacted": True, "kind": "comment", "ordinal": 1}
elif how == "unfixed-plain":
    answer.update(verdict="refuse", surface="locators",
                  unfixed=[{"surface": "locators", "index": 0, "fixed": True}])
elif how == "zero-ordinal":
    answer.update(verdict="refuse", surface="locators",
                  unfixed=[{"surface": "locators", "index": 0, "fixed": True}])
    answer["locators"][0] = {"redacted": True, "kind": sent["locators"][0]["kind"], "ordinal": 0}
elif how == "path-conflict":
    answer["paths"] = [value + "-other" for value in answer["paths"]]
elif how == "locator-rewritable":
    answer.update(verdict="refuse", surface="locators",
                  unfixed=[{"surface": "locators", "index": 0, "fixed": False}])
elif how == "pass-exit-one":
    print(json.dumps(answer))
    sys.exit(1)
elif how == "refuse-exit-zero":
    answer.update(verdict="refuse", surface="text",
                  unfixed=[{"surface": "text", "index": 0, "fixed": False}])
    print(json.dumps(answer))
    sys.exit(0)
elif how == "killed":
    import os, signal
    os.kill(os.getpid(), signal.SIGKILL)
elif how == "unsent-index":
    answer.update(verdict="refuse", surface="text",
                  unfixed=[{"surface": "text", "index": 99, "fixed": False}])
print(json.dumps(answer))
sys.exit(1 if answer["verdict"] == "refuse" else 0)
"#;

/// Rewrite the store's `write_policy` with `change`.
fn repolicy(store: &Store, change: impl FnOnce(&mut Value)) {
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    change(&mut document["write_policy"]);
    store.sandbox.project_document(&document.to_string());
}

#[test]
fn a_policy_answer_that_does_not_match_what_was_sent_is_never_a_pass() {
    let (store, _record) = sanitizing(replaced());
    let python = crate::document_store::interpreter();
    for how in [
        "lengths",
        "locators",
        "passed-unfixed",
        "passed-redacted",
        "two-ways",
        "wrong-kind",
        "unfixed-plain",
        "zero-ordinal",
        "path-conflict",
        "locator-rewritable",
        "pass-exit-one",
        "refuse-exit-zero",
        "killed",
        "unsent-index",
    ] {
        repolicy(&store, |policy| {
            policy["sanitize_command"] = json!([python.to_string_lossy(), "-c", MALFORMED, how]);
        });
        let before = tree(&store.folder("site"));
        let (kind, said) = store.refused(&[
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "Plain",
            "--body-file",
            &store.body(PLAIN),
            // The same short value twice — a title and a label — which "two-ways" answers apart.
            "--label",
            "Plain",
            // A value that reads as a path, sent on both surfaces, which "path-conflict" answers
            // apart.
            "--metadata",
            "myapp.doc=\"docs/plain.md\"",
        ]);
        assert_eq!(kind, "boundary-unavailable", "{how}: {said}");
        assert_eq!(
            tree(&store.folder("site")),
            before,
            "{how}: nothing was written"
        );
    }
}

#[test]
fn a_term_replacement_cannot_fix_is_refused_after_what_it_can_fix_is_fixed_and_a_dry_run_writes_nothing()
 {
    let (store, _record) = sanitizing(replaced());
    // `quietharbor` is replaced; the owner onevcs derives, `hiddenco`, has no replacement.
    let body = format!("Retry like {TERM} and hiddenco do.");
    store.record(
        "site",
        "tasks",
        "mixed",
        "title: Mixed\nstatus: todo\nproject: p",
        &body,
    );
    let before = tree(&store.folder("site"));
    let output = store.run(&[
        "task",
        "priority",
        "set",
        "site:mixed",
        "high",
        "--json",
        "--sanitize-dry-run",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(
        tree(&store.folder("site")),
        before,
        "a dry run writes nothing"
    );

    let output = store.run(&["task", "priority", "set", "site:mixed", "high", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("boundary-refused"));
    assert_eq!(failure["predates_write"], json!(true));
    let said = stderr(&output);
    assert!(
        said.contains("already on the item before this write")
            && said.contains("a term no replacement is declared for"),
        "{said}"
    );
    let held = text_of(&store, "site");
    assert!(
        held.contains(&format!("Retry like {WITH} and hiddenco do.")),
        "{held}"
    );
    assert!(
        !held.contains("priority: high"),
        "the write itself was not made: {held}"
    );
}

#[test]
fn a_comment_edit_is_replaced_and_a_replacement_that_empties_a_comment_writes_nothing() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "talk",
        "title: Talk\nstatus: todo\nproject: p",
        PLAIN,
    );
    let added: Value = serde_json::from_str(&store.ok(&[
        "task",
        "comment",
        "add",
        "site:talk",
        "--body-file",
        &store.body("First.\n"),
        "--json",
    ]))
    .expect("the comment");
    let comment = added["comment"]["id"]
        .as_str()
        .or_else(|| added["id"].as_str())
        .unwrap_or_else(|| panic!("a comment id: {added}"))
        .to_owned();
    store.ok(&[
        "task",
        "comment",
        "edit",
        "site:talk",
        &comment,
        "--body-file",
        &store.body(&format!("Asked {TERM}.\n")),
    ]);
    let held = text_of(&store, "site");
    assert!(
        held.contains(&format!("Asked {WITH}.")) && !held.contains(TERM),
        "{held}"
    );

    let before = tree(&store.folder("site"));
    let output = store.run(&[
        "task",
        "comment",
        "add",
        "site:talk",
        "--body-file",
        &store.body(TERM),
        "--replace",
        &format!("{TERM}="),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(!stderr(&output).contains(TERM), "{}", stderr(&output));
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
}

#[test]
fn a_project_s_and_a_document_s_metadata_value_is_written_replaced() {
    let (store, _record) = sanitizing(replaced());
    store.ok(&[
        "document",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Notes",
        "--body-file",
        &store.body(PLAIN),
    ]);
    let document = files_under(&store, "site")
        .into_iter()
        .find(|path| path.contains("documents"))
        .expect("the document");
    let document_id = std::path::Path::new(&document)
        .file_stem()
        .expect("a stem")
        .to_string_lossy()
        .into_owned();
    for (kind, id) in [
        ("project", "site:p".to_owned()),
        ("document", format!("site:{document_id}")),
    ] {
        store.ok(&[
            kind,
            "metadata",
            "set",
            &id,
            "myapp.note",
            &format!("\"seen in {TERM}\""),
        ]);
    }
    let held = text_of(&store, "site");
    assert_eq!(
        held.matches(&format!("seen in {WITH}")).count(),
        2,
        "{held}"
    );
    assert!(!held.contains(TERM), "{held}");
}

#[test]
fn a_re_render_with_a_term_in_an_answer_regenerates_the_content_replaced() {
    let (store, _record) = sanitizing(replaced());
    let template = template(&store);
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        "goal=Plain goal",
        "--no-interactive",
        "--json",
    ]))
    .expect("the task");
    let id = created["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    store.ok(&[
        "task",
        "render",
        &id,
        "--template",
        &template,
        "--var",
        &format!("goal=Match {TERM}"),
        "--no-interactive",
    ]);
    let held = text_of(&store, "site");
    assert!(held.contains(&format!("# Match {WITH}")), "{held}");
    assert!(!held.contains(TERM), "{held}");
}

#[test]
fn a_complete_sweep_reads_every_comment_page_and_rewrites_a_hit_on_a_later_one() {
    let shared = Shared::new(&[("t1", "Nothing private.")], json!({}), json!({}));
    let mut state = shared.state();
    state["comments"]["t1"] = json!(
        (0..130)
            .map(
                |n| json!({"id": format!("c-{n}"), "author": "ada", "created_at": null,
            "updated_at": null, "url": null, "origin": null,
            "body": if n == 129 { format!("Asked {TERM}.") } else { format!("Comment {n}.") }})
            )
            .collect::<Vec<_>>()
    );
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let (code, report, out, err) = sweep(&shared.store, "shared", &["--complete"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(
        entry(&report, "shared:t1")["fields"],
        json!([{"field": "comment:c-129", "outcome": "rewritten"}]),
        "{report}"
    );
    let comments = shared.state()["comments"]["t1"].clone();
    assert_eq!(comments[129]["body"], json!(format!("Asked {WITH}.")));
    assert_eq!(comments[0]["body"], json!("Comment 0."));
}

#[test]
fn a_sweep_reports_an_item_naming_a_private_repository_unfixable_and_fixes_the_rest_of_it() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "leaked",
        "title: Leaked\nstatus: todo\nproject: p\nrepositories: [github.com/hiddenco/quietharbor]",
        &format!("Mirror {TERM} here."),
    );
    let (code, report, out, err) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    let leaked = entry(&report, "site:leaked");
    assert_eq!(leaked["outcome"], json!("unfixable"), "{report}");
    assert_eq!(
        leaked["fields"][0],
        json!({"field": "repositories", "outcome": "unfixable", "why": "names a private repository"}),
        "{report}"
    );
    assert_eq!(
        leaked["fields"][1],
        json!({"field": "content", "outcome": "rewritten"}),
        "{report}"
    );
    assert!(text_of(&store, "site").contains(&format!("Mirror {WITH} here.")));
}

#[test]
fn a_public_view_a_sanitisation_would_have_to_rewrite_is_refused_before_anything_is_written() {
    let store = Store::laid(
        Some(registered),
        |_| json!({"vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]}}),
    );
    let home = store.onevcs.as_ref().expect("a home");
    let policy = home.sanitizing_write_policy(replaced());
    repolicy(&store, |configured| *configured = policy);
    store.record(
        "plan",
        "projects",
        "rollout",
        "title: Rollout\nstatus: todo",
        PLAIN,
    );
    // Private work of its own, so the plan homes privately and its public task is a view.
    store.record(
        "plan",
        "tasks",
        "producer",
        "title: Producer\nstatus: todo\nproject: rollout\n\
         repositories: [github.com/hiddenco/quietharbor]",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "leaky",
        &format!(
            "title: Port the {TERM} retry\nstatus: todo\nproject: rollout\nrepositories: \
             [github.com/openco/openwidget]"
        ),
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(
        said.contains("a public view is never rewritten") && !said.contains(TERM),
        "{said}"
    );
    let after = store.tree();
    assert!(
        after
            .iter()
            .filter(|(path, _)| !path.contains("boundary-record"))
            .eq(before
                .iter()
                .filter(|(path, _)| !path.contains("boundary-record"))),
        "nothing of the plan was written"
    );
}

#[test]
fn a_sweep_of_an_empty_source_whose_name_carries_a_term_redacts_it_and_exits_one() {
    let (store, _record) = sanitizing(replaced());
    let named = format!("{TERM}-empty");
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["sources"][&named] = json!({
        "plugin": "local-md",
        "config": {"root": store.sandbox.subdirectory("empty")},
        "visibility": "public",
    });
    store.sandbox.project_document(&document.to_string());
    let (code, report, out, err) = sweep(&store, &named, &[]);
    assert_eq!(
        code,
        Some(1),
        "a record nothing can name is not a clean sweep: {out}{err}"
    );
    assert_eq!(report["items"], json!([]), "{report}");
    assert_eq!(
        report["source"],
        json!({"redacted": true, "kind": "source", "ordinal": 1}),
        "{report}"
    );
    spelled_nowhere(&store, &[&out, &err]);
}

#[test]
fn a_record_whose_fresh_read_the_policy_refuses_is_refused_writing_nothing() {
    // The other writer's edit adds the owner the policy derives, which has no replacement.
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"paragraph": "Owned by hiddenco."}}),
    );
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(!said.contains("hiddenco") && !said.contains(TERM), "{said}");
    assert_eq!(shared.writes(), 0, "nothing was written");
    let task = shared.task("t1");
    assert_eq!(task["priority"], json!("none"));
    assert!(
        task["content"]
            .as_str()
            .is_some_and(|content| content.ends_with("Owned by hiddenco."))
    );
}

#[test]
fn a_search_hit_the_policy_finds_no_term_in_is_reported_not_a_match_and_written_nothing() {
    // The index still holds the term; the record a person since cleaned does not, and carries
    // no replacement either, so nothing a sanitisation did explains the hit.
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    shared.patch("t1", json!({"content": "Cleaned by hand."}));
    let (code, report, out, err) = sweep(&shared.store, "shared", &[]);
    assert_eq!(code, Some(3), "{out}{err}");
    assert_eq!(
        entry(&report, "shared:t1"),
        &json!({"id": "shared:t1", "kind": "task", "outcome": "not-a-match", "fields": []}),
        "{report}"
    );
    assert_eq!(shared.writes(), 0);
}

#[test]
fn a_record_another_writer_cleaned_before_the_sanitising_write_is_left_as_they_left_it() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"content": "Cleaned by hand."}}),
    );
    let answer: Value = serde_json::from_str(&shared.store.ok(&[
        "task",
        "priority",
        "set",
        "shared:t1",
        "high",
        "--json",
    ]))
    .expect("the answer");
    assert_eq!(
        shared.writes(),
        0,
        "nothing left to sanitise, so no sanitising write"
    );
    let task = shared.task("t1");
    assert_eq!(task["content"], json!("Cleaned by hand."));
    assert_eq!(task["priority"], json!("high"));
    assert!(
        answer["sanitization"]["side_effect"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{answer}"
    );
}

#[test]
fn a_comment_another_writer_adds_before_the_sanitising_write_is_sanitised_and_named_by_its_id() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"comment": format!("Ask {TERM} too.")}}),
    );
    let answer: Value = serde_json::from_str(&shared.store.ok(&[
        "task",
        "priority",
        "set",
        "shared:t1",
        "high",
        "--json",
    ]))
    .expect("the answer");
    let fields: Vec<&str> = answer["sanitization"]["side_effect"]
        .as_array()
        .expect("side effects")
        .iter()
        .map(|effect| effect["field"].as_str().expect("a plain field"))
        .collect();
    assert_eq!(fields, ["content", "comment:by-person-1"], "{answer}");
    let comments = shared.state()["comments"]["t1"].clone();
    assert_eq!(
        comments[0]["body"],
        json!(format!("Ask {WITH} too.")),
        "{comments}"
    );
}

#[test]
fn a_terms_command_that_fails_or_answers_out_of_shape_is_an_undecided_policy_exiting_two() {
    let (store, _record) = sanitizing(replaced());
    let python = crate::document_store::interpreter();
    for (what, script, said) in [
        (
            "an undecided answer",
            "import sys; sys.stdin.read(); print('{\"verdict\": \"unavailable\", \"reason\": \"registry\"}'); sys.exit(2)",
            "could not decide (registry)",
        ),
        (
            "a failure",
            "import sys; sys.exit(1)",
            "without terms this store believes",
        ),
        (
            "terms beside an undecided verdict",
            "import sys; sys.stdin.read(); print('{\"terms\": [], \"verdict\": \"unavailable\", \"reason\": \"registry\"}')",
            "could not decide (registry)",
        ),
        (
            "an answer out of shape",
            "import sys; sys.stdin.read(); print('{\"terms\": [{\"term\": 7}]}')",
            "without terms this store believes",
        ),
    ] {
        repolicy(&store, |policy| {
            policy["terms_command"] = json!([python.to_string_lossy(), "-c", script]);
        });
        // An undecided policy, however it failed, exits 2 as a write's undecided check does.
        let (code, _, out, err) = sweep(&store, "site", &[]);
        assert_eq!(code, Some(2), "{what}: {out}{err}");
        assert!(
            err.contains("was not swept") && err.contains(said),
            "{what}: {err}"
        );
    }
}

#[test]
fn a_replacement_the_environment_spells_as_a_number_or_a_list_is_read_as_the_text_it_was_given() {
    let (store, _record) = sanitizing(json!([]));
    let home = store.onevcs.as_ref().expect("a home").path().to_path_buf();
    for (with, written) in [("2024", "2024"), ("harbor,x", "harbor,x")] {
        let mut command = store.sandbox.command();
        command
            .args([
                "task",
                "create",
                "site",
                "--project",
                "p",
                "--title",
                &format!("From {TERM}"),
                "--body-file",
                &store.body(PLAIN),
                "--json",
            ])
            .env("ONEVCS_HOME", &home)
            .env("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__TERM", TERM)
            .env("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__WITH", with);
        let output = command.assert().get_output().clone();
        assert_eq!(output.status.code(), Some(0), "{with}: {}", stderr(&output));
        let created: Value = serde_json::from_str(&stdout(&output)).expect("a task");
        assert_eq!(
            created["items"][0]["item"]["title"],
            json!(format!("From {written}")),
            "{with}"
        );
    }
}

#[test]
fn a_moved_record_whose_new_comment_id_carries_a_term_is_refused_naming_it_only_redacted() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"comment": "Plain.", "comment_id": format!("{TERM}-c9")}}),
    );
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(!said.contains(TERM), "{said}");
    assert_eq!(shared.writes(), 0, "nothing was written");
}

#[test]
fn a_sanitising_write_keeps_every_forward_edge_of_its_task() {
    let shared = Shared::new(
        &[
            ("t1", &with_term()),
            ("t2", "Plain."),
            ("t3", "Plain."),
            ("t4", "Plain."),
        ],
        json!({}),
        json!({}),
    );
    let edges: Vec<Value> = ["t2", "t3", "t4"]
        .into_iter()
        .map(|to| {
            json!({"from": {"id": "t1", "kind": "task"}, "to": {"id": to, "kind": "task"},
               "kind": "blocks"})
        })
        .collect();
    let mut state = shared.state();
    state["edges"]["t1"] = json!(edges);
    state["page_size"] = json!(1);
    state["dependency_cursors"] = json!([]);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    shared
        .store
        .ok(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(shared.writes(), 1, "one sanitising write");
    assert_eq!(
        shared.state()["edges"]["t1"],
        json!(edges),
        "the edge was written back as it was"
    );
    let deps: Value =
        serde_json::from_str(&shared.store.ok(&["task", "deps", "shared:t1", "--json"]))
            .expect("the dependencies");
    assert_eq!(deps["items"].as_array().map(Vec::len), Some(3), "{deps}");
    let state = shared.state();
    let cursors = state["dependency_cursors"]
        .as_array()
        .expect("dependency pages");
    assert!(
        cursors.contains(&json!("1")) && cursors.contains(&json!("2")),
        "{state}"
    );
}

#[test]
fn a_sanitising_write_refuses_a_private_term_in_a_preserved_dependency_endpoint() {
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    let mut state = shared.state();
    let edge = json!({"from": {"id": "t1", "kind": "task"},
        "to": {"id": format!("{TERM}-t2"), "kind": "task"}, "kind": "blocks"});
    state["edges"]["t1"] = json!([edge]);
    std::fs::write(&shared.state, state.to_string()).expect("state");
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(!said.contains(TERM), "{said}");
    assert_eq!(
        shared.writes(),
        0,
        "no sanitizing write crosses an unscreened edge"
    );
    assert_eq!(shared.state()["edges"], state["edges"]);
}

#[test]
fn a_complete_sweep_writes_each_page_before_reading_the_next() {
    let shared = Shared::new(
        &[("t1", &with_term()), ("t2", &with_term())],
        json!({}),
        json!({}),
    );
    let mut state = shared.state();
    state["page_size"] = json!(1);
    std::fs::write(&shared.state, state.to_string()).expect("state");
    let (code, report, out, err) = sweep(&shared.store, "shared", &["--complete"]);
    assert_eq!(code, Some(0), "{report}{out}{err}");
    let state = shared.state();
    let log = state["log"].as_array().expect("peer calls");
    let pages: Vec<_> = log
        .iter()
        .enumerate()
        .filter_map(|(index, method)| (method == "query_tasks").then_some(index))
        .collect();
    assert_eq!(pages.len(), 2, "{state}");
    assert!(
        log[pages[0] + 1..pages[1]]
            .iter()
            .any(|method| method == "write_task"),
        "first page must be written before the second read: {state}"
    );
    assert!(
        log[pages[1] + 1..]
            .iter()
            .any(|method| method == "write_task"),
        "second page must also be written: {state}"
    );
    assert_eq!(
        report["items"].as_array().map(Vec::len),
        Some(2),
        "{report}"
    );
}

#[test]
fn a_search_sweep_repairs_matches_on_later_native_search_pages() {
    let shared = Shared::new(
        &[
            ("t1", &with_term()),
            ("t2", &with_term()),
            ("t3", &with_term()),
        ],
        json!({}),
        json!({}),
    );
    let mut state = shared.state();
    state["page_size"] = json!(1);
    state["query_cursors"] = json!([]);
    std::fs::write(&shared.state, state.to_string()).expect("state");
    let (code, report, out, err) = sweep(&shared.store, "shared", &[]);
    assert_eq!(code, Some(3), "{out}{err}");
    assert_eq!(
        report["items"].as_array().map(Vec::len),
        Some(3),
        "{report}"
    );
    for id in ["t1", "t2", "t3"] {
        let shown: Value = serde_json::from_str(&shared.store.ok(&[
            "task",
            "show",
            &format!("shared:{id}"),
            "--json",
        ]))
        .expect("task");
        assert!(
            !shown["items"][0]["item"]["content"]
                .as_str()
                .expect("body")
                .contains(TERM)
        );
    }
    let state = shared.state();
    let cursors = state["query_cursors"].as_array().expect("search pages");
    assert!(
        cursors.contains(&json!("1")) && cursors.contains(&json!("2")),
        "{state}"
    );
}

#[test]
fn the_editing_peer_applies_each_native_search_field() {
    let shared = Shared::new(
        &[("needle", "Plain."), ("body", "A needle in the body.")],
        json!({}),
        json!({}),
    );
    for (fields, expected) in [
        ("title", vec!["shared:needle"]),
        ("content", vec!["shared:body"]),
        ("both", vec!["shared:body", "shared:needle"]),
    ] {
        let report: Value = serde_json::from_str(&shared.store.ok(&[
            "task", "list", "--source", "shared", "--search", "needle", "--in", fields, "--json",
        ]))
        .expect("tasks");
        let ids: Vec<&str> = report["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, expected, "{fields}: {report}");
    }
}

#[test]
fn a_source_keeping_no_version_is_still_read_again_when_its_record_moves() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"label": "person-label"}}),
    );
    let mut state = shared.state();
    state["unversioned"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    shared
        .store
        .ok(&["task", "priority", "set", "shared:t1", "high"]);
    let task = shared.task("t1");
    assert_eq!(
        labels(&task),
        ["person-label"],
        "the other writer's label is kept"
    );
    assert!(!task["content"].as_str().expect("content").contains(TERM));
    assert_eq!(task["priority"], json!("high"));
}

/// A sanitise command rewriting the term to nothing — which onevcs refuses to declare, and a
/// linking caller's policy might still answer.
const EMPTIES: &str = r#"
import json, sys
sent = json.loads(sys.stdin.read())
answer = {surface: [value.replace("quietharbor", "") for value in sent.get(surface, [])]
          for surface in ("text", "paths", "metadata")}
answer.update(verdict="pass", replaced=1, unfixed=[],
              locators=[locator["value"] for locator in sent.get("locators", [])])
print(json.dumps(answer))
"#;

#[test]
fn a_stored_comment_a_replacement_would_empty_leaves_the_whole_task_unwritten() {
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    shared.comment("t1", "c1", TERM);
    let python = crate::document_store::interpreter();
    repolicy(&shared.store, |policy| {
        policy["sanitize_command"] = json!([python.to_string_lossy(), "-c", EMPTIES]);
    });
    let output = shared
        .store
        .run(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(shared.writes(), 0, "the task was not half sanitised");
    assert_eq!(shared.task("t1")["content"], json!(with_term()));
    assert_eq!(shared.state()["comments"]["t1"][0]["body"], json!(TERM));
}

#[test]
fn a_re_render_whose_content_is_rewritten_records_the_digest_of_what_landed() {
    let (store, _record) = sanitizing(replaced());
    let template = template(&store);
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        "goal=Plain goal",
        "--no-interactive",
        "--json",
    ]))
    .expect("the task");
    let id = created["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let private_path = store
        .sandbox
        .config_home()
        .join(format!("{TERM}-template.md"));
    std::fs::copy(&template, &private_path).expect("a second template path");
    store.ok(&[
        "task",
        "render",
        &id,
        "--template",
        private_path.to_str().expect("a template path"),
        "--var",
        &format!("goal=Match {TERM}"),
        "--no-interactive",
    ]);
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", &id, "--json"])).expect("the task");
    let item = &shown["items"][0]["item"];
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["template"],
        json!(private_path.to_string_lossy().replace(TERM, WITH)),
    );
    let content = item["content"].as_str().expect("content");
    assert!(content.contains(&format!("# Match {WITH}")), "{content}");
    let recorded = item["metadata"]["onetaskgraph.template"]["body_digest"]
        .as_str()
        .expect("a body digest");
    let landed = {
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(content.as_bytes());
        format!(
            "sha256:{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    };
    assert_eq!(
        recorded, landed,
        "the provenance names the rendering as it landed"
    );
}

#[test]
fn regeneration_answers_a_sanitization_preview_and_a_predates_refusal_without_writing() {
    let (store, _) = sanitizing(replaced());
    stage_older(&store);
    let template = template(&store);
    let before = tree(&store.folder("site"));
    let args = [
        "task",
        "render",
        "site:older",
        "--template",
        &template,
        "--var",
        "goal=Plain goal",
        "--no-interactive",
        "--json",
    ];
    let mut preview = args.to_vec();
    preview.push("--sanitize-dry-run");
    let output = store.run(&preview);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a preview");
    assert_eq!(answer["would_pass"], json!(true), "{answer}");
    assert!(
        !answer["sanitization"]["side_effect"]
            .as_array()
            .expect("changes")
            .is_empty()
    );
    assert_eq!(tree(&store.folder("site")), before);
    repolicy(&store, |policy| {
        policy["replacements"] = json!([{ "term": TERM, "reject": true }]);
    });
    let output = store.run(&args);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure");
    assert_eq!(failure["predates_write"], json!(true), "{failure}");
    assert_eq!(tree(&store.folder("site")), before);
}

#[test]
fn a_source_keeping_no_version_moving_twice_is_a_conflict_too() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2, 3]}),
        json!({"t1": {"label": "person-label"}}),
    );
    let mut state = shared.state();
    state["unversioned"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let (kind, said) = shared
        .store
        .refused(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(kind, "sanitize-conflict", "{said}");
    assert_eq!(shared.writes(), 0, "nothing was overwritten");
    assert_eq!(shared.task("t1")["priority"], json!("none"));
}

#[test]
fn sanitising_reads_and_comment_answers_must_name_the_record_requested() {
    for (control, at, expected) in [
        ("wrong_read_id", 1, "another task"),
        ("wrong_read_id", 2, "another task"),
        ("wrong_comment_id", 2, "another comment"),
    ] {
        let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
        shared.comment("t1", "c1", &format!("Ask {TERM}."));
        let mut state = shared.state();
        state[control] = json!("another-record");
        state["wrong_read_at"] = json!(at);
        std::fs::write(&shared.state, state.to_string()).expect("state");
        let output = shared
            .store
            .run(&["task", "priority", "set", "shared:t1", "high"]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{control}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        assert!(stderr(&output).contains(expected), "{}", stderr(&output));
        assert_eq!(
            shared.task("t1")["priority"],
            "none",
            "original write refused"
        );
        if control == "wrong_read_id" {
            assert_eq!(
                shared.writes(),
                0,
                "no record overwritten after a malformed read"
            );
        }
    }
}

#[test]
fn a_sanitising_write_answered_as_another_task_refuses_the_write_it_was_for() {
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    let mut state = shared.state();
    state["answer_ids"] = json!({"t1": format!("{TERM}-t9")});
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let output = shared
        .store
        .run(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("another task"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).contains(TERM) && !stderr(&output).contains(TERM),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        shared.task("t1")["priority"],
        json!("none"),
        "the write it was for was not made"
    );
}

#[test]
fn a_comment_gone_before_its_sanitising_edit_refuses_the_write_it_was_for_saying_so() {
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    shared.comment("t1", "c1", &format!("Ask {TERM}."));
    let mut state = shared.state();
    state["drop_comments_on_write"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let output = shared
        .store
        .run(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let said = stderr(&output);
    assert!(
        said.contains("was gone before its sanitising write"),
        "{said}"
    );
    assert!(!said.contains(TERM), "{said}");
    // The task's own sanitising write had landed; the write it was for was not made.
    let task = shared.task("t1");
    assert!(!task["content"].as_str().expect("content").contains(TERM));
    assert_eq!(task["priority"], json!("none"));
}

#[test]
fn a_source_whose_request_count_cannot_be_read_is_searched_rather_than_read_whole() {
    let shared = Shared::new(
        &[("t1", &with_term()), ("t2", "Plain.")],
        json!({}),
        json!({}),
    );
    let mut state = shared.state();
    state["metering_fails"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let (code, report, out, err) = sweep(&shared.store, "shared", &[]);
    assert_eq!(code, Some(3), "{out}{err}");
    assert_eq!(report["complete"], json!(false), "{report}");
    assert_eq!(
        entry(&report, "shared:t1")["outcome"],
        json!("rewritten"),
        "{report}"
    );
    let log = shared.state()["log"].clone();
    assert!(
        log.as_array()
            .expect("a log")
            .iter()
            .filter(|method| *method == "query_tasks")
            .count()
            >= 1,
        "{log}"
    );
}

#[test]
fn a_sanitising_dry_run_s_human_answer_says_nothing_was_written_and_what_would_change() {
    let (store, _record) = sanitizing(replaced());
    stage_older(&store);
    let before = tree(&store.folder("site"));
    let said = store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--sanitize-dry-run",
    ]);
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
    let lines: Vec<&str> = said.trim_end().lines().collect();
    assert!(
        lines[0].starts_with("nothing was written") && lines[0].contains("would then pass"),
        "{said}"
    );
    let mut rest: Vec<&str> = lines[1..].to_vec();
    rest.sort_unstable();
    let mut expected: Vec<String> = SIDE_EFFECTS
        .iter()
        .map(|field| format!("would sanitise site:older {field}"))
        .collect();
    expected.sort();
    assert_eq!(rest, expected, "{said}");
    assert!(!said.contains(TERM));
}

#[test]
fn a_content_set_and_a_task_metadata_set_are_written_replaced() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "plain",
        "title: Plain\nstatus: todo\nproject: p",
        PLAIN,
    );
    store.ok(&[
        "task",
        "content",
        "set",
        "site:plain",
        "--file",
        &store.body(&format!("Retry the way {TERM} does.\n")),
    ]);
    store.ok(&[
        "task",
        "metadata",
        "set",
        "site:plain",
        "myapp.note",
        &format!("\"from {TERM}\""),
    ]);
    let held = text_of(&store, "site");
    assert!(
        held.contains(&format!("Retry the way {WITH} does.")),
        "{held}"
    );
    assert!(held.contains(&format!("from {WITH}")), "{held}");
    assert!(!held.contains(TERM), "{held}");
}

#[test]
fn a_project_copy_and_a_document_copy_land_with_every_term_replaced() {
    let (store, _record) = sanitizing(replaced());
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["sources"]["mirror"] = json!({
        "plugin": "local-md",
        "config": {"root": store.sandbox.subdirectory("mirror")},
        "visibility": "public",
    });
    store.sandbox.project_document(&document.to_string());
    store.record(
        "site",
        "projects",
        "port",
        &format!("title: The {TERM} port\nstatus: todo"),
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "step",
        &format!("title: Step past {TERM}\nstatus: todo\nproject: port"),
        &format!("Mirror {TERM}."),
    );
    store.record(
        "site",
        "documents",
        "notes",
        &format!("title: {TERM} notes\nproject: port\nmetadata:\n  myapp.note: about {TERM}"),
        &format!("Notes on {TERM}."),
    );
    store.ok(&["project", "copy", "site:port", "--to", "mirror"]);
    store.ok(&["document", "copy", "site:notes", "--to", "mirror"]);
    let mirrored = text_of(&store, "mirror");
    assert!(!mirrored.contains(TERM), "{mirrored}");
    for written in [
        format!("The {WITH} port"),
        format!("Step past {WITH}"),
        format!("Mirror {WITH}."),
        format!("{WITH} notes"),
        format!("about {WITH}"),
        format!("Notes on {WITH}."),
    ] {
        assert!(mirrored.contains(&written), "{written:?}:\n{mirrored}");
    }
    assert!(
        files_under(&store, "mirror")
            .iter()
            .all(|path| !path.contains(TERM)),
        "no copied record is named after the term"
    );
}

#[test]
fn a_project_and_a_document_re_rendered_with_a_term_in_an_answer_land_replaced() {
    let (store, _record) = sanitizing(replaced());
    let template = template(&store);
    store.ok(&[
        "project",
        "create",
        "site",
        "--id",
        "q",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        "goal=Plain goal",
        "--no-interactive",
    ]);
    store.ok(&[
        "document",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Rendered notes",
        "--template",
        &template,
        "--var",
        "goal=Plain goal",
        "--no-interactive",
    ]);
    let document = files_under(&store, "site")
        .into_iter()
        .find(|path| path.contains("documents"))
        .expect("the document");
    let document_id = std::path::Path::new(&document)
        .file_stem()
        .expect("a stem")
        .to_string_lossy()
        .into_owned();
    for (kind, id) in [
        ("project", "site:q".to_owned()),
        ("document", format!("site:{document_id}")),
    ] {
        store.ok(&[
            kind,
            "render",
            &id,
            "--template",
            &template,
            "--var",
            &format!("goal=Match {TERM}"),
            "--no-interactive",
        ]);
    }
    let held = text_of(&store, "site");
    assert_eq!(
        held.matches(&format!("# Match {WITH}")).count(),
        2,
        "{held}"
    );
    assert!(!held.contains(TERM), "{held}");
}

#[test]
fn a_write_s_sanitization_member_and_a_dry_run_s_answer_validate_against_the_emitted_schema() {
    let (store, _record) = sanitizing(replaced());
    let bundle: Value = serde_json::from_str(&store.ok(&["schema"])).expect("the bundle");
    let validator = |root: &str| {
        jsonschema::validator_for(&bundle["roots"][root])
            .unwrap_or_else(|error| panic!("{root} compiles: {error}"))
    };
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        &format!("Port {TERM}"),
        "--body-file",
        &store.body(PLAIN),
        "--json",
    ]))
    .expect("the task");
    let member = &created["sanitization"];
    assert!(member.is_object(), "{created}");
    assert!(validator("Sanitization").is_valid(member), "{member}");

    stage_older(&store);
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--json",
        "--sanitize-dry-run",
    ]))
    .expect("the dry run");
    let errors: Vec<String> = validator("SanitizeDryRun")
        .iter_errors(&answer)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{answer}: {errors:?}");
}

#[test]
fn a_sweep_names_each_field_a_term_no_replacement_covers_is_left_in() {
    let (store, _record) = sanitizing(replaced());
    // `hiddenco`, the owner onevcs derives, has no replacement.
    store.record(
        "site",
        "tasks",
        "owned",
        "title: Owned by hiddenco\nstatus: todo\nproject: p",
        &format!(
            "{PLAIN}\n\n## Comments\n\n{}",
            comment("20260913T151107Z-1", "Ask hiddenco.")
        ),
    );
    let (code, report, out, err) = sweep(&store, "site", &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    let named: Vec<&str> = entry(&report, "site:owned")["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| field["field"].as_str().expect("a plain field"))
        .collect();
    assert_eq!(named, ["comment:20260913T151107Z-1", "title"], "{report}");
    assert!(
        !out.contains("hiddenco") && !err.contains("hiddenco"),
        "{out}{err}"
    );
}

#[test]
fn a_replacement_setting_addressed_by_no_index_or_of_no_list_shape_is_refused_at_load() {
    let (store, _record) = sanitizing(json!([]));
    for (setting, said) in [
        ("write_policy.replacements.first.term=x", "is not one"),
        ("write_policy.replacements=x", "is a list"),
    ] {
        let output = store.run(&["task", "list", "--set", setting]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{setting}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("write_policy") && stderr(&output).contains(said),
            "{setting}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_copy_split_between_a_private_and_a_public_destination_replaces_only_what_lands_in_public() {
    let store = Store::laid(
        Some(registered),
        |_| json!({"vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]}}),
    );
    let home = store.onevcs.as_ref().expect("a home");
    let policy = home.sanitizing_write_policy(replaced());
    repolicy(&store, |configured| *configured = policy);
    let body = format!("Retry the way {TERM} does.");
    store.record(
        "plan",
        "tasks",
        "public",
        "title: Public\nstatus: todo\nrepositories: [github.com/openco/openwidget]",
        &body,
    );
    store.record("plan", "tasks", "kept", "title: Kept\nstatus: todo", &body);
    store.ok(&["task", "copy", "plan:public", "plan:kept", "--to", "vault"]);
    let public = text_of(&store, "site");
    assert!(
        public.contains(&format!("Retry the way {WITH} does.")) && !public.contains(TERM),
        "{public}"
    );
    let private = text_of(&store, "vault");
    assert!(
        private.contains(&body),
        "the private destination takes the original: {private}"
    );
}

#[tokio::test]
async fn a_retry_on_one_engine_after_a_failed_sanitising_write_sanitises_again_rather_than_passing()
{
    // Every write fails without landing, so the task still carries the term at the retry.
    let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
    let mut state = shared.state();
    state["refuse_writes"] = json!(true);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let home = shared
        .store
        .onevcs
        .as_ref()
        .expect("a home")
        .path()
        .to_path_buf();
    let engine = super::linked_engine(&shared.store)
        .with_write_policy(std::sync::Arc::new(Sanitizing {
            home,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }))
        .with_replacements(vec![onetaskgraph_core::boundary::Replacement::Replace {
            term: TERM.to_owned(),
            with: WITH.to_owned(),
        }]);
    let id: onetaskgraph_core::GlobalId = "shared:t1".parse().expect("an id");
    for attempt in 1..=2 {
        let refused = engine
            .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
            .await
            .expect_err("the sanitising write fails");
        assert!(
            refused.to_string().contains("could not be made"),
            "{attempt}: {refused}"
        );
    }
    assert_eq!(
        shared.task("t1")["priority"],
        json!("none"),
        "the write it was for was never made"
    );
    assert_eq!(shared.writes(), 2, "each attempt sanitised again");
}

/// A linking caller's policy that cannot decide, and answers locators anyway.
struct UndecidedWithLocators;

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for UndecidedWithLocators {
    async fn repository_visibility(
        &self,
        _repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Ok(onetaskgraph_core::boundary::RepositoryVisibility::Public)
    }

    async fn check_public_write(
        &self,
        _input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        panic!("this policy sanitises")
    }

    async fn sanitize_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicSanitizeInput,
    ) -> Result<
        onetaskgraph_core::boundary::SanitizeVerdict,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Ok(onetaskgraph_core::boundary::SanitizeVerdict {
            verdict: onetaskgraph_core::boundary::WriteVerdict::Unavailable {
                reason: "registry".to_owned(),
            },
            text: input.text.clone(),
            paths: input.paths.clone(),
            metadata: input.metadata.clone(),
            replaced: 0,
            unfixed: Vec::new(),
            // Plain, and so a value a diagnostic would print — which an undecided call never
            // screened.
            locators: input
                .locators
                .iter()
                .map(|locator| onetaskgraph_core::boundary::Locator::Plain(locator.value.clone()))
                .collect(),
        })
    }
}

#[tokio::test]
async fn an_undecided_linking_policy_s_locators_are_never_printed() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "site",
        "tasks",
        &format!("{TERM}-note"),
        "title: Note\nstatus: todo\nproject: p",
        PLAIN,
    );
    let engine =
        super::linked_engine(&store).with_write_policy(std::sync::Arc::new(UndecidedWithLocators));
    let id: onetaskgraph_core::GlobalId = format!("site:{TERM}-note").parse().expect("an id");
    let refused = engine
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect_err("an undecided policy never passes");
    let said = refused.to_string();
    assert_eq!(super::kind_of(&refused), "boundary-unavailable", "{said}");
    assert!(
        said.contains("<redacted item") && !said.contains(TERM),
        "{said}"
    );
}

#[test]
fn a_sweep_whose_source_fails_part_way_fails_naming_it_and_a_task_gone_is_passed_over() {
    for (switch, said) in [
        ("query_fails", "could not be listed"),
        ("comments_fail", "comments could not be read"),
        ("later_comments_fail", "comments could not be read"),
    ] {
        let shared = Shared::new(&[("t1", &with_term())], json!({}), json!({}));
        let mut state = shared.state();
        state["comments"]["t1"] = json!(
            (0..130)
                .map(|n| json!({"id": format!("c-{n}"), "body": format!("Comment {n}.")}))
                .collect::<Vec<_>>()
        );
        state[switch] = json!(true);
        std::fs::write(&shared.state, state.to_string()).expect("the state");
        let (code, _, out, err) = sweep(&shared.store, "shared", &["--complete"]);
        assert_eq!(code, Some(1), "{switch}: {out}{err}");
        assert!(err.contains(said), "{switch}: {err}");
        assert_eq!(shared.writes(), 0, "{switch}: nothing was written");
    }
    // A task the listing names and a read then finds gone is no record to sweep.
    let shared = Shared::new(
        &[("t1", &with_term()), ("t2", &with_term())],
        json!({}),
        json!({}),
    );
    let mut state = shared.state();
    state["gone"] = json!(["t1"]);
    std::fs::write(&shared.state, state.to_string()).expect("the state");
    let (code, report, out, err) = sweep(&shared.store, "shared", &["--complete"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["id"].clone())
            .collect::<Vec<_>>(),
        [json!("shared:t2")],
        "{report}"
    );
}

#[test]
fn a_comment_edited_without_moving_its_task_is_sanitised_from_what_it_now_says() {
    let shared = Shared::new(
        &[("t1", "Nothing private.")],
        json!({"t1": [2]}),
        json!({
            "t1": {"comment_target": "c1", "comment_body": format!("Ask {TERM}, and a person's note."),
                   "silent": "yes"}
        }),
    );
    shared.comment("t1", "c1", &format!("Ask {TERM}."));
    shared
        .store
        .ok(&["task", "priority", "set", "shared:t1", "high"]);
    let comments = shared.state()["comments"]["t1"].clone();
    assert_eq!(
        comments[0]["body"],
        json!(format!("Ask {WITH}, and a person's note.")),
        "the person's edit is kept and sanitised: {comments}"
    );
    assert_eq!(shared.task("t1")["priority"], json!("high"));
}

#[test]
fn a_clean_comment_that_acquires_a_term_without_moving_its_task_is_sanitised() {
    let shared = Shared::new(
        &[("t1", &with_term())],
        json!({"t1": [2]}),
        json!({"t1": {"comment_target": "c1", "comment_body": format!("Ask {TERM}, and a person's note."), "silent": "yes"}}),
    );
    shared.comment("t1", "c1", "Nothing private.");
    let output = shared
        .store
        .ok(&["task", "priority", "set", "shared:t1", "high"]);
    assert_eq!(
        shared.state()["comments"]["t1"][0]["body"],
        json!(format!("Ask {WITH}, and a person's note."))
    );
    assert_eq!(shared.task("t1")["priority"], json!("high"));
    assert!(output.contains("sanitised"), "{output}");
}

#[test]
fn a_sweep_screens_a_term_bearing_source_even_when_its_read_fails() {
    for switch in ["query_fails", "comments_fail", "later_comments_fail"] {
        let shared = Shared::named(TERM, &[("t1", &with_term())], json!({}), json!({}));
        let mut state = shared.state();
        state["comments"]["t1"] = json!(
            (0..130)
                .map(|n| json!({"id":format!("c{n}"),"body":"Plain."}))
                .collect::<Vec<_>>()
        );
        state[switch] = json!(true);
        std::fs::write(&shared.state, state.to_string()).expect("state");
        let (code, _, out, err) = sweep(&shared.store, TERM, &["--complete"]);
        assert_eq!(code, Some(1), "{out}{err}");
        assert!(err.contains("<redacted source 1>"), "{switch}: {err}");
        spelled_nowhere(&shared.store, &[&out, &err]);
        assert_eq!(shared.writes(), 0);
    }
}

#[test]
fn a_sanitise_dry_run_of_a_write_whose_item_needs_nothing_makes_the_write() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo\nproject: p",
        PLAIN,
    );
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:clean",
        "high",
        "--json",
        "--sanitize-dry-run",
    ]))
    .expect("the answer");
    assert_eq!(answer["priority"], json!("high"), "{answer}");
    assert!(answer.get("sanitization").is_none(), "{answer}");
    assert!(text_of(&store, "site").contains("priority: high"));
}

#[test]
fn an_incoming_replacement_that_writes_does_not_claim_to_be_a_dry_run() {
    let (store, _) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo\nproject: p",
        PLAIN,
    );
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "update",
        "site:clean",
        "--title",
        &format!("New {TERM}"),
        "--json",
        "--sanitize-dry-run",
    ]))
    .expect("answer");
    assert_eq!(answer["sanitization"]["dry_run"], false, "{answer}");
    assert!(text_of(&store, "site").contains(&format!("New {WITH}")));
}
