//! `task locate`, driven the way a user drives it, against what the `task copy` after it does.
//!
//! Every store here holds two public folders of Markdown, `drafts` and `mirror`, a private one,
//! `secrets`, a public folder `site` routing every private item to `board` — the loopback
//! GitHub board, declared private — and a Linear source `ledger` whose every request is counted
//! and refused. Its `write_policy` is the released onevcs boundary commands behind a recorder,
//! over an onevcs home holding synthetic identities: `hiddenco/quietharbor` private,
//! `openco/openwidget` public.
//!
//! Each journey asks `locate` first and then makes the copy it describes, and holds the two to
//! the same answer: the destination and route a copy lands in, the counterpart it updates, and
//! the refusal it makes — while `locate` writes nothing, runs no boundary check and reads no
//! destination's live visibility.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Output;
use std::sync::{Arc, Mutex};

use onetaskgraph_e2e_support::onevcs::{BoundaryCall, BoundaryCommand, BoundaryRecord};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::GitHubBoardFields;
use crate::onevcs::OnevcsHome;

/// A private repository, by the onevcs home's registration.
const PRIVATE_REPOSITORY: &str = "github.com/hiddenco/quietharbor";
/// A public repository, by the same.
const PUBLIC_REPOSITORY: &str = "github.com/openco/openwidget";
/// The repository the fixture board files an issue in when nothing else decides.
const CONFIGURED: &str = "github.com/nickderobertis/onetaskgraph";

/// One sandboxed store, its board and the record of its boundary commands.
struct Store {
    sandbox: Sandbox,
    home: OnevcsHome,
    record: BoundaryRecord,
    board: GitHubBoardFields,
    ledger: Arc<Mutex<usize>>,
}

/// What one command sent: its output, the board's documents with their variables, and the
/// boundary commands it ran.
struct Ran {
    output: Output,
    served: Vec<(String, Value)>,
    calls: Vec<BoundaryCall>,
}

impl Ran {
    /// The live-visibility reads the board answered: its Project's `public` field and an issue
    /// repository's REST read.
    fn visibility_reads(&self) -> usize {
        self.served
            .iter()
            .filter(|(document, _)| {
                document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY
                    || document.starts_with("GET /repos/")
            })
            .count()
    }

    fn mutations(&self) -> usize {
        self.served
            .iter()
            .filter(|(document, _)| document.trim_start().starts_with("mutation"))
            .count()
    }

    fn checks(&self) -> usize {
        self.calls
            .iter()
            .filter(|call| call.command == BoundaryCommand::Check)
            .count()
    }

    /// Every repository the store put to its `visibility_command`, sorted.
    fn inspected(&self) -> Vec<String> {
        let mut inspected: Vec<String> = self
            .calls
            .iter()
            .filter(|call| call.command == BoundaryCommand::Inspect)
            .map(|call| call.input["repository"].as_str().unwrap_or("").to_owned())
            .collect();
        inspected.sort();
        inspected
    }

    /// The document a `--json` run wrote, which had to exit 0.
    fn answer(&self) -> Value {
        assert_eq!(
            self.output.status.code(),
            Some(0),
            "{}{}",
            stdout(&self.output),
            stderr(&self.output)
        );
        serde_json::from_str(&stdout(&self.output)).expect("a JSON answer")
    }

    /// The exit code, failure kind and failure message of a `--json` run that was refused.
    fn refusal(&self) -> (Option<i32>, String, String) {
        let failure: Value = serde_json::from_str(&stdout(&self.output)).unwrap_or_else(|_| {
            panic!(
                "a refusal writes its failure document: {}{}",
                stdout(&self.output),
                stderr(&self.output)
            )
        });
        (
            self.output.status.code(),
            failure["failure"]["kind"]
                .as_str()
                .expect("a kind")
                .to_owned(),
            failure["failure"]["message"]
                .as_str()
                .expect("a message")
                .to_owned(),
        )
    }
}

impl Store {
    /// The store, with the board holding `items` at the start.
    fn new(items: Vec<Value>) -> Self {
        let sandbox = Sandbox::new();
        let mut home = OnevcsHome::new(&sandbox);
        home.private("hiddenco", "quietharbor")
            .public("openco", "openwidget");
        let record = home.recorded(&sandbox);
        let (board_config, board) = crate::fixtures::github_projects_with_items(&sandbox, items);
        let github_secrets =
            std::fs::read_to_string(sandbox.config_home().join("onetaskgraph/secrets.env"))
                .unwrap_or_default();
        let (ledger_config, ledger) = crate::fixtures::linear_rate_limited(&sandbox);
        // Both fixtures write the one secrets file; the store needs both credentials.
        sandbox.secrets_file(&format!("{github_secrets}LINEAR_API_KEY=fixture-key\n"));
        let folder = |name: &str, visibility: &str| {
            json!({
                "plugin": "local-md",
                "config": {"root": sandbox.subdirectory(name)},
                "visibility": visibility,
            })
        };
        let mut site = folder("site", "public");
        site["routes"] = json!([{"classification": "private", "to": "board"}]);
        let document = json!({
            "sources": {
                "drafts": folder("drafts", "public"),
                "mirror": folder("mirror", "public"),
                "secrets": folder("secrets", "private"),
                "site": site,
                "board": {"plugin": "github-projects", "config": board_config,
                          "visibility": "private"},
                "ledger": {"plugin": "linear", "config": ledger_config},
            },
            "write_policy": record.write_policy(),
        });
        sandbox.project_document(&document.to_string());
        Self {
            sandbox,
            home,
            record,
            board,
            ledger,
        }
    }

    /// Record a task in `folder`, with `front` as its front matter beside its title and status.
    fn task(&self, folder: &str, id: &str, front: &str) {
        self.record(
            folder,
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo\n{front}"),
        );
    }

    fn record(&self, folder: &str, kind: &str, id: &str, front: &str) {
        let path = self
            .sandbox
            .project()
            .join(folder)
            .join(kind)
            .join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the kind's folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }

    fn run(&self, arguments: &[&str]) -> Ran {
        let served_before = self.board.served().len();
        let calls_before = self.record.calls().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .env("ONEVCS_HOME", self.home.path())
            .assert()
            .get_output()
            .clone();
        Ran {
            output,
            served: self.board.served()[served_before..].to_vec(),
            calls: self.record.calls()[calls_before..].to_vec(),
        }
    }

    /// `task locate <ids> --to <to> <extra> --json`, held to having written nothing, run no
    /// check and read no live visibility.
    fn locate(&self, ids: &[&str], to: &str, extra: &[&str]) -> Ran {
        let before = self.tree();
        let mut arguments = vec!["task", "locate"];
        arguments.extend(ids);
        arguments.extend(["--to", to]);
        arguments.extend(extra);
        arguments.push("--json");
        let ran = self.run(&arguments);
        assert_eq!(self.tree(), before, "{arguments:?} wrote no file");
        assert_eq!(ran.mutations(), 0, "{arguments:?}: {:#?}", ran.served);
        assert_eq!(
            ran.checks(),
            0,
            "{arguments:?} ran no check: {:?}",
            ran.calls
        );
        assert_eq!(
            ran.visibility_reads(),
            0,
            "{arguments:?} read no live visibility: {:#?}",
            ran.served
        );
        ran
    }

    /// `task copy <id> --to <to> <extra> --json`.
    fn copy(&self, id: &str, to: &str, extra: &[&str]) -> Ran {
        let mut arguments = vec!["task", "copy", id, "--to", to];
        arguments.extend(extra);
        arguments.push("--json");
        self.run(&arguments)
    }

    /// Every file under every folder of the store, with its text.
    fn tree(&self) -> BTreeMap<String, String> {
        let mut held = BTreeMap::new();
        let mut pending = vec![self.sandbox.project()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("an entry").path();
                // The recorder's own log is the record of what the store ran, not the store.
                if path.ends_with("boundary-record") {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let bytes = std::fs::read(&path).expect("a file");
                    held.insert(display(&path), String::from_utf8_lossy(&bytes).into_owned());
                }
            }
        }
        held
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// One issue of the loopback board, in the public repository, recording `origin` as where it
/// was copied from — none when it is empty.
fn board_item(id: &str, origin: &str) -> Value {
    json!({"item": format!("ITEM-{id}"), "id": id, "type": "Issue", "title": id,
           "body": "Held on the board.", "state": "OPEN", "reason": null, "parent": null,
           "repo": "openco/openwidget", "status": "Todo", "origin": origin, "labels": []})
}

/// The one entry of a locate answer.
fn only(answer: &Value) -> &Value {
    let items = answer["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{answer:#}");
    &items[0]
}

/// The one outcome of a copy report.
fn outcome(report: &Value) -> &Value {
    let items = report["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{report:#}");
    &items[0]
}

#[test]
fn locate_places_each_task_where_the_copy_after_it_lands_it_and_writes_nothing() {
    let store = Store::new(Vec::new());
    store.task(
        "drafts",
        "open",
        &format!("repositories: [{PUBLIC_REPOSITORY}]"),
    );
    store.task("drafts", "marked", "classification: private");
    store.task(
        "drafts",
        "named",
        &format!("repositories: [{PRIVATE_REPOSITORY}, {PUBLIC_REPOSITORY}]"),
    );
    store.task("secrets", "own", "");
    // A public task stays on `site`; each private one — by its record, and by a repository the
    // policy answers private — goes to `board` by route 0.
    for (id, classification, destination, route, inspected) in [
        (
            "drafts:open",
            "public",
            "site",
            Value::Null,
            vec![PUBLIC_REPOSITORY],
        ),
        ("drafts:marked", "private", "board", json!(0), vec![]),
        (
            "drafts:named",
            "private",
            "board",
            json!(0),
            // The first repository the policy answers private settles it.
            vec![PRIVATE_REPOSITORY],
        ),
        // A source declared private withholds its tasks' provenance and does not classify them:
        // a task there recording no classification and naming no repository is public, for a
        // copy and for locate alike.
        ("secrets:own", "public", "site", Value::Null, vec![]),
    ] {
        let located = store.locate(&[id], "site", &[]);
        let item = only(&located.answer()).clone();
        assert_eq!(item["source"], id, "{item:#}");
        assert_eq!(item["classification"], classification, "{id}: {item:#}");
        assert_eq!(item["destination"], destination, "{id}: {item:#}");
        assert_eq!(item["route"], route, "{id}: {item:#}");
        assert_eq!(item["counterpart"], Value::Null, "{id}: never copied");
        assert_eq!(item["carriers"], json!([]), "{id}: never copied");
        let creates_in = if destination == "board" {
            json!({"repository": CONFIGURED})
        } else {
            Value::Null
        };
        assert_eq!(item["creates_in"], creates_in, "{id}: {item:#}");

        let copied = store.copy(id, "site", &[]);
        let report = copied.answer();
        assert_eq!(
            outcome(&report)["placed"],
            json!({"destination": destination, "route": route}),
            "{id}: the copy lands where locate said: {report:#}"
        );
        // The repositories the copy's classification asked about, and only those.
        let mut asked = copied.inspected();
        asked.dedup();
        assert_eq!(located.inspected(), asked, "{id}");
        let expected: Vec<String> = inspected
            .iter()
            .map(|repository| (*repository).to_owned())
            .collect();
        assert_eq!(located.inspected(), expected, "{id}");
        // What the copy does that locate does not: its check, and the board's live visibility.
        if destination == "board" {
            assert!(copied.visibility_reads() > 0, "{id}: {:#?}", copied.served);
        } else {
            assert!(copied.checks() > 0, "{id}: {:?}", copied.calls);
        }
    }

    // After the first copy: the public landing is found by the link `drafts:open` now records,
    // the private one by the board's origin query, because no link naming a private landing is
    // written on a public source. Each time the next copy updates exactly that item.
    for (id, via) in [("drafts:open", "link"), ("drafts:marked", "scan")] {
        let item = only(&store.locate(&[id], "site", &[]).answer()).clone();
        let counterpart = item["counterpart"]["id"].clone();
        assert!(counterpart.is_string(), "{id}: {item:#}");
        assert_eq!(item["counterpart"]["via"], via, "{id}: {item:#}");
        let carriers = if via == "scan" {
            json!([counterpart.clone()])
        } else {
            json!([])
        };
        assert_eq!(item["carriers"], carriers, "{id}: {item:#}");
        // Edited first, so the copy writes it rather than finding it unchanged.
        let path = store
            .sandbox
            .project()
            .join(id.replace(':', "/tasks/") + ".md");
        let text = std::fs::read_to_string(&path).expect("the task");
        std::fs::write(&path, text.replace("Body of", "Edited body of")).expect("the edit");
        let report = store.copy(id, "site", &[]).answer();
        assert_eq!(outcome(&report)["action"], "updated", "{id}: {report:#}");
        assert_eq!(
            outcome(&report)["destination"],
            counterpart,
            "{id}: {report:#}"
        );
        assert_eq!(outcome(&report)["via"], via, "{id}: {report:#}");
    }

    // The same facts for a person.
    let located = store.run(&["task", "locate", "drafts:marked", "--to", "site"]);
    assert_eq!(
        located.output.status.code(),
        Some(0),
        "{}",
        stderr(&located.output)
    );
    let text = stdout(&located.output);
    for expected in [
        "task:",
        "drafts:marked",
        "classification:",
        "private",
        "board by route 0",
        "via scan",
        "creates in:",
        CONFIGURED,
    ] {
        assert!(text.contains(expected), "{expected} in:\n{text}");
    }
}

#[test]
fn locate_lists_every_carrier_across_pages_and_finds_a_copy_back_by_its_origin() {
    let store = Store::new(vec![
        board_item("TWIN-1", "drafts:twin"),
        board_item("TWIN-2", "drafts:twin"),
        board_item("BACK-1", ""),
    ]);
    // One row a page, fewer than the two carriers: the origin query is read to its last page.
    store.board.answer_search_pages_of(1);
    store.task("drafts", "twin", "classification: private");
    store.task(
        "drafts",
        "back",
        "classification: private\nmetadata: {onetaskgraph.origin: \"board:BACK-1\"}",
    );

    let answer = store.locate(&["drafts:twin"], "site", &[]).answer();
    // The board meters its own requests, and the answer says what they were.
    assert!(
        answer["spent"]["requests"]
            .as_u64()
            .is_some_and(|sent| sent > 0),
        "{answer:#}"
    );
    let item = only(&answer).clone();
    assert_eq!(
        item["carriers"],
        json!(["board:TWIN-1", "board:TWIN-2"]),
        "{item:#}"
    );
    assert_eq!(
        item["counterpart"],
        json!({"id": "board:TWIN-1", "via": "scan"})
    );
    let report = store.copy("drafts:twin", "site", &[]).answer();
    assert_eq!(
        outcome(&report)["destination"],
        "board:TWIN-1",
        "{report:#}"
    );
    assert_eq!(outcome(&report)["action"], "updated", "{report:#}");

    // A task whose own origin names a board item is a copy-back, found by that origin.
    let item = only(&store.locate(&["drafts:back"], "site", &[]).answer()).clone();
    assert_eq!(
        item["counterpart"],
        json!({"id": "board:BACK-1", "via": "origin"})
    );
    assert_eq!(
        item["carriers"],
        json!([]),
        "the origin query did not run: {item:#}"
    );
    let report = store.copy("drafts:back", "site", &[]).answer();
    assert_eq!(
        outcome(&report)["destination"],
        "board:BACK-1",
        "{report:#}"
    );
    assert_eq!(outcome(&report)["via"], "origin", "{report:#}");

    // `--no-counterpart` sends the board nothing at all for a task in no project, and the
    // answer names neither a counterpart nor carriers.
    let located = store.locate(&["drafts:twin"], "site", &["--no-counterpart"]);
    assert!(located.served.is_empty(), "{:#?}", located.served);
    let item = only(&located.answer()).clone();
    assert_eq!(item["destination"], "board", "{item:#}");
    assert_eq!(
        item["creates_in"],
        json!({"repository": CONFIGURED}),
        "{item:#}"
    );
    assert!(
        item.get("counterpart").is_none() && item.get("carriers").is_none(),
        "{item:#}"
    );

    // A Linear destination and a folder of Markdown file no issue in a repository, and a
    // `--no-counterpart` locate asks Linear nothing.
    store.task("drafts", "plain", "");
    for to in ["ledger", "mirror"] {
        let item = only(
            &store
                .locate(&["drafts:plain"], to, &["--no-counterpart"])
                .answer(),
        )
        .clone();
        assert_eq!(item["creates_in"], Value::Null, "{to}: {item:#}");
        assert_eq!(item["destination"], to, "{to}: {item:#}");
    }
    assert_eq!(*store.ledger.lock().unwrap(), 0, "Linear was never asked");

    // For a person: no route, nothing found, no carrier and no repository each said in words,
    // and a lookup not made not mentioned at all.
    let text = |extra: &[&str]| {
        let mut arguments = vec!["task", "locate", "drafts:plain", "--to", "mirror"];
        arguments.extend(extra);
        let ran = store.run(&arguments);
        assert_eq!(ran.output.status.code(), Some(0), "{}", stderr(&ran.output));
        stdout(&ran.output)
    };
    let looked = text(&[]);
    for expected in [
        "mirror by no route",
        "none: a copy would create one",
        "carriers:",
        "creates in:",
        "no repository",
    ] {
        assert!(looked.contains(expected), "{expected} in:\n{looked}");
    }
    let placed = text(&["--no-counterpart"]);
    assert!(
        placed.contains("mirror by no route") && placed.contains("no repository"),
        "{placed}"
    );
    assert!(
        !placed.contains("counterpart:") && !placed.contains("carriers:"),
        "{placed}"
    );
}

#[test]
fn locate_refuses_each_item_a_copy_of_it_refuses_with_the_copy_s_own_failure() {
    let store = Store::new(vec![board_item("MOVED-1", "drafts:moved")]);
    // A link naming an item `site` no longer holds.
    store.task(
        "drafts",
        "stale",
        "metadata: {onetaskgraph.copies: {site: \"site:gone\"}}",
    );
    // A public task whose counterpart sits on `board`, where a route could have sent it, while
    // it routes to `site` now.
    store.task(
        "drafts",
        "moved",
        "metadata: {onetaskgraph.copies: {board: \"board:MOVED-1\"}}",
    );
    // A private task bound for `mirror`, which is not declared private.
    store.task("drafts", "marked", "classification: private");
    // A public task whose counterpart on `mirror` is private: what it would update is private,
    // and `mirror` is not declared so.
    store.task(
        "drafts",
        "kept",
        "metadata: {onetaskgraph.copies: {mirror: \"mirror:held\"}}",
    );
    store.task(
        "mirror",
        "held",
        "classification: private\nmetadata: {onetaskgraph.origin: \"drafts:kept\"}",
    );
    for (id, to, kind) in [
        ("drafts:stale", "site", "stale-link"),
        ("drafts:moved", "site", "misrouted"),
        ("drafts:marked", "mirror", "not-private-destination"),
        ("drafts:kept", "mirror", "not-private-destination"),
    ] {
        let located = store.locate(&[id], to, &[]).refusal();
        let before = store.tree();
        let copied = store.copy(id, to, &[]);
        assert_eq!(store.tree(), before, "{id}: the copy was refused too");
        assert_eq!(located.1, kind, "{id}: {located:?}");
        assert_eq!(
            located,
            copied.refusal(),
            "{id}: locate refuses as the copy does"
        );
        assert_eq!(located.0, Some(1), "{id}");
    }
    // A destination that cannot be a source's name is refused naming the flag.
    let (code, kind, message) = store.locate(&["drafts:stale"], "not a name", &[]).refusal();
    assert_eq!(
        (code, kind.as_str()),
        (Some(1), "invalid-source-name"),
        "{message}"
    );
    assert!(message.contains("--to not a name"), "{message}");
    // A project and a document are no task to locate.
    store.record("drafts", "projects", "goal", "title: Goal\nstatus: todo");
    store.record("drafts", "documents", "notes", "title: Notes");
    for (id, named) in [("drafts:goal", "project"), ("drafts:notes", "document")] {
        let (code, kind, message) = store.locate(&[id], "site", &[]).refusal();
        assert_eq!((code, kind.as_str()), (Some(1), "not-a-task"), "{message}");
        assert!(
            message.contains(id) && message.contains(&format!("is a {named}")),
            "{message}"
        );
    }
}

#[test]
fn target_native_locates_the_counterpart_the_caller_names_and_nothing_else() {
    let store = Store::new(vec![
        board_item("PIN-1", "drafts:pinned"),
        board_item("PIN-2", "drafts:pinned"),
        board_item("DUP-1", ""),
        board_item("SP-1", "drafts:stalepin"),
    ]);
    // One task's link names a withdrawn duplicate, another's an item the board no longer holds.
    store.task(
        "drafts",
        "pinned",
        "metadata: {onetaskgraph.copies: {board: \"board:DUP-1\"}}",
    );
    store.task(
        "drafts",
        "stalepin",
        "metadata: {onetaskgraph.copies: {board: \"board:GONE\"}}",
    );
    let (_, kind, _) = store.locate(&["drafts:stalepin"], "board", &[]).refusal();
    assert_eq!(
        kind, "stale-link",
        "without --target-native the stale link refuses"
    );

    for (id, native) in [("drafts:pinned", "PIN-2"), ("drafts:stalepin", "SP-1")] {
        let located = store.locate(&[id], "board", &["--target-native", native]);
        let item = only(&located.answer()).clone();
        let expected = format!("board:{native}");
        assert_eq!(
            item["counterpart"],
            json!({"id": expected, "via": "target-task"}),
            "{id}"
        );
        assert_eq!(item["carriers"], json!([]), "{id}: {item:#}");
        // No destination read but the named item's: no link, no origin, no search.
        for (document, variables) in &located.served {
            let sent = format!("{document}{variables}");
            assert!(
                !sent.contains("DUP-1")
                    && !sent.contains("GONE-1")
                    && variables.get("search").is_none(),
                "{id}: {document} {variables}"
            );
        }
        assert!(!located.served.is_empty(), "{id}: the named item was read");
        let report = store
            .copy(id, "board", &["--target-task", &expected])
            .answer();
        assert_eq!(
            outcome(&report)["destination"],
            json!(expected),
            "{report:#}"
        );
        assert_eq!(outcome(&report)["action"], "updated", "{report:#}");
        assert_eq!(outcome(&report)["via"], "target-task", "{report:#}");
    }

    // An item the board does not hold, and one not naming the task back, are refused as
    // `task copy --target-task` refuses them.
    for native in ["NOPE-1", "DUP-1"] {
        let located = store
            .locate(&["drafts:pinned"], "board", &["--target-native", native])
            .refusal();
        let copied = store
            .copy(
                "drafts:pinned",
                "board",
                &["--target-task", &format!("board:{native}")],
            )
            .refusal();
        assert_eq!(located.0, copied.0, "{native}");
        assert_eq!(located.1, copied.1, "{native}");
    }

    // Beside two ids, or beside `--no-counterpart`, it is refused before anything is read.
    for (ids, extra, kind) in [
        (
            &["drafts:pinned", "drafts:stalepin"][..],
            &["--target-native", "PIN-2"][..],
            "target-native-needs-one",
        ),
        (
            &["drafts:pinned"],
            &["--target-native", "PIN-2", "--no-counterpart"],
            "target-native-with",
        ),
    ] {
        let located = store.locate(ids, "board", extra);
        assert!(located.served.is_empty(), "{kind}: {:#?}", located.served);
        let (code, refused, _) = located.refusal();
        assert_eq!((code, refused.as_str()), (Some(1), kind));
    }
}
