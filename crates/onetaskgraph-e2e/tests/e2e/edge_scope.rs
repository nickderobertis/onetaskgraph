//! Which trackers a task's dependency read asks, and how a public task names a private
//! prerequisite without saying anything about it.
//!
//! Every journey drives the binary against folders of Markdown and loopback Linear workspaces,
//! each workspace recording every request it is sent, so a journey asserts which source was
//! asked what — designator discovery (`organization`) counted apart from the dependency and
//! item reads — and that a source never asked received nothing at all. Every name here is
//! synthetic; a private item's title, key, team, project and address are distinctive strings,
//! so a journey can say none of them reached a public store, a request to one, or a refusal.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_linear::graphql;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::{
    LinearWorkspace, dataset, document, github_projects_with_board, linear_rate_limited,
    linear_workspace,
};

/// A configuration of `sources`, with a `write_policy` whose check passes every write and which
/// answers every repository public: a store declaring a visibility is held to the public
/// boundary, and what is under test here is the store's own rule for a private reference, which
/// it applies before any policy is asked.
fn configured(sources: &Value) -> String {
    let mut written: Value = serde_json::from_str(&document(sources)).expect("a document");
    written["write_policy"] = json!({
        "check_command": [
            interpreter(), "-c", "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"
        ],
        "visibility_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"
        ],
    });
    serde_json::to_string_pretty(&written).expect("a document renders")
}

/// The contract's designator for a backend `plugin` identifies by `identifier`, computed here
/// rather than asked of the engine that writes it.
fn designator(plugin: &str, identifier: &str) -> String {
    let digest: String = Sha256::digest(format!("{plugin}\n{identifier}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{plugin}/{digest}")
}

/// A Linear workspace of organization `organization` holding `tasks`, and nothing else.
fn workspace(sandbox: &Sandbox, organization: &str, tasks: Value) -> (Value, LinearWorkspace) {
    let mut held = dataset();
    for (_, kept) in held.as_object_mut().expect("the dataset is an object") {
        *kept = json!([]);
    }
    held["tasks"] = tasks;
    held["organization"] = json!(organization);
    linear_workspace(sandbox, held)
}

/// One Linear issue at `Todo`, carrying `extra` beside its title.
fn issue(id: &str, title: &str, extra: Value) -> Value {
    let mut issue = json!({"id": id, "title": title, "content": "",
                           "status": {"category": "todo", "name": "Todo"}, "labels": []});
    issue
        .as_object_mut()
        .expect("an issue")
        .extend(extra.as_object().expect("extra keys").clone());
    issue
}

/// A folder of Markdown under the sandbox, holding each `(id, front matter)` as a task.
fn folder(sandbox: &Sandbox, name: &str, tasks: &[(&str, &str)]) -> PathBuf {
    let root = sandbox.subdirectory(name);
    std::fs::create_dir_all(root.join("tasks")).expect("the tasks folder");
    for (id, front) in tasks {
        std::fs::write(
            root.join("tasks").join(format!("{id}.md")),
            format!("---\n{front}---\n"),
        )
        .expect("a task file");
    }
    root
}

/// `onetaskgraph <arguments>`, with this sandbox's own cache home.
fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .env("XDG_CACHE_HOME", sandbox.subdirectory("cache"))
        .args(arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone()
}

/// The JSON a run printed, having exited `code`.
fn answered(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Value {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`:\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// The `(from, to)` of every edge a dependency read answered.
fn edges(read: &Value) -> Vec<(String, String)> {
    read["items"]
        .as_array()
        .expect("the edges")
        .iter()
        .map(|edge| {
            (
                edge["from"]["id"].as_str().expect("an id").to_owned(),
                edge["to"]["id"].as_str().expect("an id").to_owned(),
            )
        })
        .collect()
}

/// What `workspace` was sent since `from` requests ago, each as its operation's name.
fn sent(workspace: &LinearWorkspace, from: usize) -> Vec<String> {
    workspace
        .served()
        .into_iter()
        .skip(from)
        .map(|(query, variables)| operation(&query, &variables))
        .collect()
}

/// A request's operation, by the plugin's own constant it is, with the id it reads where it
/// reads one.
fn operation(query: &str, variables: &Value) -> String {
    let name = if query == graphql::ORGANIZATION {
        "organization"
    } else if query == graphql::ISSUE {
        "issue"
    } else if query == graphql::ISSUES {
        "issues"
    } else if query == graphql::ISSUE_RELATIONS {
        "issue-relations"
    } else if query == graphql::VIEWER {
        "viewer"
    } else {
        "other"
    };
    match variables.get("id").and_then(Value::as_str) {
        Some(id) => format!("{name} {id}"),
        None => name.to_owned(),
    }
}

/// How many of `sent` are the designator's one request.
fn discovered(sent: &[String]) -> usize {
    sent.iter().filter(|name| *name == "organization").count()
}

/// A task in a source not declared private, landed by no copy, reads its edges from its own
/// source alone: neither a private Linear source that would answer rate limited, nor any other
/// source, is sent anything — and the read answers its native edge.
#[test]
fn a_task_no_copy_landed_asks_no_other_source() {
    let sandbox = Sandbox::new();
    let notes = folder(
        &sandbox,
        "notes",
        &[
            ("T-1", "title: Ship\ndepends_on: [T-2]\n"),
            ("T-2", "title: Build\n"),
        ],
    );
    let drafts = folder(&sandbox, "drafts", &[("D-1", "title: Draft\n")]);
    let (limited, limited_sent) = linear_rate_limited(&sandbox);
    let (open, open_workspace) = workspace(
        &sandbox,
        "org-open",
        json!([issue("O-1", "Open", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
        "drafts": {"plugin": "local-md", "config": {"root": drafts}},
        "secret": {"plugin": "linear", "config": limited, "visibility": "private"},
        "open": {"plugin": "linear", "config": open, "visibility": "public"},
    })));

    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);

    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "notes:T-2".to_owned())]
    );
    assert!(read.get("unanswered").is_none(), "{read}");
    assert_eq!(
        *limited_sent.lock().unwrap(),
        0,
        "the private source was sent nothing"
    );
    assert_eq!(
        sent(&open_workspace, 0),
        Vec::<String>::new(),
        "nor the public one"
    );
}

/// The three conditions under which a dependency read asks a private source what it records of
/// a task, each failing alone: the reverse lookup runs for a copy destination lacking the mark
/// in a source not declared private, and finds its legacy record there — and does not run, the
/// private source sent nothing, when the task carries the mark, carries no copy record, or is
/// itself in a source declared private.
#[test]
fn the_reverse_lookup_runs_only_for_an_unmarked_copy_destination_in_a_public_source() {
    let recorded = "  onetaskgraph.copied_destination_version: sha256:00\n";
    let cases: [(&str, &str, &str, bool); 4] = [
        ("an unmarked copy destination", "public", recorded, true),
        (
            "a copy destination marked as holding its edges",
            "public",
            "  onetaskgraph.copied_destination_version: sha256:00\n  onetaskgraph.edges_on_record: 1\n",
            false,
        ),
        (
            "a task no copy landed",
            "public",
            "  caller.note: authored here\n",
            false,
        ),
        (
            "a task of a source declared private",
            "private",
            recorded,
            false,
        ),
    ];
    for (case, visibility, metadata, looked_up) in cases {
        let sandbox = Sandbox::new();
        let site = folder(
            &sandbox,
            "site",
            &[("T-1", &format!("title: Ship\nmetadata:\n{metadata}"))],
        );
        let (vault, vault_workspace) = workspace(
            &sandbox,
            "org-vault",
            json!([issue(
                "V-1",
                "Approve",
                json!({"metadata": {
                    "onetaskgraph.depended_on_by": {"site:T-1": {"id": "site:T-1", "kind": "task"}}
                }})
            )]),
        );
        sandbox.project_document(&configured(&json!({
            "site": {"plugin": "local-md", "config": {"root": site}, "visibility": visibility},
            "vault": {"plugin": "linear", "config": vault, "visibility": "private"},
        })));

        let read = answered(&sandbox, &["task", "deps", "site:T-1", "--json"], 0);
        let asked = sent(&vault_workspace, 0);

        if looked_up {
            assert_eq!(
                edges(&read),
                [("site:T-1".to_owned(), "vault:V-1".to_owned())],
                "{case}: the legacy record is found"
            );
            assert!(
                asked.iter().any(|name| name == "issues"),
                "{case}: the private source was asked what it records: {asked:?}"
            );
            assert_eq!(
                discovered(&asked),
                0,
                "{case}: no designator was looked for"
            );
        } else {
            assert_eq!(edges(&read), Vec::<(String, String)>::new(), "{case}");
            assert_eq!(
                asked,
                Vec::<String>::new(),
                "{case}: the private source was sent nothing"
            );
        }
    }
}

/// Three Linear sources whose names put a workspace of another organization first, the one an
/// opaque edge names second, and another source over that same workspace third; and a folder
/// task recording a far end there opaquely.
struct Resolution {
    sandbox: Sandbox,
    first: LinearWorkspace,
    second: LinearWorkspace,
    third: LinearWorkspace,
}

fn resolution() -> Resolution {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!("title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"),
        )],
    );
    let (first, first_workspace) = workspace(
        &sandbox,
        "org-elsewhere",
        json!([issue("L-9", "Not this", json!({}))]),
    );
    let (second, second_workspace) = workspace(
        &sandbox,
        "org-plans",
        json!([issue("L-9", "Review", json!({}))]),
    );
    let (third, third_workspace) = workspace(
        &sandbox,
        "org-plans",
        json!([issue("L-9", "Review", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
        "aa-elsewhere": {"plugin": "linear", "config": first},
        "bb-plans": {"plugin": "linear", "config": second, "visibility": "private"},
        "cc-plans-too": {"plugin": "linear", "config": third, "visibility": "private"},
    })));
    Resolution {
        sandbox,
        first: first_workspace,
        second: second_workspace,
        third: third_workspace,
    }
}

/// A recorded opaque edge resolves to the source whose designator it is: on a cold cache each
/// candidate is sent at most one designator request, in name order, discovery stops at the
/// first match, and the far end is read on the matching source alone. Warm, nothing is
/// discovered again.
#[test]
fn an_opaque_edge_resolves_on_the_first_source_whose_designator_it_is() {
    let world = resolution();

    let read = answered(&world.sandbox, &["task", "deps", "notes:T-1", "--json"], 0);

    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "bb-plans:L-9".to_owned())]
    );
    assert!(read.get("unanswered").is_none(), "{read}");
    assert_eq!(
        sent(&world.first, 0),
        ["organization"],
        "asked who it is, and nothing else"
    );
    assert_eq!(
        sent(&world.second, 0),
        ["organization", "issue L-9"],
        "asked who it is, then the far end read there"
    );
    assert_eq!(sent(&world.third, 0), Vec::<String>::new(), "never reached");

    let (first, second) = (world.first.served().len(), world.second.served().len());
    let again = answered(&world.sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(edges(&again), edges(&read));
    assert_eq!(
        discovered(&sent(&world.first, first)),
        0,
        "kept on this host"
    );
    assert_eq!(
        sent(&world.second, second),
        ["issue L-9"],
        "kept on this host"
    );
}

/// A designator no configured source is leaves the edge out and names it, with why, in the
/// read's `unanswered` — never dropped silently — and the read still exits zero.
#[test]
fn an_opaque_edge_no_configured_source_is_is_named_unanswered() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-nobody-here");
    let notes = folder(
        &sandbox,
        "notes",
        &[
            (
                "T-1",
                &format!(
                    "title: Ship\ndepends_on:\n  - T-2\n  - {{designator: {far}, id: L-9, item: task}}\n"
                ),
            ),
            ("T-2", "title: Build\n"),
        ],
    );
    let (other, other_workspace) = workspace(
        &sandbox,
        "org-other",
        json!([issue("L-9", "Other", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "other": {"plugin": "linear", "config": other},
    })));

    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);

    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "notes:T-2".to_owned())]
    );
    assert_eq!(
        read["unanswered"],
        json!([{"designator": far, "reason": "not configured on this host"}])
    );
    assert_eq!(
        sent(&other_workspace, 0),
        ["organization"],
        "asked who it is, no more"
    );
    let text = run(&sandbox, &["task", "deps", "notes:T-1"]);
    assert!(
        stderr(&text).contains(&far) && stderr(&text).contains("not configured on this host"),
        "{}",
        stderr(&text)
    );
}

/// A private item's distinctive particulars, none of which may reach a public store.
const TITLE: &str = "Covert Nightjar Merger";
const KEY: &str = "ZVK-77";
const TEAM: &str = "Zephyr Vault Team";
const PROJECT: &str = "Project Nightjar";
const ADDRESS: &str = "https://linear.app/zephyr/issue/ZVK-77/covert-nightjar-merger";
const PRIVATE_SOURCE: &str = "zephyr-vault";
const NATIVE: &str = "9f2c41d0-5b1e-4d7a-9a63-0c7f2e3b8d11";

/// A public Linear board holding `B-1`, and a private Linear workspace holding the item
/// [`NATIVE`] whose particulars are the constants above, beside a decoy record `covert`.
fn boundary() -> (Sandbox, LinearWorkspace, LinearWorkspace) {
    let sandbox = Sandbox::new();
    let (board, board_workspace) = workspace(
        &sandbox,
        "org-public",
        json!([issue("B-1", "Publish the release notes", json!({}))]),
    );
    let (vault, vault_workspace) = workspace(
        &sandbox,
        "org-zephyr",
        json!([
            issue(
                NATIVE,
                TITLE,
                json!({
                    "_identifier": KEY, "url": ADDRESS,
                    "content": format!("{TEAM} — {PROJECT}"),
                })
            ),
            issue("covert", "Decoy record", json!({}))
        ]),
    );
    sandbox.project_document(&configured(&json!({
        "board": {"plugin": "linear", "config": board, "visibility": "public"},
        PRIVATE_SOURCE: {"plugin": "linear", "config": vault, "visibility": "private"},
    })));
    (sandbox, board_workspace, vault_workspace)
}

/// Assert none of the private item's particulars, nor its source's name, is in `text`.
fn conveys_nothing(what: &str, text: &str) {
    for secret in [
        TITLE,
        KEY,
        TEAM,
        PROJECT,
        ADDRESS,
        PRIVATE_SOURCE,
        "nightjar",
        "Nightjar",
    ] {
        assert!(!text.contains(secret), "{what} carries {secret:?}:\n{text}");
    }
}

/// Onto a destination not declared private, an edge naming a private item by a prefixed key, a
/// URL or a slug — or by an endpoint object carrying any key beyond `designator`, `id` and
/// `kind` — is refused as `private-reference-content`, naming the form or the key and never its
/// value, and nothing is written; the same edge given as the qualified native id is stored as
/// exactly `{designator, id, kind}`, and no request to the public board names the private item.
#[test]
fn a_public_destination_takes_a_private_prerequisite_only_by_its_native_id_opaquely() {
    let (sandbox, board, _vault) = boundary();
    let before = board.long_form("tasks", "B-1").expect("the board's task");

    let mut refusals: Vec<(String, &str)> = vec![
        (format!("{PRIVATE_SOURCE}:{KEY}"), "a prefixed key"),
        (format!("{PRIVATE_SOURCE}:{ADDRESS}"), "a URL"),
        (format!("{PRIVATE_SOURCE}:covert-nightjar-merger"), "a slug"),
    ];
    let far = designator("linear", "org-zephyr");
    let objects: Vec<(&str, Value)> = vec![
        ("title", json!(TITLE)),
        ("team", json!(TEAM)),
        ("project", json!(PROJECT)),
        ("labels", json!(["nightjar"])),
        ("status", json!("In Progress")),
        ("arbitrary", json!(ADDRESS)),
    ];
    for (key, value) in &objects {
        let mut object = json!({"designator": far, "id": NATIVE, "kind": "task"});
        object[*key] = value.clone();
        refusals.push((object.to_string(), key));
    }
    for (given, form) in &refusals {
        let output = run(
            &sandbox,
            &[
                "task",
                "update",
                "board:B-1",
                "--depends-on",
                given,
                "--json",
            ],
        );
        assert_ne!(output.status.code(), Some(0), "{given} is refused");
        let failure: Value = serde_json::from_str(&stdout(&output))
            .or_else(|_| serde_json::from_str(&stderr(&output)))
            .unwrap_or_else(|_| panic!("a failure document for {given}:\n{}", stderr(&output)));
        assert_eq!(
            failure["failure"]["kind"], "private-reference-content",
            "{given}: {failure}"
        );
        let message = format!("{}{}", stdout(&output), stderr(&output));
        assert!(message.contains(form), "{given}: names {form}:\n{message}");
        conveys_nothing("a refusal", &message);
    }
    assert_eq!(
        board.long_form("tasks", "B-1").as_deref(),
        Some(before.as_str()),
        "no refusal wrote anything"
    );

    let written_from = board.served().len();
    let updated = answered(
        &sandbox,
        &[
            "task",
            "update",
            "board:B-1",
            "--depends-on",
            &format!("{PRIVATE_SOURCE}:{NATIVE}"),
            "--json",
        ],
        0,
    );
    conveys_nothing("the update's answer", &updated.to_string());
    let stored = board.long_form("tasks", "B-1").expect("the board's task");
    let slot = stored
        .split("onetaskgraph.depends_on")
        .nth(1)
        .expect("the far end is recorded on the board's own item");
    assert!(
        slot.contains(&json!([{"designator": far, "id": NATIVE, "kind": "task"}]).to_string()),
        "stored as exactly the three keys:\n{stored}"
    );
    conveys_nothing("the stored record", &stored);
    for (query, variables) in board.served().into_iter().skip(written_from) {
        conveys_nothing(
            "a request to the public board",
            &format!("{query}{variables}"),
        );
    }
    assert!(
        !far.contains("org-zephyr"),
        "the designator conveys nothing of its identifier"
    );

    let read = answered(&sandbox, &["task", "deps", "board:B-1", "--json"], 0);
    assert_eq!(
        edges(&read),
        [("board:B-1".to_owned(), format!("{PRIVATE_SOURCE}:{NATIVE}"))]
    );
}

/// The designator is the contract's digest of the backend's own identifier: two hosts — each
/// with its own cache directory and its own name for a source over the same workspace — derive
/// the same one, each with one request that its cache directory then answers, and an edge
/// written under one name resolves under the other.
#[test]
fn two_hosts_naming_one_workspace_differently_derive_one_designator() {
    let shared = Sandbox::new();
    let notes = folder(&shared, "notes", &[("T-1", "title: Ship\n")]);
    let hosts: Vec<(Sandbox, LinearWorkspace, &str)> = ["alpha", "beta"]
        .into_iter()
        .map(|name| {
            let sandbox = Sandbox::new();
            let (plans, plans_workspace) = workspace(
                &sandbox,
                "org-shared-plans",
                json!([issue(SHARED_NATIVE, "Review", json!({}))]),
            );
            sandbox.project_document(&configured(&json!({
                "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
                name: {"plugin": "linear", "config": plans, "visibility": "private"},
            })));
            (sandbox, plans_workspace, name)
        })
        .collect();
    let (alpha, alpha_plans, _) = &hosts[0];
    let (beta, beta_plans, _) = &hosts[1];

    answered(
        alpha,
        &[
            "task",
            "update",
            "notes:T-1",
            "--depends-on",
            &format!("alpha:{SHARED_NATIVE}"),
            "--json",
        ],
        0,
    );
    let file = std::fs::read_to_string(notes.join("tasks/T-1.md")).expect("the task file");
    let expected = designator("linear", "org-shared-plans");
    assert!(file.contains(&expected), "written opaquely:\n{file}");
    assert!(
        !file.contains("org-shared-plans") && !file.contains("alpha"),
        "{file}"
    );
    assert_eq!(
        discovered(&sent(alpha_plans, 0)),
        1,
        "one request on a cold cache"
    );

    let read = answered(beta, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), format!("beta:{SHARED_NATIVE}"))]
    );
    let first = beta_plans.served().len();
    answered(beta, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        discovered(&sent(beta_plans, 0)),
        1,
        "one request per cache directory"
    );
    assert_eq!(sent(beta_plans, first), [format!("issue {SHARED_NATIVE}")]);
    for host in [alpha, beta] {
        let kept = kept_designators(&host.subdirectory("cache"));
        assert_eq!(kept.len(), 1, "one designator kept per source");
        assert!(kept[0].contains(&expected) && !kept[0].contains("org-shared-plans"));
    }
}

/// The native id of the item two hosts' sources name: an issue's id, as Linear spells one.
const SHARED_NATIVE: &str = "4b7d0c2e-8f3a-4e19-b6d5-2a9c71e04f68";

/// Every `designator.json` under `cache`, read.
fn kept_designators(cache: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![cache.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == "designator.json")
            {
                found.push(std::fs::read_to_string(path).expect("a kept designator"));
            }
        }
    }
    found
}

/// A folder of Markdown has no identifier of its own, so a `public_alias` names it: two
/// configurations naming one private folder differently, declaring the same alias, write and
/// resolve one opaque edge. An alias off the pattern is refused when the configuration loads.
#[test]
fn a_public_alias_names_a_folder_alike_under_every_name() {
    const ALIAS: &str = "planning_vault-2026";
    let sandbox = Sandbox::new();
    let notes = folder(&sandbox, "notes", &[("T-1", "title: Ship\n")]);
    let vault = folder(
        &sandbox,
        "vault",
        &[("approve-plan", "title: Approve the plan\n")],
    );
    let sources = |name: &str, alias: &str| {
        json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
            name: {"plugin": "local-md", "config": {"root": vault}, "visibility": "private",
                   "public_alias": alias},
        })
    };

    sandbox.project_document(&configured(&sources("vault", ALIAS)));
    answered(
        &sandbox,
        &[
            "task",
            "update",
            "notes:T-1",
            "--depends-on",
            "vault:approve-plan",
            "--json",
        ],
        0,
    );
    let file = std::fs::read_to_string(notes.join("tasks/T-1.md")).expect("the task file");
    assert!(file.contains(&format!("alias/{ALIAS}")), "{file}");
    assert!(
        !file.contains("vault:"),
        "the folder's name is not written: {file}"
    );

    sandbox.project_document(&configured(&sources("strongbox", ALIAS)));
    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "strongbox:approve-plan".to_owned())]
    );

    let mut numbered = sources("strongbox", ALIAS);
    numbered["strongbox"]["public_alias"] = json!(1_234_567_890_123_456_u64);
    sandbox.project_document(&configured(&numbered));
    let output = run(&sandbox, &["task", "deps", "notes:T-1"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("public_alias is a string")
            && !stderr(&output).contains("1234567890123456"),
        "{}",
        stderr(&output)
    );
    for refused in [
        "too-short",
        "has spaces in it, ok",
        "dots.are.not.in.the.pattern",
    ] {
        sandbox.project_document(&configured(&sources("strongbox", refused)));
        let output = run(&sandbox, &["task", "deps", "notes:T-1"]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{refused}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("public_alias"),
            "{}",
            stderr(&output)
        );
        assert!(
            !stderr(&output).contains(refused),
            "the value is not repeated"
        );
    }
}

/// The private Linear issues of the copy journey, by native ids as Linear spells them.
const DEPENDENT: &str = "1c7e5a90-3d2b-4f6e-8a11-5b9d0e2c4f73";
const PREREQUISITE: &str = "6a0f2d18-9b4c-4e57-a3d2-7e1b8c5f9a06";

/// A copy from a private Linear source onto a public folder writes the dependent's private
/// prerequisite on the dependent itself, opaquely; takes the legacy `depended_on_by` entry an
/// earlier copy left on the prerequisite off; and marks the dependent — whose next dependency
/// read then sends the private source no reverse-record query, only the read of the far end.
#[test]
fn a_copy_moves_a_legacy_private_record_onto_the_dependent_opaquely() {
    let sandbox = Sandbox::new();
    let site = folder(
        &sandbox,
        "site",
        &[(
            "D-1",
            &format!(
                "title: Draft the plan\nmetadata:\n  onetaskgraph.origin: vault:{DEPENDENT}\n  \
                 onetaskgraph.copied_destination_version: sha256:00\n"
            ),
        )],
    );
    let (vault, vault_workspace) = workspace(
        &sandbox,
        "org-vault",
        json!([
            issue(DEPENDENT, "Draft the plan", json!({})),
            issue(
                PREREQUISITE,
                "Approve the budget",
                json!({"metadata": {
                    "onetaskgraph.depended_on_by": {"site:D-1": {"id": "site:D-1", "kind": "task"}}
                }})
            ),
        ]),
    );
    vault_workspace.relate(DEPENDENT, PREREQUISITE, "blocks");
    sandbox.project_document(&configured(&json!({
        "site": {"plugin": "local-md", "config": {"root": site}, "visibility": "public"},
        "vault": {"plugin": "linear", "config": vault, "visibility": "private"},
    })));
    let prerequisite_id = format!("vault:{PREREQUISITE}");
    let before = answered(&sandbox, &["task", "deps", "site:D-1", "--json"], 0);
    assert_eq!(
        edges(&before),
        [("site:D-1".to_owned(), prerequisite_id.clone())],
        "the legacy record is read"
    );

    answered(
        &sandbox,
        &[
            "task",
            "copy",
            &format!("vault:{DEPENDENT}"),
            "--to",
            "site",
            "--json",
        ],
        0,
    );

    let file = std::fs::read_to_string(site.join("tasks/D-1.md")).expect("the dependent");
    let far = designator("linear", "org-vault");
    assert!(
        file.contains(&far) && file.contains(PREREQUISITE),
        "the prerequisite is on the dependent, opaquely:\n{file}"
    );
    assert!(file.contains("onetaskgraph.edges_on_record: 1"), "{file}");
    let prerequisite = vault_workspace
        .long_form("tasks", PREREQUISITE)
        .expect("the prerequisite");
    assert!(
        !prerequisite.contains("depended_on_by"),
        "the legacy entry is taken off:\n{prerequisite}"
    );

    let from = vault_workspace.served().len();
    let after = answered(&sandbox, &["task", "deps", "site:D-1", "--json"], 0);
    assert_eq!(edges(&after), [("site:D-1".to_owned(), prerequisite_id)]);
    assert_eq!(
        sent(&vault_workspace, from),
        [format!("issue {PREREQUISITE}")],
        "the far end read, and no reverse-record query"
    );
}

/// The digest the store names a remote by: SHA-256 hex of `<plugin>\n<identity>`.
fn remote(plugin: &str, identity: &str) -> String {
    Sha256::digest(format!("{plugin}\n{identity}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A kept designator that is unreadable, in another schema or about another remote is a miss:
/// the next read asks the source once more, and keeps its answer whole in its place.
#[test]
fn a_kept_designator_that_cannot_be_believed_costs_one_request_and_is_replaced() {
    let world = resolution();
    answered(&world.sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    let kept: Vec<PathBuf> = kept_paths(&world.sandbox.subdirectory("cache"));
    assert_eq!(kept.len(), 2, "one per source asked: {kept:?}");
    let good = std::fs::read_to_string(&kept[0]).expect("a kept designator");
    let mut other_remote: Value = serde_json::from_str(&good).expect("JSON");
    other_remote["remote"] = json!("another remote");
    let mut other_schema: Value = serde_json::from_str(&good).expect("JSON");
    other_schema["schema"] = json!(99);
    for (case, planted) in [
        ("unreadable", "not json at all".to_owned()),
        ("another remote's", other_remote.to_string()),
        ("another schema's", other_schema.to_string()),
    ] {
        for path in &kept {
            std::fs::write(path, &planted).expect("a planted file");
        }
        let (first, second) = (world.first.served().len(), world.second.served().len());
        let read = answered(&world.sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
        assert_eq!(
            edges(&read),
            [("notes:T-1".to_owned(), "bb-plans:L-9".to_owned())],
            "{case}"
        );
        assert_eq!(
            discovered(&sent(&world.first, first)),
            1,
            "{case}: asked once more"
        );
        assert_eq!(
            discovered(&sent(&world.second, second)),
            1,
            "{case}: asked once more"
        );
        for path in &kept {
            let replaced: Value =
                serde_json::from_str(&std::fs::read_to_string(path).expect("kept again"))
                    .expect("{case}: replaced whole");
            assert_eq!(replaced["schema"], 1, "{case}");
        }
    }
}

/// Every `designator.json` under `cache`.
fn kept_paths(cache: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![cache.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == "designator.json")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A designator that cannot be kept — its directory is under a file — still resolves the edge,
/// and the next process asks the source again rather than failing.
#[test]
fn a_designator_that_cannot_be_kept_is_asked_again_and_never_fails_the_read() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!("title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"),
        )],
    );
    let blocker = sandbox.subdirectory("blocked").join("a-file");
    std::fs::write(&blocker, b"in the way").expect("a file");
    let (plans, plans_workspace) = workspace(
        &sandbox,
        "org-plans",
        json!([issue("L-9", "Review", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": plans, "cache": {"dir": blocker.join("cache")}},
    })));
    for _ in 0..2 {
        let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
        assert_eq!(
            edges(&read),
            [("notes:T-1".to_owned(), "plans:L-9".to_owned())]
        );
    }
    assert_eq!(
        discovered(&sent(&plans_workspace, 0)),
        2,
        "asked by each process"
    );
}

/// An opaque far end naming a project resolves through the project read; one the source holds
/// no item by fails the read naming the designator rather than being drawn; and a candidate
/// that cannot say which tracker it is is named unanswered while discovery goes on to the next.
#[test]
fn an_opaque_project_resolves_a_missing_far_end_fails_and_a_mute_candidate_is_named() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[
            (
                "T-1",
                &format!(
                    "title: Ship\ndepends_on:\n  - {{designator: {far}, id: P-9, item: project}}\n"
                ),
            ),
            (
                "T-2",
                &format!(
                    "title: Land\ndepends_on:\n  - {{designator: {far}, id: L-404, item: task}}\n"
                ),
            ),
        ],
    );
    let (mute, mute_sent) = linear_rate_limited(&sandbox);
    let mut held = dataset();
    for (_, kept) in held.as_object_mut().expect("the dataset is an object") {
        *kept = json!([]);
    }
    held["projects"] = json!([{"id": "P-9", "title": "Roadmap",
                               "status": {"category": "todo", "name": "Planned"}, "labels": []}]);
    held["organization"] = json!("org-plans");
    let (plans, plans_workspace) = linear_workspace(&sandbox, held);
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "aa-mute": {"plugin": "linear", "config": mute},
        "bb-plans": {"plugin": "linear", "config": plans},
    })));

    let project = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        edges(&project),
        [("notes:T-1".to_owned(), "bb-plans:P-9".to_owned())]
    );
    assert_eq!(project["items"][0]["to"]["kind"], "project");
    assert_eq!(project["unanswered"][0]["source"], "aa-mute", "{project}");
    assert!(
        *mute_sent.lock().unwrap() >= 1,
        "the mute candidate was asked"
    );
    assert!(
        plans_workspace
            .served()
            .iter()
            .any(|(query, variables)| query == graphql::PROJECT && variables["id"] == "P-9"),
        "the far end read as a project"
    );

    let missing = answered(&sandbox, &["task", "deps", "notes:T-2", "--json"], 4);
    assert_eq!(edges(&missing), Vec::<(String, String)>::new());
    assert_eq!(missing["errors"][0]["source"], "bb-plans", "{missing}");
    assert!(
        missing["errors"][0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(&far) && message.contains("L-404")),
        "{missing}"
    );
}

/// A far end on a source whose designator this host keeps, but which cannot be asked, stays an
/// edge — its qualified id is known — and the source is named unanswered.
#[test]
fn a_far_end_whose_source_cannot_be_asked_stays_an_edge_and_names_the_source() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!("title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"),
        )],
    );
    let (mute, _) = linear_rate_limited(&sandbox);
    let kept = sandbox.subdirectory("kept");
    let identity = format!("{}\nFIX", mute["endpoint"].as_str().expect("an endpoint"));
    std::fs::write(
        kept.join("designator.json"),
        json!({"schema": 1, "remote": remote("linear", &identity), "designator": far}).to_string(),
    )
    .expect("a kept designator");
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": mute, "cache": {"dir": kept}},
    })));

    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "plans:L-9".to_owned())]
    );
    assert_eq!(read["unanswered"][0]["source"], "plans", "{read}");
    assert_eq!(
        read["unanswered"][0]["error"]["kind"], "rate-limited",
        "{read}"
    );
}

/// `project graph` draws an opaque far end as the source its designator is, and names one no
/// source is in its `unanswered`.
#[test]
fn a_project_graph_draws_a_resolved_opaque_far_end_and_names_an_unresolved_one() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let nowhere = designator("linear", "org-nowhere");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\nproject: P-1\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n  \
                 - {{designator: {nowhere}, id: L-1, item: task}}\n"
            ),
        )],
    );
    std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
    std::fs::write(notes.join("projects/P-1.md"), "---\ntitle: Launch\n---\n").expect("a project");
    let (plans, _) = workspace(
        &sandbox,
        "org-plans",
        json!([issue("L-9", "Review", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": plans},
    })));

    let graph = answered(
        &sandbox,
        &["project", "graph", "notes:P-1", "--format", "json"],
        0,
    );
    assert!(
        graph["edges"]
            .as_array()
            .expect("the edges")
            .contains(&json!({"from": "plans:L-9", "to": "notes:T-1"})),
        "{graph:#}"
    );
    assert_eq!(
        graph["unanswered"],
        json!([{"designator": nowhere, "reason": "not configured on this host"}]),
        "{graph:#}"
    );
}

/// `task create --depends-on` onto a public folder writes a private prerequisite opaquely, and
/// refuses one named by a prefixed key before anything is created.
#[test]
fn a_create_names_its_private_prerequisite_opaquely_or_not_at_all() {
    let (sandbox, _board, _vault) = boundary();
    let notes = folder(&sandbox, "notes", &[]);
    std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
    std::fs::write(notes.join("projects/P-1.md"), "---\ntitle: Launch\n---\n").expect("a project");
    let body = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&body, "the release notes").expect("a body");
    let mut sources: Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.project().join("onetaskgraph.yaml"))
            .expect("the document"),
    )
    .expect("JSON");
    sources["sources"]["notes"] =
        json!({"plugin": "local-md", "config": {"root": notes}, "visibility": "public"});
    sandbox.project_document(&sources.to_string());
    let create = |far: &str| {
        run(
            &sandbox,
            &[
                "task",
                "create",
                "notes",
                "--project",
                "P-1",
                "--title",
                "Publish notes",
                "--body-file",
                body.to_str().expect("a path"),
                "--depends-on",
                far,
            ],
        )
    };

    let refused = create(&format!("{PRIVATE_SOURCE}:{KEY}"));
    assert_ne!(refused.status.code(), Some(0));
    conveys_nothing("a refusal", &stderr(&refused));
    assert!(
        stderr(&refused).contains("a prefixed key"),
        "{}",
        stderr(&refused)
    );
    assert!(
        std::fs::read_dir(notes.join("tasks")).map_or(true, |mut entries| entries.next().is_none()),
        "nothing was created"
    );

    let created = create(&format!("{PRIVATE_SOURCE}:{NATIVE}"));
    assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));
    let id = stdout(&created).trim().to_owned();
    let native = id.strip_prefix("notes:").expect("a notes id");
    let file = std::fs::read_to_string(notes.join("tasks").join(format!("{native}.md")))
        .expect("the created task");
    assert!(
        file.contains(&designator("linear", "org-zephyr")) && file.contains(NATIVE),
        "{file}"
    );
    conveys_nothing("the created task", &file);
}

/// What the command line refuses of an endpoint object before anything is asked: one that is
/// not JSON, one that is no endpoint, and one whose id is not qualified.
#[test]
fn an_endpoint_object_that_is_no_endpoint_is_refused_by_the_command_line() {
    let (sandbox, board, _vault) = boundary();
    let before = board.served().len();
    for (given, kind) in [
        ("{not json", "invalid-dependency"),
        (r#"{"id": 7, "kind": "task"}"#, "invalid-dependency"),
        (r#"{"id": "B-2", "kind": "task"}"#, "invalid-dependency"),
    ] {
        let output = run(
            &sandbox,
            &[
                "task",
                "update",
                "board:B-1",
                "--depends-on",
                given,
                "--json",
            ],
        );
        assert_ne!(output.status.code(), Some(0), "{given}");
        let failure: Value = serde_json::from_str(&stdout(&output))
            .or_else(|_| serde_json::from_str(&stderr(&output)))
            .unwrap_or_else(|_| panic!("a failure document for {given}:\n{}", stderr(&output)));
        assert_eq!(failure["failure"]["kind"], kind, "{given}: {failure}");
    }
    assert_eq!(
        board.served().len(),
        before,
        "nothing was asked of the board"
    );
}

/// An opaque endpoint a caller gives is held to the test a qualified one is: accepted when its
/// tracker is configured and holds the item by that native id, refused when its id is a key,
/// and refused when no configured source is its tracker.
#[test]
fn a_caller_s_opaque_endpoint_is_confirmed_before_it_is_written() {
    let (sandbox, board, _vault) = boundary();
    let far = designator("linear", "org-zephyr");
    for (id, designated, refused) in [
        (KEY, far.as_str(), Some("a prefixed key")),
        (
            NATIVE,
            &designator("linear", "org-unknown") as &str,
            Some("no source this host"),
        ),
        (NATIVE, far.as_str(), None),
    ] {
        let given = json!({"designator": designated, "id": id, "kind": "task"}).to_string();
        let output = run(
            &sandbox,
            &[
                "task",
                "update",
                "board:B-1",
                "--depends-on",
                &given,
                "--json",
            ],
        );
        match refused {
            Some(form) => {
                assert_ne!(output.status.code(), Some(0), "{given}");
                assert!(
                    stdout(&output).contains(form) || stderr(&output).contains(form),
                    "{given}"
                );
                conveys_nothing(
                    "a refusal",
                    &format!("{}{}", stdout(&output), stderr(&output)),
                );
            }
            None => assert_eq!(output.status.code(), Some(0), "{}", stderr(&output)),
        }
    }
    let stored = board.long_form("tasks", "B-1").expect("the board's task");
    assert!(stored.contains(&far) && stored.contains(NATIVE), "{stored}");
}

/// A GitHub board's designator is the digest of its Project's node id, found with the one read
/// of the board's fields, and an opaque edge naming it resolves to the board's item.
#[test]
fn a_github_board_is_designated_by_its_project_node_id() {
    let sandbox = Sandbox::new();
    let (block, board) = github_projects_with_board(&sandbox);
    let far = designator("github-projects", "PVT-board");
    sandbox.project_document(&configured(&json!({
        "board": {"plugin": "github-projects", "config": block},
    })));
    let listed = answered(
        &sandbox,
        &["task", "list", "--source", "board", "--json"],
        0,
    );
    let item = listed["items"][0]["id"]
        .as_str()
        .expect("a board task")
        .strip_prefix("board:")
        .expect("a board id")
        .to_owned();
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\ndepends_on:\n  - {{designator: {far}, id: {item}, item: task}}\n"
            ),
        )],
    );
    sandbox.project_document(&configured(&json!({
        "board": {"plugin": "github-projects", "config": block},
        "notes": {"plugin": "local-md", "config": {"root": notes}},
    })));
    let before = board.served().len();
    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), format!("board:{item}"))]
    );
    let fields_reads = board
        .served()
        .into_iter()
        .skip(before)
        .filter(|(query, _)| query.contains("boardFields:repositoryOwner"))
        .count();
    assert_eq!(
        fields_reads, 1,
        "the designator costs the one read of the board's fields"
    );
}

/// A copy writes a private prerequisite opaquely only when its id is that item's native id: one
/// named by what reads as a prefixed key, and one its source holds nothing under, are withheld
/// as a source with no designator's are, and the dependent is not marked; one named by its
/// native id is written opaquely and the dependent is.
#[test]
fn a_copy_writes_opaquely_only_a_prerequisite_named_by_its_native_id() {
    const ALIAS: &str = "planning_vault-2026";
    let sandbox = Sandbox::new();
    let site = folder(&sandbox, "site", &[]);
    let vault = folder(
        &sandbox,
        "vault",
        &[
            ("keyed", "title: Keyed\ndepends_on: [ABC-12]\n"),
            (
                "ghostly",
                "title: Ghostly\ndepends_on: [{id: vault:ghost, item: task}]\n",
            ),
            ("native", "title: Native\ndepends_on: [approve-plan]\n"),
            ("ABC-12", "title: Covert keyed item\n"),
            ("approve-plan", "title: Approve the plan\n"),
        ],
    );
    sandbox.project_document(&configured(&json!({
        "site": {"plugin": "local-md", "config": {"root": site}, "visibility": "public"},
        "vault": {"plugin": "local-md", "config": {"root": vault}, "visibility": "private",
                  "public_alias": ALIAS},
    })));
    for (task, opaque) in [("keyed", false), ("ghostly", false), ("native", true)] {
        let report = answered(
            &sandbox,
            &[
                "task",
                "copy",
                &format!("vault:{task}"),
                "--to",
                "site",
                "--json",
            ],
            0,
        );
        let landed = report["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .strip_prefix("site:")
            .expect("a site id")
            .to_owned();
        let file = std::fs::read_to_string(site.join("tasks").join(format!("{landed}.md")))
            .expect("the copy");
        assert_eq!(
            file.contains(&format!("alias/{ALIAS}")),
            opaque,
            "{task}:\n{file}"
        );
        assert_eq!(file.contains("edges_on_record"), opaque, "{task}:\n{file}");
        assert!(
            !file.contains("ABC-12") && !file.contains("ghost"),
            "{task}:\n{file}"
        );
    }
}

/// Asked for every edge, a read whose opaque far end no configured source is is refused naming
/// the designator — `task deps` and `project graph` alike — rather than answered short.
#[test]
fn a_complete_read_refuses_an_opaque_far_end_no_source_is() {
    let sandbox = Sandbox::new();
    let nowhere = designator("linear", "org-nowhere");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\nproject: P-1\ndepends_on:\n  - {{designator: {nowhere}, id: L-1, item: task}}\n"
            ),
        )],
    );
    std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
    std::fs::write(notes.join("projects/P-1.md"), "---\ntitle: Launch\n---\n").expect("a project");
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
    })));
    for arguments in [
        vec!["task", "deps", "notes:T-1", "--complete", "--json"],
        vec![
            "project",
            "graph",
            "notes:P-1",
            "--complete",
            "--format",
            "json",
        ],
    ] {
        let output = run(&sandbox, &arguments);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{arguments:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(&nowhere) && stderr(&output).contains("--complete"),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
}

/// A task and a project sharing one native id are two far ends: a source holding the project
/// alone confirms the opaque edge to it and fails the one to a task of that id.
#[test]
fn a_task_and_a_project_sharing_an_id_are_confirmed_apart() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\ndepends_on:\n  - {{designator: {far}, id: X-0, item: project}}\n  \
                 - {{designator: {far}, id: X-0, item: task}}\n"
            ),
        )],
    );
    let mut held = dataset();
    for (_, kept) in held.as_object_mut().expect("the dataset is an object") {
        *kept = json!([]);
    }
    held["projects"] = json!([{"id": "X-0", "title": "Roadmap",
                               "status": {"category": "todo", "name": "Planned"}, "labels": []}]);
    held["organization"] = json!("org-plans");
    let (plans, _) = linear_workspace(&sandbox, held);
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": plans},
    })));

    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 4);
    assert_eq!(
        edges(&read),
        [("notes:T-1".to_owned(), "plans:X-0".to_owned())]
    );
    assert_eq!(read["items"][0]["to"]["kind"], "project");
    assert_eq!(read["errors"][0]["source"], "plans", "{read}");
}

/// A source whose designator this host keeps but which cannot be asked: `task deps --complete`
/// counts it a failure — exit 4, named in `errors` — rather than answering short.
#[test]
fn a_complete_read_counts_an_opaque_far_end_s_unanswering_source_a_failure() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!("title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"),
        )],
    );
    let (mute, _) = linear_rate_limited(&sandbox);
    let kept = sandbox.subdirectory("kept");
    let identity = format!("{}\nFIX", mute["endpoint"].as_str().expect("an endpoint"));
    std::fs::write(
        kept.join("designator.json"),
        json!({"schema": 1, "remote": remote("linear", &identity), "designator": far}).to_string(),
    )
    .expect("a kept designator");
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": mute, "cache": {"dir": kept}},
    })));

    let read = answered(
        &sandbox,
        &["task", "deps", "notes:T-1", "--complete", "--json"],
        4,
    );
    assert_eq!(read["errors"][0]["source"], "plans", "{read}");
    assert!(read.get("unanswered").is_none(), "{read}");
}

/// A project graph whose task records an opaque far end its source holds no item by fails, as
/// any failed read of the graph does, naming the source and the designator.
#[test]
fn a_project_graph_fails_on_an_opaque_far_end_its_source_does_not_hold() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "org-plans");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\nproject: P-1\ndepends_on:\n  - {{designator: {far}, id: L-404, item: task}}\n"
            ),
        )],
    );
    std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
    std::fs::write(notes.join("projects/P-1.md"), "---\ntitle: Launch\n---\n").expect("a project");
    let (plans, _) = workspace(
        &sandbox,
        "org-plans",
        json!([issue("L-9", "Review", json!({}))]),
    );
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "plans": {"plugin": "linear", "config": plans},
    })));

    let output = run(
        &sandbox,
        &["project", "graph", "notes:P-1", "--format", "json"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("plans") && stderr(&output).contains(&far),
        "{}",
        stderr(&output)
    );
}

/// A backend that answers a lookup with an item under another native id — as Linear answers an
/// issue looked up by its key — has not confirmed the id: a read drops and fails that far end,
/// and a write naming the item by that id is refused as not its native id.
#[test]
fn an_item_answered_under_another_native_id_is_not_confirmed() {
    let (sandbox, _board, vault) = boundary();
    let alias_of = "covert";
    let rewrite = |workspace: &LinearWorkspace| {
        workspace.bend_answers(
            move |query, variables| query == graphql::ISSUE && variables["id"] == alias_of,
            usize::MAX,
            |_, data| {
                data["issue"]["id"] = json!(NATIVE);
            },
        );
    };
    rewrite(&vault);
    let far = designator("linear", "org-zephyr");

    let given = json!({"designator": far, "id": alias_of, "kind": "task"}).to_string();
    let refused = run(
        &sandbox,
        &[
            "task",
            "update",
            "board:B-1",
            "--depends-on",
            &given,
            "--json",
        ],
    );
    assert_ne!(refused.status.code(), Some(0));
    let said = format!("{}{}", stdout(&refused), stderr(&refused));
    assert!(said.contains("not the item's native id"), "{said}");
    conveys_nothing("a refusal", &said);

    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\ndepends_on:\n  - {{designator: {far}, id: {alias_of}, item: task}}\n"
            ),
        )],
    );
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.project().join("onetaskgraph.yaml"))
            .expect("the document"),
    )
    .expect("JSON");
    document["sources"]["notes"] = json!({"plugin": "local-md", "config": {"root": notes}});
    sandbox.project_document(&document.to_string());
    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 4);
    assert_eq!(edges(&read), Vec::<(String, String)>::new());
    assert_eq!(read["errors"][0]["source"], PRIVATE_SOURCE, "{read}");
}

/// A Linear workspace answering an empty organization id has said nothing of which tracker it
/// is: it is named unanswered, and an opaque far end only it could have been stays unresolved.
#[test]
fn a_workspace_answering_an_empty_organization_id_designates_nothing() {
    let sandbox = Sandbox::new();
    let far = designator("linear", "");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!("title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"),
        )],
    );
    let (blank, blank_workspace) =
        workspace(&sandbox, "", json!([issue("L-9", "Review", json!({}))]));
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "blank": {"plugin": "linear", "config": blank},
    })));

    let read = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
    assert_eq!(edges(&read), Vec::<(String, String)>::new());
    let unanswered = read["unanswered"].as_array().expect("unanswered");
    assert!(
        unanswered
            .iter()
            .any(|entry| entry["source"] == "blank" && entry["error"]["kind"] == "malformed"),
        "{read}"
    );
    assert!(
        unanswered.iter().any(|entry| entry["designator"] == far),
        "{read}"
    );
    assert_eq!(
        sent(&blank_workspace, 0),
        ["organization"],
        "no far end read"
    );
}

/// A copy carries an opaque far end as it was recorded — designator, id and kind — and a recopy
/// after the far end is recorded under another designator, with the same id, writes the change
/// rather than reading it as unchanged.
#[test]
fn a_copy_carries_an_opaque_far_end_and_a_recopy_follows_its_designator() {
    let sandbox = Sandbox::new();
    let first = designator("linear", "org-first");
    let second = designator("linear", "org-second");
    let notes = folder(
        &sandbox,
        "notes",
        &[(
            "T-1",
            &format!(
                "title: Ship\ndepends_on:\n  - {{designator: {first}, id: L-9, item: project}}\n"
            ),
        )],
    );
    let site = folder(&sandbox, "site", &[]);
    sandbox.project_document(&configured(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "site": {"plugin": "local-md", "config": {"root": site}},
    })));
    let landed = |report: &Value| {
        let id = report["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .strip_prefix("site:")
            .expect("a site id")
            .to_owned();
        std::fs::read_to_string(site.join("tasks").join(format!("{id}.md"))).expect("the copy")
    };

    let report = answered(
        &sandbox,
        &["task", "copy", "notes:T-1", "--to", "site", "--json"],
        0,
    );
    let copy = landed(&report);
    assert!(
        copy.contains(&first) && copy.contains("L-9") && copy.contains("project"),
        "{copy}"
    );

    let source = notes.join("tasks/T-1.md");
    let text = std::fs::read_to_string(&source).expect("the source task");
    std::fs::write(&source, text.replace(&first, &second)).expect("the source rewritten");
    let again = answered(
        &sandbox,
        &["task", "copy", "notes:T-1", "--to", "site", "--json"],
        0,
    );
    assert_eq!(again["items"][0]["action"], "updated", "{again}");
    let copy = landed(&again);
    assert!(copy.contains(&second) && !copy.contains(&first), "{copy}");
}

/// Onto a GitHub board declared public, a dependency on a private Linear item is written into
/// the issue's own record opaquely, and reads back as that item.
#[test]
fn a_github_board_holds_a_private_prerequisite_opaquely_and_reads_it_back() {
    let (sandbox, _board, _vault) = boundary();
    let (block, fields) = github_projects_with_board(&sandbox);
    // Each fixture writes the one credential it needs; this store needs both.
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.project().join("onetaskgraph.yaml"))
            .expect("the document"),
    )
    .expect("JSON");
    document["sources"]["github"] =
        json!({"plugin": "github-projects", "config": block, "visibility": "public"});
    sandbox.project_document(&document.to_string());
    let listed = answered(
        &sandbox,
        &["task", "list", "--source", "github", "--json"],
        0,
    );
    let item = listed["items"][0]["id"]
        .as_str()
        .expect("a board task")
        .to_owned();

    let updated = answered(
        &sandbox,
        &[
            "task",
            "update",
            &item,
            "--depends-on",
            &format!("{PRIVATE_SOURCE}:{NATIVE}"),
            "--json",
        ],
        0,
    );
    conveys_nothing("the update's answer", &updated.to_string());
    // The issue's body, read as a person reads the issue on GitHub: its record holds the far end
    // as exactly the three keys, and nothing of the private item.
    let body = fields
        .body(item.strip_prefix("github:").expect("a board id"))
        .as_str()
        .expect("an issue body")
        .to_owned();
    let far = designator("linear", "org-zephyr");
    let slot = body
        .split("onetaskgraph.depends_on")
        .nth(1)
        .expect("the far end is recorded in the issue's own record");
    assert!(slot.contains(&far) && slot.contains(NATIVE), "{body}");
    conveys_nothing("the board's issue", &body);
    let read = answered(&sandbox, &["task", "deps", &item, "--json"], 0);
    assert!(
        edges(&read).contains(&(item.clone(), format!("{PRIVATE_SOURCE}:{NATIVE}"))),
        "{read}"
    );
}
