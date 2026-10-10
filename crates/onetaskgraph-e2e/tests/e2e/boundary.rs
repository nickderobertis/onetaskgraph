//! The public boundary, driven the way a user drives it: classification, declared visibility,
//! classification routes and the `write_policy` commands, through the binary.
//!
//! Every store here is folders of Markdown — `plan` and `vault` declared private, `site`
//! declared public. Most run the released onevcs boundary commands as their `write_policy`,
//! against an onevcs home this run owns, holding synthetic identities alone; the journeys about
//! what a store hands its policy, or does with a policy that fails, configure a small Python
//! command of their own instead:
//!
//! - `hiddenco/quietharbor` (A) and `lanternco/brightwater` (B), registered private;
//! - `openco/openwidget`, registered public;
//! - `brokenco/crackedbell`, registered private with a committed manifest that does not parse.
//!
//! Each test spawns the compiled binary and asserts on its exit code, its streams and the
//! folders it wrote — a refused write is held to having written nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::onevcs::OnevcsHome;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};

/// A private repository, its owner and name as a term check matches them.
const A: &str = "github.com/hiddenco/quietharbor";
const A_TEXT: &str = "Mirror the retry loop hiddenco/quietharbor uses.";
/// A second private repository, under another owner.
const B: &str = "github.com/lanternco/brightwater";
const B_TEXT: &str = "Mirror the retry loop lanternco/brightwater uses.";
/// A public repository.
const OPEN: &str = "github.com/openco/openwidget";
/// Text naming nothing private.
const PLAIN: &str = "Retry with a capped exponential backoff.";

/// One sandboxed store: its folders, and the onevcs home its policy asks.
struct Store {
    sandbox: Sandbox,
    onevcs: Option<OnevcsHome>,
}

impl Store {
    /// The folders `plan` (private), `vault` (private) and `site` (public), plus `extra`
    /// laid over them, under the policy `onevcs` answers — none at all when it is `None`.
    fn new(onevcs: Option<fn(&mut OnevcsHome)>, extra: Value) -> Self {
        Self::laid(onevcs, |_| extra)
    }

    /// [`Store::new`], with `extra` built against the sandbox — so a journey can add a folder
    /// of its own.
    fn laid(onevcs: Option<fn(&mut OnevcsHome)>, extra: impl FnOnce(&Sandbox) -> Value) -> Self {
        let sandbox = Sandbox::new();
        let extra = extra(&sandbox);
        let onevcs = onevcs.map(|register| {
            let mut home = OnevcsHome::new(&sandbox);
            register(&mut home);
            home
        });
        let policy = onevcs.as_ref().map(OnevcsHome::write_policy);
        Self::configured(sandbox, onevcs, policy, extra)
    }

    /// The same folders under exactly `policy` — none at all when it is `None` — with no
    /// onevcs home behind it.
    fn with_policy(policy: Option<Value>) -> Self {
        Self::configured(Sandbox::new(), None, policy, json!({}))
    }

    fn configured(
        sandbox: Sandbox,
        onevcs: Option<OnevcsHome>,
        policy: Option<Value>,
        extra: Value,
    ) -> Self {
        let mut sources = json!({
            "plan": folder(&sandbox, "plan", "private"),
            "vault": folder(&sandbox, "vault", "private"),
            "site": folder(&sandbox, "site", "public"),
        });
        merge(&mut sources, extra);
        let mut document = json!({ "sources": sources });
        if let Some(policy) = policy {
            document["write_policy"] = policy;
        }
        sandbox.project_document(&document.to_string());
        Self { sandbox, onevcs }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        if let Some(home) = &self.onevcs {
            command.env("ONEVCS_HOME", home.path());
        }
        command.assert().get_output().clone()
    }

    /// Standard output of a run that had to succeed.
    fn ok(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` exited {:?}\n{}{}",
            arguments.join(" "),
            output.status.code(),
            stdout(&output),
            stderr(&output)
        );
        stdout(&output)
    }

    /// The failure kind of a run that had to fail, read off its `--json` failure document,
    /// with its stderr.
    fn refused(&self, arguments: &[&str]) -> (String, String) {
        let mut with_json = arguments.to_vec();
        with_json.push("--json");
        let output = self.run(&with_json);
        assert_eq!(
            output.status.code(),
            Some(1),
            "`onetaskgraph {}` was expected to be refused\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        let failure: Value = serde_json::from_str(&stdout(&output)).unwrap_or_else(|problem| {
            panic!(
                "a refusal writes its failure document ({problem}): {}",
                stdout(&output)
            )
        });
        (
            failure["failure"]["kind"]
                .as_str()
                .expect("a failure kind")
                .to_owned(),
            stderr(&output),
        )
    }

    fn folder(&self, name: &str) -> PathBuf {
        self.sandbox.project().join(name)
    }

    /// Every file under every folder of this store's project — each source's, a journey's own
    /// added ones included — with its bytes.
    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        let mut held = BTreeMap::new();
        for entry in std::fs::read_dir(self.sandbox.project()).expect("the project directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                held.extend(tree(&path));
            }
        }
        held
    }

    fn record(&self, folder: &str, kind: &str, id: &str, front: &str, body: &str) {
        let path = self.folder(folder).join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the kind's folder");
        std::fs::write(&path, format!("---\n{front}\n---\n{body}\n")).expect("a record");
    }

    /// Record the public project `p` on `site`, which a create there names — while the boundary
    /// is active, a project its source does not hold is no project to file under.
    fn public_project_p(&self) {
        self.record("site", "projects", "p", "title: P\nstatus: todo", PLAIN);
    }

    /// A body file holding `text`, for a create.
    fn body(&self, text: &str) -> String {
        let path = self
            .sandbox
            .config_home()
            .join(format!("body-{}.md", text.len()));
        std::fs::write(&path, text).expect("a body file");
        path.display().to_string()
    }
}

/// One local-md folder declared `visibility`.
fn folder(sandbox: &Sandbox, name: &str, visibility: &str) -> Value {
    json!({
        "plugin": "local-md",
        "config": {"root": sandbox.subdirectory(name)},
        "visibility": visibility,
    })
}

/// `extra`'s keys laid over `base`'s, one level down, so a journey can add a source or a key of
/// one.
fn merge(base: &mut Value, extra: Value) {
    if let Value::Object(extra) = extra {
        for (name, value) in extra {
            match (base.get_mut(&name), value) {
                (Some(Value::Object(held)), Value::Object(laid)) => held.extend(laid),
                (_, value) => {
                    base[&name] = value;
                }
            }
        }
    }
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut held = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                held.insert(
                    path.display().to_string(),
                    std::fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    held
}

fn registered(home: &mut OnevcsHome) {
    home.private("hiddenco", "quietharbor")
        .private("lanternco", "brightwater")
        .public("openco", "openwidget");
}

fn files_under(store: &Store, folder: &str) -> Vec<String> {
    tree(&store.folder(folder)).into_keys().collect()
}

/// Assert `site` holds nothing but the project [`Store::public_project_p`] recorded, naming
/// every file it does hold otherwise. The path is built the way the record was, so it is spelled
/// with the platform's separator.
fn assert_site_holds_only_p(store: &Store) {
    let p = store
        .folder("site")
        .join("projects")
        .join("p.md")
        .display()
        .to_string();
    assert_eq!(files_under(store, "site"), vec![p], "site holds only p");
}

fn one_file(store: &Store, folder: &str, kind: &str) -> String {
    let files: Vec<String> = files_under(store, folder)
        .into_iter()
        .filter(|path| path.contains(kind))
        .collect();
    assert_eq!(files.len(), 1, "one {kind} file in {folder}: {files:?}");
    std::fs::read_to_string(&files[0]).expect("a readable record")
}

#[test]
fn an_inactive_store_still_refuses_an_explicitly_private_item_anywhere_not_declared_private() {
    // No write_policy and no declared visibility: the boundary is not active, and repository
    // visibility is never consulted — but a private item still has nowhere to go.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    let notes = sandbox.subdirectory("notes");
    sandbox.project_document(
        &json!({"sources": {
            "plan": {"plugin": "local-md", "config": {"root": plan}},
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        }})
        .to_string(),
    );
    let store = Store {
        sandbox,
        onevcs: None,
    };
    store.record("plan", "projects", "goal", "title: Goal\nstatus: todo", "");
    store.record(
        "plan",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nproject: goal\nclassification: private",
        PLAIN,
    );
    // A task naming a repository no policy has answered for is not private while inactive.
    store.record(
        "plan",
        "tasks",
        "named",
        &format!("title: Named\nstatus: todo\nrepositories: [{A}]"),
        PLAIN,
    );
    let body = store.body(PLAIN);
    let before = tree(&store.folder("notes"));
    for arguments in [
        vec!["task", "copy", "plan:secret", "--to", "notes"],
        vec![
            "task",
            "create",
            "notes",
            "--project",
            "p",
            "--title",
            "T",
            "--body-file",
            &body,
            "--classification",
            "private",
        ],
        vec![
            "document",
            "create",
            "notes",
            "--project",
            "p",
            "--title",
            "D",
            "--body-file",
            &body,
            "--classification",
            "private",
        ],
        vec![
            "project",
            "create",
            "notes",
            "--id",
            "q",
            "--title",
            "Q",
            "--body-file",
            &body,
            "--classification",
            "private",
        ],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "not-private-destination", "{said}");
        assert!(
            said.contains("is private") && said.contains("not declared private"),
            "{said}"
        );
    }
    assert_eq!(tree(&store.folder("notes")), before, "nothing was written");
    // Everything else writes exactly as before the boundary existed.
    store.ok(&["task", "copy", "plan:named", "--to", "notes"]);
    let landed = one_file(&store, "notes", "tasks");
    assert!(!landed.contains("classification"), "{landed}");
    // Learning a project's classification is a read, which an inactive store does not spend:
    // a task created public under a project only its own record calls private is written
    // public, and is held to that project's classification only once the store opts in.
    store.record(
        "notes",
        "projects",
        "hand-made",
        "title: Hand-made\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "notes",
        "--project",
        "hand-made",
        "--title",
        "Filed",
        "--body-file",
        &body,
        "--json",
    ]))
    .expect("a created task");
    // Public is the classification the wire leaves out.
    assert!(
        created["items"][0]["item"].get("classification").is_none(),
        "{created:#}"
    );
}

#[test]
fn a_task_of_a_private_or_unknown_repository_is_refused_onto_a_public_source() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "plan",
        "tasks",
        "private-repo",
        &format!("title: Private repo work\nstatus: todo\nrepositories: [{A}]"),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "unknown-repo",
        "title: Unregistered repo work\nstatus: todo\n\
         repositories: [github.com/nobodyco/unregistered]",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "public-repo",
        &format!("title: Open work\nstatus: todo\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let before = store.tree();
    for task in ["plan:private-repo", "plan:unknown-repo"] {
        let (kind, said) = store.refused(&["task", "copy", task, "--to", "site"]);
        assert_eq!(kind, "not-private-destination", "{said}");
    }
    let body = store.body(PLAIN);
    let (kind, _) = store.refused(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "T",
        "--body-file",
        &body,
        "--repository",
        A,
    ]);
    // Private by its repository, under the public project `p`: a public project never holds a
    // private member, so that is what refuses it first.
    assert_eq!(kind, "private-member");
    assert_eq!(store.tree(), before, "no refused write left anything");
    // A public repository's task, naming nothing private, is written.
    store.ok(&["task", "copy", "plan:public-repo", "--to", "site"]);
    // And a private repository's task is written to a private source.
    store.ok(&["task", "copy", "plan:private-repo", "--to", "vault"]);
    let vaulted = one_file(&store, "vault", "tasks");
    assert!(vaulted.contains("classification: private"), "{vaulted}");
}

#[test]
fn a_public_write_naming_a_private_repository_in_its_text_title_or_metadata_is_refused_neutrally() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "plan",
        "tasks",
        "leaks",
        "title: Leaks\nstatus: todo",
        A_TEXT,
    );
    store.record(
        "plan",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo",
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["task", "copy", "plan:leaks", "--to", "site"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(
        said.contains("term of a private repository in its text") && !said.contains("quietharbor"),
        "the refusal is neutral: it says where, never what: {said}"
    );
    // A title is checked as well as the body, and so is a metadata value.
    let body = store.body(PLAIN);
    let (kind, _) = store.refused(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Port hiddenco/quietharbor",
        "--body-file",
        &body,
    ]);
    assert_eq!(kind, "boundary-refused");
    let (kind, _) = store.refused(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "T",
        "--body-file",
        &body,
        "--metadata",
        "team.note=\"see hiddenco/quietharbor\"",
    ]);
    assert_eq!(kind, "boundary-refused");
    assert_eq!(store.tree(), before, "no refused write left anything");
    store.ok(&["task", "copy", "plan:clean", "--to", "site"]);
}

#[test]
fn a_missing_or_absent_policy_never_approves_a_public_write() {
    // A write_policy naming only a visibility command, an empty one, and none at all beside a
    // declared visibility: each is an active store whose check is missing.
    for policy in [
        Some(json!({"visibility_command": ["onetaskgraph-no-such-visibility-command"]})),
        Some(json!({})),
        None,
    ] {
        let store = Store::with_policy(policy.clone());
        store.record(
            "plan",
            "tasks",
            "clean",
            "title: Clean\nstatus: todo",
            PLAIN,
        );
        let (kind, said) = store.refused(&["task", "copy", "plan:clean", "--to", "site"]);
        assert_eq!(kind, "boundary-unavailable", "{policy:?}: {said}");
        assert!(said.contains("check_command"), "{said}");
        assert!(files_under(&store, "site").is_empty());
        // A private destination needs no check, so the same write lands there.
        store.ok(&["task", "copy", "plan:clean", "--to", "vault"]);
    }
}

#[test]
fn a_check_command_that_fails_or_answers_out_of_schema_is_unavailable_never_a_pass() {
    for command in [
        // Exits 0 having answered nothing: not an explicit pass.
        json!(["git", "--version"]),
        // A program that does not exist.
        json!(["onetaskgraph-no-such-check-command"]),
    ] {
        let store = Store::with_policy(Some(json!({"check_command": command})));
        store.record(
            "plan",
            "tasks",
            "clean",
            "title: Clean\nstatus: todo",
            PLAIN,
        );
        let (kind, said) = store.refused(&["task", "copy", "plan:clean", "--to", "site"]);
        assert_eq!(kind, "boundary-unavailable", "{command}: {said}");
        assert!(files_under(&store, "site").is_empty());
    }
}

#[test]
fn a_visibility_command_that_fails_or_answers_out_of_schema_leaves_a_repository_private() {
    let passing = json!(["python3", "-c", "print('{\"verdict\": \"pass\"}')"]);
    let answering = |answer: &str| {
        json!([
            "python3",
            "-c",
            format!("import sys; sys.stdin.read(); print('{answer}')")
        ])
    };
    // A command that answers `public` lets the task through, so a refusal below is the failure
    // being read as private rather than every repository being refused anyway.
    let store = Store::with_policy(Some(json!({
        "visibility_command": answering("{\"visibility\": \"public\"}"),
        "check_command": passing,
    })));
    store.record(
        "plan",
        "tasks",
        "open",
        &format!("title: Open work\nstatus: todo\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    store.ok(&["task", "copy", "plan:open", "--to", "site"]);
    for command in [
        // Exits non-zero.
        json!(["python3", "-c", "import sys; sys.stdin.read(); sys.exit(3)"]),
        // Exits 0 with a visibility the schema does not name.
        answering("{\"visibility\": \"sideways\"}"),
        // Exits 0 with something that is not JSON at all.
        answering("public"),
    ] {
        let store = Store::with_policy(Some(json!({
            "visibility_command": command,
            "check_command": passing,
        })));
        store.record(
            "plan",
            "tasks",
            "open",
            &format!("title: Open work\nstatus: todo\nrepositories: [{OPEN}]"),
            PLAIN,
        );
        let (kind, said) = store.refused(&["task", "copy", "plan:open", "--to", "site"]);
        assert_eq!(kind, "not-private-destination", "{command}: {said}");
        assert!(files_under(&store, "site").is_empty(), "{command}");
    }
}

/// `task create site` of `body` with `scope`'s flags, as an argument list.
fn create_on_site(body: &str, scope: &[&str]) -> Vec<String> {
    let mut arguments = vec![
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "T",
        "--body-file",
        body,
    ];
    arguments.extend_from_slice(scope);
    arguments.into_iter().map(str::to_owned).collect()
}

fn refused_kind(store: &Store, arguments: &[String]) -> String {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    store.refused(&arguments).0
}

fn written(store: &Store, arguments: &[String]) {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    store.ok(&arguments);
}

#[test]
fn a_caller_supplied_term_scope_decides_which_private_repositories_terms_are_derived() {
    let mut store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let a = store.body(A_TEXT);
    let b = store.body(B_TEXT);
    // Scoped to A: A's owner/name refuses, B's alone is written.
    assert_eq!(
        refused_kind(&store, &create_on_site(&a, &["--term-scope", A])),
        "boundary-refused"
    );
    written(&store, &create_on_site(&b, &["--term-scope", A]));
    // And the other way round.
    assert_eq!(
        refused_kind(&store, &create_on_site(&b, &["--term-scope", B])),
        "boundary-refused"
    );
    written(&store, &create_on_site(&a, &["--term-scope", B]));
    // No scope: every registered private identity's terms, so both refuse.
    assert_eq!(
        refused_kind(&store, &create_on_site(&a, &[])),
        "boundary-refused"
    );
    assert_eq!(
        refused_kind(&store, &create_on_site(&b, &[])),
        "boundary-refused"
    );
    // A third private identity whose committed manifest does not parse makes a check deriving
    // from every identity unavailable — and one deriving from none still answers.
    store
        .onevcs
        .as_mut()
        .expect("an onevcs home")
        .private_with_malformed_manifest("brokenco", "crackedbell");
    assert_eq!(
        refused_kind(&store, &create_on_site(&b, &[])),
        "boundary-unavailable"
    );
    written(&store, &create_on_site(&a, &["--term-scope-empty"]));
    written(&store, &create_on_site(&b, &["--term-scope-empty"]));
    // The two flags name opposite scopes, and are refused together as an invocation.
    let output = store.run(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "T",
        "--body-file",
        &b,
        "--term-scope",
        A,
        "--term-scope-empty",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[test]
fn a_mixed_project_and_everything_in_it_goes_wholly_to_the_private_source_its_route_names() {
    let store = Store::new(
        Some(registered),
        json!({
            "site": {"routes": [
                {"repositories": ["github.com/openco/*"], "to": "elsewhere"},
                {"classification": "private", "to": "vault"},
            ]},
            "elsewhere": {"plugin": "local-md", "config": {"root": "elsewhere"}, "visibility": "public"},
        }),
    );
    std::fs::create_dir_all(store.folder("elsewhere")).expect("the folder");
    store.record(
        "plan",
        "projects",
        "goal",
        "title: One goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        &format!("title: Open half\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "closed",
        &format!(
            "title: Private half\nstatus: todo\nproject: goal\nrepositories: [{A}]\n\
             depends_on: [open]"
        ),
        PLAIN,
    );
    store.record(
        "plan",
        "documents",
        "design",
        "title: Design\nproject: goal",
        PLAIN,
    );
    let report: Value = serde_json::from_str(&store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--to",
        "site",
        "--json",
    ]))
    .expect("a copy report");
    let items = report["items"].as_array().expect("outcomes");
    assert_eq!(items.len(), 3, "{report:#}");
    for outcome in items {
        assert_eq!(
            outcome["placed"]["destination"], "vault",
            "classification wins over the repository route: {report:#}"
        );
        assert_eq!(outcome["placed"]["route"], 1, "{report:#}");
    }
    assert!(files_under(&store, "site").is_empty());
    assert!(tree(&store.folder("elsewhere")).is_empty());
    let project = one_file(&store, "vault", "projects");
    assert!(project.contains("classification: private"), "{project}");
    // The document follows its project's home.
    store.ok(&["document", "copy", "plan:design", "--to", "site"]);
    let document = one_file(&store, "vault", "documents");
    assert!(document.contains("classification: private"), "{document}");
    assert!(files_under(&store, "site").is_empty());
    // And `sources route` answers the same placement from configuration alone.
    let route: Value = serde_json::from_str(&store.ok(&[
        "sources",
        "route",
        "site",
        "--classification",
        "private",
        "--repository",
        OPEN,
        "--json",
    ]))
    .expect("a route");
    assert_eq!(route["destination"], "vault");
}

#[test]
fn a_private_document_alone_takes_its_project_and_every_public_task_in_it_to_the_private_source() {
    let store = Store::new(
        Some(registered),
        json!({"site": {"routes": [{"classification": "private", "to": "vault"}]}}),
    );
    store.record(
        "plan",
        "projects",
        "goal",
        "title: One goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        &format!("title: Open work\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    // Nothing but the document is private: no repository and no task makes the project so.
    store.record(
        "plan",
        "documents",
        "design",
        "title: Design\nproject: goal\nclassification: private",
        PLAIN,
    );
    let report: Value = serde_json::from_str(&store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--to",
        "site",
        "--json",
    ]))
    .expect("a copy report");
    let items = report["items"].as_array().expect("outcomes");
    assert_eq!(items.len(), 2, "the project and its task: {report:#}");
    for outcome in items {
        assert_eq!(outcome["placed"]["destination"], "vault", "{report:#}");
    }
    assert!(files_under(&store, "site").is_empty());
    let project = one_file(&store, "vault", "projects");
    assert!(project.contains("classification: private"), "{project}");
    let task = one_file(&store, "vault", "tasks");
    assert!(task.contains("classification: private"), "{task}");
}

#[test]
fn a_public_classification_route_places_a_public_item_before_any_repository_route() {
    // `vault` sends public work to `site` by classification, and work of `openco` repositories
    // to `plan` by repository; a public item of an `openco` repository matches both.
    let store = Store::new(
        Some(registered),
        json!({"vault": {"routes": [
            {"repositories": ["github.com/openco/*"], "to": "plan"},
            {"classification": "public", "to": "site"},
        ]}}),
    );
    store.record(
        "plan",
        "tasks",
        "open",
        &format!("title: Open work\nstatus: todo\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "closed",
        &format!(
            "title: Closed work\nstatus: todo\nclassification: private\nrepositories: [{OPEN}]"
        ),
        PLAIN,
    );
    let report: Value =
        serde_json::from_str(&store.ok(&["task", "copy", "plan:open", "--to", "vault", "--json"]))
            .expect("a copy report");
    assert_eq!(
        report["items"][0]["placed"]["destination"], "site",
        "{report:#}"
    );
    assert_eq!(report["items"][0]["placed"]["route"], 1, "{report:#}");
    assert!(one_file(&store, "site", "tasks").contains("Open work"));
    // A private item is never sent by a route to a source not declared private: the public
    // entry is passed over, and the repository entry places it.
    let report: Value = serde_json::from_str(&store.ok(&[
        "task",
        "copy",
        "plan:closed",
        "--to",
        "vault",
        "--json",
    ]))
    .expect("a copy report");
    assert_eq!(
        report["items"][0]["placed"]["destination"], "plan",
        "{report:#}"
    );
    assert_eq!(report["items"][0]["placed"]["route"], 0, "{report:#}");
    // `sources route` answers both placements from configuration alone.
    for (classification, destination) in [("public", "site"), ("private", "plan")] {
        let route: Value = serde_json::from_str(&store.ok(&[
            "sources",
            "route",
            "vault",
            "--classification",
            classification,
            "--repository",
            OPEN,
            "--json",
        ]))
        .expect("a route");
        assert_eq!(
            route["destination"], destination,
            "{classification}: {route:#}"
        );
    }
}

#[test]
fn a_private_document_replaced_by_an_explicitly_public_request_stays_private() {
    // An inactive store, so nothing but the record being replaced says the document is private:
    // its project is public, and no declaration or policy reads it.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    sandbox.project_document(
        &json!({"sources": {"plan": {"plugin": "local-md", "config": {"root": plan}}}}).to_string(),
    );
    let store = Store {
        sandbox,
        onevcs: None,
    };
    store.record("plan", "projects", "p", "title: P\nstatus: todo", PLAIN);
    store.record(
        "plan",
        "documents",
        "secret",
        "title: Secret\nproject: p\nclassification: private",
        PLAIN,
    );
    let held = std::fs::read(store.folder("plan").join("documents/secret.md")).expect("held");
    let body = store.body(PLAIN);
    // The replacement keeps the private classification it replaces, so it is held to it — a
    // public project holds no private member — rather than written as public.
    let (kind, said) = store.refused(&[
        "document",
        "create",
        "plan",
        "--project",
        "p",
        "--id",
        "secret",
        "--title",
        "Replaced",
        "--body-file",
        &body,
        "--classification",
        "public",
    ]);
    assert_eq!(kind, "private-member", "{said}");
    assert_eq!(
        std::fs::read(store.folder("plan").join("documents/secret.md")).expect("held"),
        held,
        "the private document is as it was"
    );
}

#[test]
fn a_mixed_project_is_refused_whole_where_no_private_source_is_reachable() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "projects",
        "goal",
        "title: One goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        &format!("title: Open half\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "closed",
        &format!("title: Private half\nstatus: todo\nproject: goal\nrepositories: [{A}]"),
        PLAIN,
    );
    let (kind, _) = store.refused(&["project", "copy", "plan:goal", "--to", "site"]);
    assert_eq!(kind, "not-private-destination");
    assert!(
        files_under(&store, "site").is_empty(),
        "not even the project or its public task landed"
    );
    // A public task of that project, copied on its own, is private too: it inherits.
    let (kind, _) = store.refused(&["task", "copy", "plan:open", "--to", "site"]);
    assert_eq!(kind, "not-private-destination");
    assert!(files_under(&store, "site").is_empty());
}

#[test]
fn a_classification_route_to_a_source_not_declared_private_and_a_chain_are_refused_at_load() {
    for (extra, problem) in [
        (
            json!({"site": {"routes": [{"classification": "private", "to": "loose"}]},
                   "loose": {"plugin": "local-md", "config": {"root": "loose"}}}),
            "not declared private",
        ),
        (
            json!({"site": {"routes": [{"classification": "private", "to": "vault"}]},
                   "vault": {"routes": [{"classification": "private", "to": "plan"}]}}),
            "never chains",
        ),
        (
            json!({"site": {"routes": [{"classification": "secret", "to": "vault"}]}}),
            "not a classification",
        ),
        (
            json!({"site": {"routes": [{"classification": "private",
                                        "repositories": ["github.com/openco/*"], "to": "vault"}]}}),
            "names both",
        ),
    ] {
        let store = Store::new(Some(registered), extra);
        let output = store.run(&["sources", "list"]);
        assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
        assert!(stderr(&output).contains(problem), "{}", stderr(&output));
    }
}

#[test]
fn provenance_naming_a_private_source_never_reaches_a_public_one() {
    let store = Store::new(Some(registered), json!({}));
    // A public task kept in a private source: `vault` names no repository and is in no
    // term list, so only the filter keeps its name out.
    store.record(
        "vault",
        "tasks",
        "example",
        "title: Generic example\nstatus: todo",
        PLAIN,
    );
    store.ok(&["task", "copy", "vault:example", "--to", "site"]);
    let landed = one_file(&store, "site", "tasks");
    assert!(
        !landed.contains("vault") && !landed.contains("onetaskgraph.origin"),
        "{landed}"
    );
    // The correspondence is kept on the private side, so a second copy updates the first.
    let source = std::fs::read_to_string(store.folder("vault").join("tasks/example.md"))
        .expect("the source record");
    assert!(source.contains("onetaskgraph.copies"), "{source}");
    std::fs::write(
        store.folder("vault").join("tasks/example.md"),
        source.replace("title: Generic example", "title: Generic example, revised"),
    )
    .expect("the source edited");
    store.ok(&["task", "copy", "vault:example", "--to", "site"]);
    let landed = one_file(&store, "site", "tasks");
    assert!(landed.contains("revised"), "{landed}");
    // A copy from a public source into a private one records no link naming the private one.
    store.record(
        "site",
        "tasks",
        "shared",
        "title: Shared\nstatus: todo",
        PLAIN,
    );
    store.ok(&["task", "copy", "site:shared", "--to", "vault"]);
    let shared = std::fs::read_to_string(store.folder("site").join("tasks/shared.md"))
        .expect("the public record");
    assert!(!shared.contains("vault"), "{shared}");
}

#[test]
fn an_inherited_classification_is_never_loosened_by_an_explicit_public() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "projects",
        "secret-goal",
        "title: Secret goal\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "inside",
        "title: Inside\nstatus: todo\nproject: secret-goal\nclassification: public",
        PLAIN,
    );
    let (kind, _) = store.refused(&["task", "copy", "plan:inside", "--to", "site"]);
    assert_eq!(kind, "not-private-destination");
    let body = store.body(PLAIN);
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "secret-goal",
        "--title",
        "Created inside",
        "--body-file",
        &body,
        "--classification",
        "public",
    ]);
    let created = std::fs::read_to_string(store.folder("plan").join("tasks/created-inside.md"))
        .or_else(|_| {
            std::fs::read_to_string(
                store
                    .folder("plan")
                    .join("tasks/secret-goal/created-inside.md"),
            )
        })
        .unwrap_or_else(|_| panic!("the created task: {:?}", files_under(&store, "plan")));
    assert!(
        created.contains("classification: private"),
        "the project's private classification is the task's: {created}"
    );
}

#[test]
fn a_private_member_is_refused_by_a_public_project_before_anything_is_written() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "projects",
        "open-goal",
        "title: Open goal\nstatus: todo",
        PLAIN,
    );
    let before = store.tree();
    let body = store.body(PLAIN);
    for kind in ["task", "document"] {
        let (failure, said) = store.refused(&[
            kind,
            "create",
            "plan",
            "--project",
            "open-goal",
            "--title",
            "Secret member",
            "--body-file",
            &body,
            "--classification",
            "private",
        ]);
        assert_eq!(failure, "private-member", "{said}");
        assert!(said.contains("plan:open-goal"), "{said}");
    }
    // A task naming a private repository is a private member as well.
    let (failure, _) = store.refused(&[
        "task",
        "create",
        "plan",
        "--project",
        "open-goal",
        "--title",
        "Repo member",
        "--body-file",
        &body,
        "--repository",
        A,
    ]);
    assert_eq!(failure, "private-member");
    assert_eq!(store.tree(), before, "nothing was written");
}

#[test]
fn a_project_stays_private_after_the_members_that_made_it_so_are_removed() {
    let store = Store::new(
        Some(registered),
        json!({"site": {"routes": [{"classification": "private", "to": "vault"}]}}),
    );
    store.record(
        "plan",
        "projects",
        "goal",
        "title: One goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "closed",
        &format!("title: Private half\nstatus: todo\nproject: goal\nrepositories: [{A}]"),
        PLAIN,
    );
    store.ok(&["project", "copy", "plan:goal", "--to", "site"]);
    std::fs::remove_file(store.folder("plan").join("tasks/closed.md")).expect("removed");
    store.ok(&["project", "copy", "plan:goal", "--to", "site"]);
    let project = one_file(&store, "vault", "projects");
    assert!(project.contains("classification: private"), "{project}");
    // Read back through the binary, too.
    let landed = files_under(&store, "vault")
        .into_iter()
        .find(|path| path.contains("projects"))
        .expect("the vault project");
    let id = Path::new(&landed)
        .file_stem()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    let shown: Value =
        serde_json::from_str(&store.ok(&["project", "show", &format!("vault:{id}"), "--json"]))
            .expect("a project");
    assert_eq!(
        shown["items"][0]["item"]["classification"], "private",
        "{shown:#}"
    );
}

#[test]
fn every_write_adding_to_a_private_item_held_by_a_public_source_is_refused() {
    let store = Store::new(Some(registered), json!({}));
    // Held there before the boundary existed: a hand-made record a public source holds.
    store.record(
        "site",
        "tasks",
        "stray",
        "title: Stray\nstatus: todo\nclassification: private",
        &format!(
            "{PLAIN}\n\n## Comments\n\n\
             <!-- onetaskgraph:comment id=\"C-1\" created_at=\"2026-09-13T15:11:07Z\" \
             updated_at=\"2026-09-13T15:11:07Z\" -->\n\
             ### comment — 2026-09-13T15:11:07Z\n\nAn earlier note.\n\n\
             <!-- /onetaskgraph:comment -->"
        ),
    );
    store.record("site", "tasks", "fine", "title: Fine\nstatus: todo", PLAIN);
    store.record(
        "site",
        "projects",
        "stray-project",
        "title: Stray project\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "site",
        "documents",
        "stray-document",
        "title: Stray document\nclassification: private",
        PLAIN,
    );
    let content = store.body(PLAIN);
    let template = store.sandbox.config_home().join("plain-template.md");
    std::fs::write(
        &template,
        "---\nonetaskgraph_template: 1\nvariables: {}\n---\nRendered afresh.\n",
    )
    .expect("a template");
    let template = template.display().to_string();
    let before = store.tree();
    for arguments in [
        vec!["task", "render", "site:stray", "--template", &template],
        vec![
            "project",
            "render",
            "site:stray-project",
            "--template",
            &template,
        ],
        vec![
            "document",
            "render",
            "site:stray-document",
            "--template",
            &template,
        ],
        vec![
            "task",
            "comment",
            "edit",
            "site:stray",
            "C-1",
            "--body-file",
            &content,
        ],
        vec![
            "project",
            "metadata",
            "set",
            "site:stray-project",
            "team.note",
            "\"x\"",
        ],
        vec![
            "document",
            "metadata",
            "set",
            "site:stray-document",
            "team.note",
            "\"x\"",
        ],
        vec!["task", "status", "set", "site:stray", "done"],
        vec!["task", "priority", "set", "site:stray", "high"],
        vec!["task", "content", "set", "site:stray", "--file", &content],
        vec![
            "task",
            "metadata",
            "set",
            "site:stray",
            "team.note",
            "\"x\"",
        ],
        vec!["task", "update", "site:stray", "--title", "Renamed"],
        vec![
            "task",
            "comment",
            "add",
            "site:stray",
            "--body-file",
            &content,
        ],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "not-private-destination", "{arguments:?}: {said}");
    }
    // A public item there is written, and what a narrow write adds is still checked.
    let (kind, _) = store.refused(&[
        "task",
        "metadata",
        "set",
        "site:fine",
        "team.note",
        "\"see hiddenco/quietharbor\"",
    ]);
    assert_eq!(kind, "boundary-refused");
    assert_eq!(store.tree(), before, "no refused write changed anything");
    store.ok(&[
        "task",
        "metadata",
        "set",
        "site:fine",
        "team.note",
        "\"generic\"",
    ]);
    store.ok(&["task", "status", "set", "site:fine", "done"]);
}

#[test]
fn an_asset_named_after_a_private_repository_is_refused_onto_a_public_source() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let image = store.sandbox.config_home().join("quietharbor.png");
    std::fs::write(&image, onetaskgraph_e2e_support::images::png(7, 60_000)).expect("an image");
    let body = store.body("Diagram: ![flow](./quietharbor.png)");
    let (kind, said) = store.refused(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "T",
        "--body-file",
        &body,
        "--asset",
        &image.display().to_string(),
    ]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert_site_holds_only_p(&store);
}

#[test]
fn a_hosted_plugin_declared_private_is_never_verified_private_so_a_private_item_is_refused_there() {
    let store = Store::new(
        Some(registered),
        json!({"hosted": {
            "plugin": "subprocess",
            "config": {
                "command": onetaskgraph_e2e_support::source_binary(),
                "settings": {"kind": "local-md", "config": {"root": "hosted"}},
            },
            "visibility": "private",
        }}),
    );
    std::fs::create_dir_all(store.folder("hosted")).expect("the hosted folder");
    store.record(
        "plan",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let (kind, said) = store.refused(&["task", "copy", "plan:secret", "--to", "hosted"]);
    assert_eq!(kind, "destination-not-private", "{said}");
    assert!(said.contains("reads unknown"), "{said}");
    assert!(tree(&store.folder("hosted")).is_empty());
}

/// A program linking the engine, implementing the policy itself over the released onevcs:
/// each answer read through a file rather than standard input, its home named per command.
struct LinkedOnevcs {
    home: PathBuf,
}

impl LinkedOnevcs {
    fn run(&self, arguments: &[&str], input: &Value) -> (Option<i32>, Vec<u8>) {
        let file = tempfile::NamedTempFile::new().expect("an input file");
        std::fs::write(file.path(), input.to_string()).expect("the input is written");
        let output = std::process::Command::new(crate::onevcs::onevcs())
            .args(arguments)
            .arg("--input")
            .arg(file.path())
            .env("ONEVCS_HOME", &self.home)
            .output()
            .expect("onevcs runs");
        (output.status.code(), output.stdout)
    }
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for LinkedOnevcs {
    async fn repository_visibility(
        &self,
        repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        let (code, answer) = self.run(
            &["boundary", "inspect"],
            &json!({"repository": repository.as_str()}),
        );
        let answer: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
        match (code, answer["visibility"].as_str()) {
            (Some(0), Some("public")) => {
                Ok(onetaskgraph_core::boundary::RepositoryVisibility::Public)
            }
            (Some(0), Some("private")) => {
                Ok(onetaskgraph_core::boundary::RepositoryVisibility::Private)
            }
            _ => Ok(onetaskgraph_core::boundary::RepositoryVisibility::Unknown),
        }
    }

    async fn check_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        let (code, answer) = self.run(
            &["boundary", "check", "--destination", "public"],
            &serde_json::to_value(input).expect("an input serializes"),
        );
        Ok(onetaskgraph_core::boundary::check_verdict(code, &answer))
    }
}

/// A policy that cannot be asked at all.
struct Unreachable;

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Unreachable {
    async fn repository_visibility(
        &self,
        _repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Err(onetaskgraph_core::boundary::PolicyError {
            message: "the policy is not reachable".to_owned(),
        })
    }

    async fn check_public_write(
        &self,
        _input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        Err(onetaskgraph_core::boundary::PolicyError {
            message: "the policy is not reachable".to_owned(),
        })
    }
}

/// An engine over a store's folders, as a linking caller builds one: from the configuration
/// document, with no `write_policy` of its own.
fn linked_engine(store: &Store) -> onetaskgraph_core::Engine {
    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    let mut document = document;
    document
        .as_object_mut()
        .expect("a document")
        .remove("write_policy");
    let config = onetaskgraph_core::Config::from_document(document).expect("a configuration");
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs(
        Vec::<(String, String)>::new(),
    ))
    .expect("no credentials");
    onetaskgraph_core::Engine::build(&config, &secrets)
}

fn task_on_site(text: &str, repositories: &[&str]) -> onetaskgraph_core::TaskCreate {
    onetaskgraph_core::TaskCreate {
        source: onetaskgraph_plugin_api::SourceName::new("site").expect("a name"),
        project: onetaskgraph_plugin_api::NativeId::from("p"),
        title: "Linked".to_owned(),
        body: onetaskgraph_core::Body::plain(text),
        status: None,
        labels: Vec::new(),
        repositories: repositories
            .iter()
            .map(|origin| {
                onetaskgraph_plugin_api::Repository::try_from((*origin).to_owned())
                    .expect("an origin")
            })
            .collect(),
        depends_on: Vec::new(),
        delivers: Vec::new(),
        metadata: BTreeMap::new(),
        assets: Vec::new(),
        classification: onetaskgraph_plugin_api::Classification::Public,
    }
}

fn kind_of(error: &onetaskgraph_core::EngineError) -> String {
    onetaskgraph_core::Failure::from(error).kind().to_owned()
}

#[tokio::test]
async fn a_linking_caller_s_own_policy_and_term_scope_reach_the_verdicts_the_command_line_does() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let home = store
        .onevcs
        .as_ref()
        .expect("an onevcs home")
        .path()
        .to_path_buf();
    let linked = || {
        linked_engine(&store)
            .with_write_policy(std::sync::Arc::new(LinkedOnevcs { home: home.clone() }))
    };
    // Scoped to A on the store itself.
    let scoped = linked().with_term_scope(Some(vec![A.to_owned()]));
    let refused = scoped
        .create_task(&task_on_site(A_TEXT, &[]))
        .await
        .expect_err("A's owner/name is refused");
    assert_eq!(kind_of(&refused), "boundary-refused");
    scoped
        .create_task(&task_on_site(B_TEXT, &[]))
        .await
        .expect("B's owner/name alone is written");
    // Unscoped, both refuse; scoped to nothing, both are written.
    for text in [A_TEXT, B_TEXT] {
        let refused = linked()
            .create_task(&task_on_site(text, &[]))
            .await
            .expect_err("every private identity's terms");
        assert_eq!(kind_of(&refused), "boundary-refused");
        linked()
            .with_term_scope(Some(Vec::new()))
            .create_task(&task_on_site(text, &[]))
            .await
            .expect("no terms derived");
    }
    // The repository answer is the policy's: a private one makes the task private.
    let refused = linked()
        .create_task(&task_on_site(PLAIN, &[A]))
        .await
        .expect_err("a private repository's task");
    assert_eq!(kind_of(&refused), "private-member");
    linked()
        .create_task(&task_on_site(PLAIN, &[OPEN]))
        .await
        .expect("a public repository's task");
}

#[tokio::test]
async fn a_linking_caller_s_missing_or_unreachable_policy_never_approves_a_public_write() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    for engine in [
        linked_engine(&store).with_write_policy(std::sync::Arc::new(
            onetaskgraph_core::boundary::MissingWritePolicy,
        )),
        linked_engine(&store).with_write_policy(std::sync::Arc::new(Unreachable)),
    ] {
        assert!(engine.boundary_active());
        let refused = engine
            .create_task(&task_on_site(PLAIN, &[]))
            .await
            .expect_err("never approved");
        assert_eq!(kind_of(&refused), "boundary-unavailable");
        // An unanswered repository is private.
        let refused = engine
            .create_task(&task_on_site(PLAIN, &[OPEN]))
            .await
            .expect_err("unknown is private");
        assert_eq!(kind_of(&refused), "private-member");
    }
    assert_site_holds_only_p(&store);
}

#[tokio::test]
async fn a_linking_caller_s_policy_alone_activates_a_store_that_declares_nothing() {
    // No declared visibility and no write_policy: the store a program links is inactive, and
    // writes as it always did until the program hands it a policy.
    let sandbox = Sandbox::new();
    let site = sandbox.subdirectory("site");
    sandbox.project_document(
        &json!({"sources": {"site": {"plugin": "local-md", "config": {"root": site}}}}).to_string(),
    );
    let store = Store {
        sandbox,
        onevcs: None,
    };
    store.public_project_p();
    let inactive = linked_engine(&store);
    assert!(!inactive.boundary_active());
    inactive
        .create_task(&task_on_site(PLAIN, &[OPEN]))
        .await
        .expect("an inactive store asks nobody");
    let written = files_under(&store, "site");
    let active = linked_engine(&store).with_write_policy(std::sync::Arc::new(Unreachable));
    assert!(active.boundary_active());
    let refused = active
        .create_task(&task_on_site(PLAIN, &[]))
        .await
        .expect_err("a policy that cannot answer never approves");
    // The task the inactive store wrote under `p` names a repository this policy cannot answer
    // for, so it reads private — and so does `p`, which holds it, and the task filed under it.
    assert_eq!(kind_of(&refused), "not-private-destination");
    assert_eq!(
        files_under(&store, "site"),
        written,
        "nothing more was written"
    );
}

#[test]
fn a_private_deliverer_s_id_is_withheld_from_a_delivered_task_by_its_classification_alone() {
    // An inactive store: nothing declares `plan` or `notes` private, so the deliverer's own
    // classification is the one thing that says its id must not reach `notes`.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    let notes = sandbox.subdirectory("notes");
    sandbox.project_document(
        &json!({"sources": {
            "plan": {"plugin": "local-md", "config": {"root": plan}},
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        }})
        .to_string(),
    );
    let store = Store {
        sandbox,
        onevcs: None,
    };
    store.record(
        "plan",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private\ndelivers: [\"notes:ticket\"]",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open\nstatus: todo\ndelivers: [\"notes:pair\"]",
        PLAIN,
    );
    store.record(
        "notes",
        "tasks",
        "ticket",
        "title: Ticket\nstatus: todo",
        PLAIN,
    );
    store.record("notes", "tasks", "pair", "title: Pair\nstatus: todo", PLAIN);
    store.ok(&["task", "status", "set", "plan:secret", "in-progress"]);
    let ticket =
        std::fs::read_to_string(store.folder("notes").join("tasks/ticket.md")).expect("the ticket");
    assert!(
        ticket.contains("status: in progress"),
        "the rule still applies: {ticket}"
    );
    assert!(!ticket.contains("plan:secret"), "{ticket}");
    // A public deliverer in the same source is named as ever.
    store.ok(&["task", "status", "set", "plan:open", "in-progress"]);
    let pair =
        std::fs::read_to_string(store.folder("notes").join("tasks/pair.md")).expect("the pair");
    assert!(pair.contains("plan:open"), "{pair}");
}

#[test]
fn the_boundary_payloads_reconcile_with_the_released_onevcs_schema() {
    use onetaskgraph_core::boundary::{
        ONEVCS_BOUNDARY_SCHEMA, ONEVCS_BOUNDARY_SCHEMA_VERSION, PublicWriteInput,
        RepositoryVisibility, WriteVerdict, check_verdict,
    };
    let output = std::process::Command::new(crate::onevcs::onevcs())
        .args(["boundary", "schema", "--json"])
        .output()
        .expect("onevcs runs");
    assert!(output.status.success());
    let released: Value = serde_json::from_slice(&output.stdout).expect("the schema is JSON");
    let pinned: Value = serde_json::from_str(ONEVCS_BOUNDARY_SCHEMA).expect("JSON");
    assert_eq!(
        released, pinned,
        "the schema the command adapter holds answers to is the one the pinned release prints; \
         a moved pin re-pins it"
    );
    assert_eq!(
        released["schema_version"],
        json!(ONEVCS_BOUNDARY_SCHEMA_VERSION)
    );

    // What the store sends: every shape of a write's input, and an inspect request.
    let check_input = jsonschema::validator_for(&released["check"]["input"]).expect("compiles");
    for scope in [
        None,
        Some(Vec::new()),
        Some(vec![A.to_owned(), B.to_owned()]),
    ] {
        let input = PublicWriteInput {
            destination: RepositoryVisibility::Public,
            text: vec![PLAIN.to_owned()],
            paths: vec!["docs/shot.png".to_owned()],
            metadata: vec!["Title".to_owned()],
            scope: scope.clone(),
        };
        let sent = serde_json::to_value(&input).expect("serializes");
        assert!(check_input.is_valid(&sent), "{sent}");
        assert_eq!(
            sent.get("scope").is_some(),
            scope.is_some(),
            "an absent scope is left out, which the check reads as its registry: {sent}"
        );
    }
    let inspect_input = jsonschema::validator_for(&released["inspect"]["input"]).expect("compiles");
    assert!(inspect_input.is_valid(&json!({"repository": A})));
    // The defaults the store reads an omitted member as are the producer's.
    // The store's own shape, as the bundle both SDKs are generated from emits it.
    let ours = onetaskgraph_core::schema_bundle()["roots"]["PublicWriteInput"].clone();
    let theirs = &released["check"]["input"];
    let names = |schema: &Value| {
        let mut names: Vec<String> = schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&ours), names(theirs));
    assert_eq!(ours["required"], theirs["required"]);
    for member in ["text", "paths", "metadata"] {
        assert_eq!(theirs["properties"][member]["default"], json!([]));
        assert_eq!(ours["properties"][member]["default"], json!([]));
    }
    let defaulted: PublicWriteInput =
        serde_json::from_value(json!({"destination": "public"})).expect("the defaults read");
    assert!(defaulted.text.is_empty() && defaulted.paths.is_empty());
    assert!(defaulted.metadata.is_empty() && defaulted.scope.is_none());

    // What it reads: every verdict the producer can answer, and nothing passes but a pass.
    let verdicts = &released["check"]["output"];
    let constants = |definition: &str| -> Vec<String> {
        verdicts["$defs"][definition]["oneOf"]
            .as_array()
            .expect("a oneOf")
            .iter()
            .map(|variant| variant["const"].as_str().expect("a const").to_owned())
            .collect()
    };
    assert_eq!(
        check_verdict(Some(0), br#"{"verdict":"pass"}"#),
        WriteVerdict::Pass
    );
    assert!(matches!(
        check_verdict(Some(1), br#"{"verdict":"pass"}"#),
        WriteVerdict::Unavailable { .. }
    ));
    for surface in constants("Surface") {
        let answer = json!({"verdict": "refuse", "surface": surface}).to_string();
        assert_eq!(
            check_verdict(Some(1), answer.as_bytes()),
            WriteVerdict::Refuse {
                reason: format!("it carries a term of a private repository in its {surface}")
            }
        );
        assert!(matches!(
            check_verdict(Some(0), answer.as_bytes()),
            WriteVerdict::Unavailable { .. }
        ));
    }
    for reason in constants("Unavailability") {
        let answer = json!({"verdict": "unavailable", "reason": reason}).to_string();
        for code in [Some(2), Some(0), None] {
            assert!(
                matches!(
                    check_verdict(code, answer.as_bytes()),
                    WriteVerdict::Unavailable { .. }
                ),
                "{reason} under {code:?}"
            );
        }
    }
    for foreign in [&b"{}"[..], b"not json", br#"{"verdict":"maybe"}"#, b""] {
        assert!(matches!(
            check_verdict(Some(0), foreign),
            WriteVerdict::Unavailable { .. }
        ));
    }
}

#[test]
fn a_standalone_private_document_or_project_is_refused_onto_a_public_source() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "documents",
        "secret-design",
        "title: Secret design\nclassification: private",
        PLAIN,
    );
    store.record(
        "plan",
        "projects",
        "secret-goal",
        "title: Secret goal\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let before = store.tree();
    for arguments in [
        vec!["document", "copy", "plan:secret-design", "--to", "site"],
        vec!["project", "copy", "plan:secret-goal", "--to", "site"],
        vec![
            "project",
            "copy",
            "plan:secret-goal",
            "--to",
            "site",
            "--dry-run",
        ],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "not-private-destination", "{arguments:?}: {said}");
    }
    assert_eq!(store.tree(), before);
    store.ok(&["document", "copy", "plan:secret-design", "--to", "vault"]);
    store.ok(&["project", "copy", "plan:secret-goal", "--to", "vault"]);
}

#[test]
fn a_private_task_copied_on_its_own_is_refused_by_the_public_project_it_would_join() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "projects",
        "goal",
        "title: Goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open\nstatus: todo\nproject: goal",
        PLAIN,
    );
    // The project lands in the private vault while everything in it is public.
    store.ok(&["project", "copy", "plan:goal", "--to", "vault"]);
    let landed = one_file(&store, "vault", "projects");
    assert!(!landed.contains("classification"), "{landed}");
    // A private task is added at the source, and copied on its own.
    store.record(
        "plan",
        "tasks",
        "closed",
        "title: Closed\nstatus: todo\nproject: goal\nclassification: private",
        PLAIN,
    );
    let before = store.tree();
    // A dry run reads the project it would join, so it meets the refusal its copy does.
    for arguments in [
        vec!["task", "copy", "plan:closed", "--to", "vault", "--dry-run"],
        vec!["task", "copy", "plan:closed", "--to", "vault"],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "private-member", "{arguments:?}: {said}");
    }
    assert_eq!(store.tree(), before, "nothing was written");
    // Copying the whole project again classifies it private, and its new member lands with it.
    store.ok(&["project", "copy", "plan:goal", "--to", "vault"]);
    let landed = one_file(&store, "vault", "projects");
    assert!(landed.contains("classification: private"), "{landed}");
}

#[test]
fn a_term_carried_only_in_template_provenance_is_refused_onto_a_public_source() {
    let store = Store::new(Some(registered), json!({}));
    // A template kept under a directory named after a private repository: the rendered task
    // says nothing private, but the provenance it records names the template's path.
    let directory = store.sandbox.config_home().join("quietharbor");
    std::fs::create_dir_all(&directory).expect("the template directory");
    let template = directory.join("task.md");
    std::fs::write(
        &template,
        "---\nonetaskgraph_template: 1\nvariables: {}\n---\nA generic step.\n",
    )
    .expect("a template");
    store.record(
        "plan",
        "projects",
        "goal",
        "title: Goal\nstatus: todo",
        PLAIN,
    );
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "goal",
        "--title",
        "Rendered",
        "--template",
        &template.display().to_string(),
    ]);
    let created = files_under(&store, "plan")
        .into_iter()
        .find(|path| path.contains("rendered"))
        .expect("the rendered task");
    let id = Path::new(&created)
        .strip_prefix(store.folder("plan").join("tasks"))
        .expect("under tasks")
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let (kind, said) = store.refused(&["task", "copy", &format!("plan:{id}"), "--to", "site"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(files_under(&store, "site").is_empty());
}

fn held_text(store: &Store, folder: &str) -> String {
    files_under(store, folder)
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("a record"))
        .collect()
}

#[test]
fn a_copy_withholds_an_id_naming_a_private_item_from_a_public_write_and_a_private_one_keeps_it() {
    // `shelf` is a second public folder, so an edge between two public sources is in play.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({ "shelf": folder(sandbox, "shelf", "public") }),
    );
    // `vault` and `plan` name no repository and are in no term list: only their declarations
    // keep their ids off `site`.
    store.record(
        "vault",
        "tasks",
        "hidden",
        "title: Hidden\nstatus: todo",
        PLAIN,
    );
    store.record("shelf", "tasks", "open", "title: Open\nstatus: todo", PLAIN);
    store.record("plan", "tasks", "left", "title: Left\nstatus: todo", PLAIN);
    store.record(
        "plan",
        "tasks",
        "sibling",
        "title: Sibling\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "example",
        "title: Example\nstatus: todo\n\
         depends_on: [\"vault:hidden\", \"shelf:open\", left, sibling]\n\
         delivers: [\"vault:hidden\", \"shelf:open\"]",
        PLAIN,
    );

    let report: Value = serde_json::from_str(&store.ok(&[
        "task",
        "copy",
        "plan:example",
        "plan:sibling",
        "--to",
        "site",
        "--json",
    ]))
    .expect("a copy report");
    let landed = held_text(&store, "site");
    // A dependency on an item of a private source — one named by its source, and one the copy
    // did not carry and would have qualified as `plan:left` — and a `delivers` entry naming
    // one are all left off the public write.
    assert!(!landed.contains("vault"), "{landed}");
    assert!(!landed.contains("plan:"), "{landed}");
    assert!(!landed.contains("hidden"), "{landed}");
    // Between public items nothing moves: the edge into `shelf` and the delivery of a `shelf`
    // task are written as before, and so is the edge to the sibling the copy carried along.
    assert!(landed.contains("shelf:open"), "{landed}");
    let example = report["items"]
        .as_array()
        .expect("the copied items")
        .iter()
        .find(|item| item["source"] == "plan:example")
        .expect("the example's outcome");
    let example_at = example["destination"]
        .as_str()
        .expect("where the example landed")
        .to_owned();
    let sibling_at = report["items"][1]["destination"]
        .as_str()
        .expect("where the sibling landed")
        .to_owned();
    let sibling_native = sibling_at.strip_prefix("site:").expect("a site id");
    let example_file = std::fs::read_to_string(store.folder("site").join("tasks").join(format!(
        "{}.md",
        example_at.strip_prefix("site:").expect("a site id")
    )))
    .expect("the example's copy");
    assert!(example_file.contains(sibling_native), "{example_file}");
    // The private delivered task still learns who delivers it — in the private store's own
    // record — and the public one is told exactly as before.
    let hidden = std::fs::read_to_string(store.folder("vault").join("tasks/hidden.md"))
        .expect("the hidden task");
    assert!(hidden.contains(&example_at), "{hidden}");
    let open = std::fs::read_to_string(store.folder("shelf").join("tasks/open.md"))
        .expect("the open task");
    assert!(open.contains(&example_at), "{open}");

    // A destination declared private keeps every id: the dependency on `plan:left` and the
    // `delivers` entry naming `vault:hidden` arrive as the copy read them.
    store.ok(&["task", "copy", "plan:example", "--to", "vault"]);
    let kept = held_text(&store, "vault");
    assert!(kept.contains("plan:left"), "{kept}");
    assert!(kept.contains("shelf:open"), "{kept}");
}

#[test]
fn a_delivered_task_in_a_public_source_is_never_handed_its_private_deliverers_id() {
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({ "shelf": folder(sandbox, "shelf", "public") }),
    );
    store.record(
        "site",
        "tasks",
        "ticket",
        "title: Ticket\nstatus: todo",
        PLAIN,
    );
    store.record(
        "vault",
        "tasks",
        "worker",
        "title: Worker\nstatus: todo\ndelivers: [\"site:ticket\"]",
        PLAIN,
    );
    store.record("site", "tasks", "pair", "title: Pair\nstatus: todo", PLAIN);
    store.record(
        "shelf",
        "tasks",
        "helper",
        "title: Helper\nstatus: todo\ndelivers: [\"site:pair\"]",
        PLAIN,
    );

    let set: Value = serde_json::from_str(&store.ok(&[
        "task",
        "status",
        "set",
        "vault:worker",
        "in-progress",
        "--json",
    ]))
    .expect("a status answer");
    // The rule still follows the private deliverer — the ticket moves with it — but the
    // ticket's own record names nobody, because the one deliverer it has is private.
    assert_eq!(set["delivered"][0]["outcome"], "written", "{set:#}");
    let ticket =
        std::fs::read_to_string(store.folder("site").join("tasks/ticket.md")).expect("the ticket");
    assert!(!ticket.contains("vault"), "{ticket}");
    assert!(!ticket.contains("delivered_by"), "{ticket}");
    assert!(ticket.contains("status: in progress"), "{ticket}");

    // A public deliverer in another public source is named on the task it delivers, as ever.
    store.ok(&["task", "status", "set", "shelf:helper", "in-progress"]);
    let pair =
        std::fs::read_to_string(store.folder("site").join("tasks/pair.md")).expect("the pair");
    assert!(pair.contains("shelf:helper"), "{pair}");
}

#[test]
fn a_reference_a_caller_names_to_a_private_source_is_refused_onto_a_public_one() {
    // A copy and a delivery withhold such an id, because the engine carries it along; a
    // `--depends-on` or a `--delivers` is the caller's own request to write it, so leaving it
    // off would answer a different request — it is refused instead.
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "vault",
        "tasks",
        "hidden",
        "title: Hidden\nstatus: todo",
        PLAIN,
    );
    store.record("site", "tasks", "fine", "title: Fine\nstatus: todo", PLAIN);
    let body = store.body(PLAIN);
    let before = store.tree();
    for arguments in [
        vec![
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "T",
            "--body-file",
            &body,
            "--depends-on",
            "vault:hidden",
        ],
        vec![
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "T",
            "--body-file",
            &body,
            "--delivers",
            "vault:hidden",
        ],
        vec![
            "task",
            "update",
            "site:fine",
            "--depends-on",
            "vault:hidden",
        ],
        vec!["task", "update", "site:fine", "--delivers", "vault:hidden"],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "private-reference", "{arguments:?}: {said}");
        assert!(said.contains("source vault"), "{said}");
    }
    assert_eq!(store.tree(), before, "no refused write left anything");
}

/// A check command that records the scope of every input it is handed, one JSON line each, in
/// the file its one argument names, and passes it.
const RECORDING_CHECK: &str = "import json, sys\n\
    payload = json.load(sys.stdin)\n\
    with open(sys.argv[1], 'a', encoding='utf-8') as record:\n\
    \x20   record.write(json.dumps(payload.get('scope', 'absent')) + '\\n')\n\
    print(json.dumps({'verdict': 'pass'}))\n";

#[test]
fn every_writing_verb_hands_its_term_scope_to_the_check_unchanged() {
    let recorded = tempfile::tempdir().expect("a directory for the record");
    let record = recorded.path().join("scopes.jsonl");
    let store = Store::with_policy(Some(json!({
        "check_command": ["python3", "-c", RECORDING_CHECK, record.display().to_string()],
    })));
    let comment = "\n\n## Comments\n\n\
        <!-- onetaskgraph:comment id=\"C-1\" created_at=\"2026-09-13T15:11:07Z\" \
        updated_at=\"2026-09-13T15:11:07Z\" -->\n\
        ### comment — 2026-09-13T15:11:07Z\n\nAn earlier note.\n\n\
        <!-- /onetaskgraph:comment -->";
    store.record(
        "site",
        "tasks",
        "held",
        "title: Held\nstatus: todo\nproject: home",
        &format!("{PLAIN}{comment}"),
    );
    store.record(
        "site",
        "projects",
        "home",
        "title: Home\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "documents",
        "notes",
        "title: Notes\nproject: home",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "source",
        "title: Source\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "projects",
        "away",
        "title: Away\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "documents",
        "memo",
        "title: Memo\nproject: away",
        PLAIN,
    );
    let body = store.body(PLAIN);
    let template = store.sandbox.config_home().join("scoped-template.md");
    std::fs::write(
        &template,
        "---\nonetaskgraph_template: 1\nvariables: {}\n---\nRendered afresh.\n",
    )
    .expect("a template");
    let template = template.display().to_string();
    let verbs: Vec<Vec<&str>> = vec![
        vec![
            "task",
            "create",
            "site",
            "--project",
            "home",
            "--title",
            "New",
            "--body-file",
            &body,
        ],
        vec!["task", "copy", "plan:source", "--to", "site"],
        vec!["task", "status", "set", "site:held", "in-progress"],
        vec!["task", "priority", "set", "site:held", "high"],
        vec!["task", "content", "set", "site:held", "--file", &body],
        vec![
            "task",
            "metadata",
            "set",
            "site:held",
            "team.note",
            "\"generic\"",
        ],
        vec!["task", "update", "site:held", "--title", "Renamed"],
        vec!["task", "render", "site:held", "--template", &template],
        vec!["task", "comment", "add", "site:held", "--body-file", &body],
        vec![
            "task",
            "comment",
            "edit",
            "site:held",
            "C-1",
            "--body-file",
            &body,
        ],
        vec![
            "project",
            "create",
            "site",
            "--id",
            "fresh",
            "--title",
            "Fresh",
            "--body-file",
            &body,
        ],
        vec!["project", "copy", "plan:away", "--to", "site", "--no-tasks"],
        vec![
            "project",
            "metadata",
            "set",
            "site:home",
            "team.note",
            "\"generic\"",
        ],
        vec!["project", "render", "site:home", "--template", &template],
        vec![
            "document",
            "create",
            "site",
            "--project",
            "home",
            "--title",
            "Fresh",
            "--body-file",
            &body,
        ],
        vec!["document", "copy", "plan:memo", "--to", "site"],
        vec![
            "document",
            "metadata",
            "set",
            "site:notes",
            "team.note",
            "\"generic\"",
        ],
        vec!["document", "render", "site:notes", "--template", &template],
    ];
    let lines = || {
        std::fs::read_to_string(&record)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("a recorded scope"))
            .collect::<Vec<Value>>()
    };
    for (index, verb) in verbs.iter().enumerate() {
        let before = lines().len();
        // A scope of its own for every verb, so a scope reaching the wrong write is seen too.
        let scope = format!("github.com/scoped/verb-{index}");
        let mut arguments = verb.clone();
        arguments.extend(["--term-scope", &scope]);
        store.ok(&arguments);
        let seen = lines();
        assert!(seen.len() > before, "{verb:?} was never checked");
        for handed in &seen[before..] {
            assert_eq!(handed, &json!([scope]), "{verb:?}");
        }
    }
    // An empty scope and none at all are told apart, through the same command line.
    let before = lines().len();
    store.ok(&[
        "task",
        "status",
        "set",
        "site:held",
        "done",
        "--term-scope-empty",
    ]);
    store.ok(&["task", "status", "set", "site:held", "todo"]);
    assert_eq!(lines()[before..], [json!([]), json!("absent")]);
}

#[test]
fn an_in_memory_source_declared_private_is_a_host_local_destination_a_private_item_lands_in() {
    let store = Store::new(
        Some(registered),
        json!({"memory": {"plugin": "in-memory", "config": {}, "visibility": "private"}}),
    );
    store.record(
        "plan",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        A_TEXT,
    );
    // Nothing written to this process's memory leaves the host, so the source's own answer is
    // private, and a private item naming a private repository lands there unchecked.
    let report: Value = serde_json::from_str(&store.ok(&[
        "task",
        "copy",
        "plan:secret",
        "--to",
        "memory",
        "--json",
    ]))
    .expect("a copy report");
    assert_eq!(report["items"][0]["action"], "created", "{report:#}");
}

/// A check command that records every input it is handed whole, one JSON line each, in the
/// file its one argument names, and passes it.
const PAYLOAD_CHECK: &str = "import json, sys\n\
    payload = json.load(sys.stdin)\n\
    with open(sys.argv[1], 'a', encoding='utf-8') as record:\n\
    \x20   record.write(json.dumps(payload) + '\\n')\n\
    print(json.dumps({'verdict': 'pass'}))\n";

#[test]
fn a_targeted_update_puts_every_field_it_writes_in_front_of_the_check() {
    let recorded = tempfile::tempdir().expect("a directory for the record");
    let record = recorded.path().join("payloads.jsonl");
    let store = Store::with_policy(Some(json!({
        "check_command": ["python3", "-c", PAYLOAD_CHECK, record.display().to_string()],
    })));
    for id in ["held", "other", "dep"] {
        store.record(
            "site",
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo"),
            PLAIN,
        );
    }
    let body = store.body("Content marker.");
    store.ok(&[
        "task",
        "update",
        "site:held",
        "--title",
        "Title marker",
        "--body-file",
        &body,
        "--status",
        "in-progress",
        "--status-name",
        "in progress",
        "--priority",
        "high",
        "--metadata",
        "team.note=\"Metadata marker\"",
        "--delivers",
        "site:other",
        "--depends-on",
        "site:dep",
    ]);
    let payloads: Vec<Value> = std::fs::read_to_string(&record)
        .expect("the check was run")
        .lines()
        .map(|line| serde_json::from_str(line).expect("a payload"))
        .collect();
    let update = payloads
        .iter()
        .find(|payload| {
            payload["text"]
                .as_array()
                .is_some_and(|text| text.iter().any(|entry| entry == "Content marker."))
        })
        .unwrap_or_else(|| panic!("no check was shown the new content: {payloads:#?}"));
    let metadata: Vec<&str> = update["metadata"]
        .as_array()
        .expect("metadata strings")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for shown in [
        "Title marker",
        "in progress",
        "high",
        "team.note",
        "Metadata marker",
        "dep",
    ] {
        assert!(
            metadata.iter().any(|entry| entry.contains(shown)),
            "{shown} was not shown to the check: {update:#}"
        );
    }
    assert!(
        metadata.iter().any(|entry| entry.contains("other")),
        "the delivered task was not shown to the check: {update:#}"
    );
}

#[test]
fn a_create_routed_by_classification_lands_in_the_private_source_and_a_refused_one_files_nothing() {
    let store = Store::new(
        Some(registered),
        json!({
            "plan": {"routes": [{"classification": "private", "to": "vault"}]},
            "site": {"routes": [
                {"repositories": ["github.com/openco/*"], "to": "elsewhere"},
                {"classification": "private", "to": "vault"},
            ]},
            "elsewhere": {"plugin": "local-md", "config": {"root": "elsewhere"}, "visibility": "public"},
        }),
    );
    std::fs::create_dir_all(store.folder("elsewhere")).expect("the folder");
    store.record(
        "plan",
        "projects",
        "home",
        "title: Home\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record("site", "projects", "pub", "title: Pub\nstatus: todo", PLAIN);
    let body = store.body(PLAIN);
    // A private task created into `plan` goes where its classification routes it, under the
    // home's member project there, ahead of any repository entry.
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "home",
        "--title",
        "Secret",
        "--body-file",
        &body,
        "--classification",
        "private",
        "--repository",
        OPEN,
        "--json",
    ]))
    .expect("a created task");
    let id = created["task"]["id"]
        .as_str()
        .or_else(|| created["items"][0]["id"].as_str())
        .unwrap_or_else(|| panic!("an id: {created:#}"))
        .to_owned();
    assert!(id.starts_with("vault:"), "{created:#}");
    assert!(one_file(&store, "vault", "projects").contains("classification: private"));
    // A routed create the check refuses is refused before its member project is written: the
    // repository entry would send it to `elsewhere`, which then holds nothing at all.
    let before = store.tree();
    let (kind, said) = store.refused(&[
        "task",
        "create",
        "site",
        "--project",
        "pub",
        "--title",
        "Port hiddenco/quietharbor",
        "--body-file",
        &body,
        "--repository",
        OPEN,
    ]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(tree(&store.folder("elsewhere")).is_empty());
    assert_eq!(store.tree(), before);
}

#[test]
fn a_markdown_record_whose_classification_is_malformed_is_refused_rather_than_read_public() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "projects",
        "odd",
        "title: Odd\nstatus: todo\nclassification: secret",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "odd",
        "title: Odd\nstatus: todo\nclassification: secret",
        PLAIN,
    );
    store.record(
        "plan",
        "documents",
        "odd",
        "title: Odd\nclassification: secret",
        PLAIN,
    );
    for kind in ["task", "project", "document"] {
        let output = store.run(&[kind, "show", "plan:odd"]);
        assert_ne!(output.status.code(), Some(0), "{kind}: {}", stdout(&output));
        assert!(
            stderr(&output).contains("classification"),
            "{kind}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_private_project_replaced_by_a_create_naming_public_or_nothing_stays_private() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "vault",
        "projects",
        "goal",
        "title: Goal\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let body = store.body(PLAIN);
    for classification in [None, Some("public")] {
        let mut arguments = vec![
            "project",
            "create",
            "vault",
            "--id",
            "goal",
            "--title",
            "Replaced",
            "--body-file",
            &body,
        ];
        if let Some(classification) = classification {
            arguments.extend(["--classification", classification]);
        }
        store.ok(&arguments);
        let replaced = one_file(&store, "vault", "projects");
        assert!(replaced.contains("title: Replaced"), "{replaced}");
        assert!(
            replaced.contains("classification: private"),
            "{classification:?}: {replaced}"
        );
    }
}

#[test]
fn a_store_declaring_every_source_unknown_and_naming_no_policy_stays_inactive() {
    // `unknown` is what an absent declaration means, so writing it out opts in to nothing.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    let notes = sandbox.subdirectory("notes");
    sandbox.project_document(
        &json!({"sources": {
            "plan": {"plugin": "local-md", "config": {"root": plan}, "visibility": "unknown"},
            "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "unknown"},
        }})
        .to_string(),
    );
    let store = Store {
        sandbox,
        onevcs: None,
    };
    // Active, an unanswered repository would make this private and no check would pass it.
    store.record(
        "plan",
        "tasks",
        "named",
        &format!("title: Named\nstatus: todo\nrepositories: [{A}]"),
        A_TEXT,
    );
    store.ok(&["task", "copy", "plan:named", "--to", "notes"]);
    assert!(!one_file(&store, "notes", "tasks").contains("classification"));
    // And an explicitly private item still has nowhere to go.
    store.record(
        "plan",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        PLAIN,
    );
    let (kind, said) = store.refused(&["task", "copy", "plan:secret", "--to", "notes"]);
    assert_eq!(kind, "not-private-destination", "{said}");
}

#[test]
fn a_copy_over_an_item_the_destination_holds_private_keeps_it_private() {
    let store = Store::new(Some(registered), json!({}));
    // Filed under no project, so nothing but the destination's own record says private.
    store.record("plan", "tasks", "open", "title: Open\nstatus: todo", PLAIN);
    store.record("plan", "documents", "design", "title: Design", PLAIN);
    store.ok(&["task", "copy", "plan:open", "--to", "vault"]);
    store.ok(&["document", "copy", "plan:design", "--to", "vault"]);
    // Somebody marks what landed private, at the destination alone.
    for kind in ["tasks", "documents"] {
        let files: Vec<String> = files_under(&store, "vault")
            .into_iter()
            .filter(|path| path.contains(kind))
            .collect();
        assert_eq!(files.len(), 1, "{kind}: {files:?}");
        let text = std::fs::read_to_string(&files[0]).expect("a record");
        std::fs::write(
            &files[0],
            text.replacen("---\n", "---\nclassification: private\n", 1),
        )
        .expect("marked private");
    }
    // A copy of the same public items again updates them, and loosens nothing.
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open, revised\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "documents",
        "design",
        "title: Design, revised",
        PLAIN,
    );
    store.ok(&["task", "copy", "plan:open", "--to", "vault"]);
    store.ok(&["document", "copy", "plan:design", "--to", "vault"]);
    for kind in ["tasks", "documents"] {
        let held = one_file(&store, "vault", kind);
        assert!(held.contains("revised"), "{kind}: {held}");
        assert!(held.contains("classification: private"), "{kind}: {held}");
    }
}

#[test]
fn a_policy_command_with_no_program_is_refused_when_the_configuration_is_read() {
    for command in [json!([]), json!(["   ", "--flag"])] {
        for key in ["check_command", "visibility_command"] {
            let store = Store::with_policy(Some(json!({ key: command })));
            let output = store.run(&["sources", "list"]);
            assert_ne!(
                output.status.code(),
                Some(0),
                "{key} {command}: {}",
                stdout(&output)
            );
            assert!(
                stderr(&output).contains("names the program to run"),
                "{key} {command}: {}",
                stderr(&output)
            );
        }
    }
}

#[test]
fn a_delivered_task_the_boundary_refuses_is_reported_failed_and_left_as_it_was() {
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({ "shelf": folder(sandbox, "shelf", "public") }),
    );
    // Held private by hand in a public source, as no write here could leave one: the rule may
    // neither name its deliverer there nor move its status.
    store.record(
        "site",
        "tasks",
        "unnamed",
        "title: Unnamed\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "named",
        "title: Named\nstatus: todo\nclassification: private\ndelivered_by: [\"shelf:helper\"]",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "helper",
        "title: Helper\nstatus: todo\ndelivers: [\"site:unnamed\", \"site:named\"]",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let output = store.run(&[
        "task",
        "status",
        "set",
        "shelf:helper",
        "in-progress",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let set: Value = serde_json::from_str(&stdout(&output)).expect("a status answer");
    let delivered = set["delivered"]
        .as_array()
        .expect("one entry per delivered task");
    assert_eq!(delivered.len(), 2, "{set:#}");
    for entry in delivered {
        // `unnamed` is refused writing its `delivered_by`; `named` already lists the deliverer,
        // so its refusal is the status write's.
        assert_eq!(entry["outcome"], "failed", "{set:#}");
        assert_eq!(
            entry["failure"]["kind"], "not-private-destination",
            "{set:#}"
        );
    }
    assert_eq!(tree(&store.folder("site")), before, "both are as they were");
    // The deliverer's own write landed.
    let helper =
        std::fs::read_to_string(store.folder("shelf").join("tasks/helper.md")).expect("the helper");
    assert!(helper.contains("status: in progress"), "{helper}");
}

#[test]
fn a_narrow_write_to_a_record_of_a_private_repository_held_by_a_public_source_is_refused() {
    let store = Store::new(Some(registered), json!({}));
    // Neither is classified private by its own record: one names a private repository.
    store.record(
        "site",
        "tasks",
        "closed",
        &format!("title: Closed\nstatus: todo\nrepositories: [{A}]"),
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "open",
        &format!("title: Open\nstatus: todo\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["task", "status", "set", "site:closed", "done"]);
    assert_eq!(kind, "not-private-destination", "{said}");
    assert_eq!(store.tree(), before, "nothing was written");
    store.ok(&["task", "status", "set", "site:open", "done"]);
}

#[test]
fn a_copy_withholds_a_carried_private_item_s_id_though_its_source_is_not_declared_private() {
    // `shelf` and `notes` declare nothing, so only `secret`'s own classification says its id
    // must not reach `site`.
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}},
            "notes": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("notes")}},
            "site": {"routes": [{"classification": "private", "to": "vault"}]},
        })
    });
    store.record(
        "shelf",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "plain",
        "title: Plain\nstatus: todo",
        PLAIN,
    );
    store.record(
        "notes",
        "tasks",
        "open",
        "title: Open\nstatus: todo\n\
         depends_on: [{id: \"shelf:secret\", item: task}, {id: \"shelf:plain\", item: task}]\n\
         delivers: [\"shelf:secret\"]",
        PLAIN,
    );
    let report: Value = serde_json::from_str(&store.ok(&[
        "task",
        "copy",
        "notes:open",
        "shelf:secret",
        "--to",
        "site",
        "--json",
    ]))
    .expect("a copy report");
    let placed = |source: &str| {
        report["items"]
            .as_array()
            .expect("outcomes")
            .iter()
            .find(|item| item["source"] == source)
            .unwrap_or_else(|| panic!("{source}: {report:#}"))["destination"]
            .as_str()
            .expect("a destination")
            .to_owned()
    };
    let secret_at = placed("shelf:secret");
    assert!(secret_at.starts_with("vault:"), "{report:#}");
    let landed = held_text(&store, "site");
    // Neither the id it was read under nor the one it landed under reaches the public write…
    assert!(!landed.contains("shelf:secret"), "{landed}");
    assert!(!landed.contains(&secret_at), "{landed}");
    // …while the edge to a public item outside the copy is written as before.
    assert!(landed.contains("shelf:plain"), "{landed}");
    // A destination declared private keeps both.
    store.ok(&["task", "copy", "notes:open", "--to", "vault"]);
    let kept = held_text(&store, "vault");
    assert!(kept.contains("shelf:secret"), "{kept}");
    assert!(kept.contains("shelf:plain"), "{kept}");
}

#[test]
fn a_copy_withholds_an_external_private_item_s_id_though_its_source_declares_nothing() {
    // `shelf` declares nothing; `secret` is private by its own record and is not copied.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.record(
        "shelf",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "plain",
        "title: Plain\nstatus: todo",
        PLAIN,
    );
    store.record(
        "shelf",
        "projects",
        "hidden-plan",
        "title: Hidden plan\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open\nstatus: todo\n\
         depends_on: [{id: \"shelf:secret\", item: task}, {id: \"shelf:plain\", item: task}, \
         {id: \"shelf:hidden-plan\", item: project}]\n\
         delivers: [\"shelf:secret\", \"shelf:plain\"]",
        PLAIN,
    );
    // Keeping the private delivered task in step from a public deliverer would write that
    // deliverer onto a private item a source declaring nothing holds, so it is reported failed —
    // exit 4 — while the copy itself lands. From a private deliverer there is nothing to write:
    // its own id is withheld from `shelf` in turn.
    let copied = |to: &str, failed_on: Option<&str>| {
        let output = store.run(&["task", "copy", "plan:open", "--to", to, "--json"]);
        let report: Value = serde_json::from_str(&stdout(&output)).expect("a copy report");
        let failed: Vec<&Value> = report["delivered"]
            .as_array()
            .expect("delivered")
            .iter()
            .filter(|entry| entry["outcome"] == "failed")
            .collect();
        match failed_on {
            Some(ticket) => {
                assert_eq!(output.status.code(), Some(4), "{report:#}");
                assert_eq!(failed.len(), 1, "{report:#}");
                assert_eq!(failed[0]["ticket"], ticket, "{report:#}");
            }
            None => {
                assert_eq!(output.status.code(), Some(0), "{report:#}");
                assert!(failed.is_empty(), "{report:#}");
            }
        }
    };
    copied("site", Some("shelf:secret"));
    let landed = held_text(&store, "site");
    assert!(!landed.contains("shelf:secret"), "{landed}");
    // A project it depends on is read and withheld the same way.
    assert!(!landed.contains("shelf:hidden-plan"), "{landed}");
    // Public-to-public references are written as before.
    assert!(landed.contains("shelf:plain"), "{landed}");
    // A destination declared private keeps the private one.
    copied("vault", None);
    let kept = held_text(&store, "vault");
    assert!(kept.contains("shelf:secret"), "{kept}");
    assert!(kept.contains("shelf:hidden-plan"), "{kept}");
    assert!(kept.contains("shelf:plain"), "{kept}");
}

#[test]
fn a_copy_whose_external_reference_is_missing_or_unreadable_is_refused_before_any_write() {
    // A configured source declaring nothing that holds no such item, and one whose record of it
    // cannot be read, refuse exactly as an unconfigured source does.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.record(
        "shelf",
        "tasks",
        "broken",
        "title: Broken\nstatus: todo\nclassification: secret",
        PLAIN,
    );
    // A task read whole whose project cannot be read is no more classified than one unread.
    store.record(
        "shelf",
        "projects",
        "broken-plan",
        "title: Broken plan\nstatus: todo\nclassification: secret",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "filed",
        "title: Filed\nstatus: todo\nproject: broken-plan",
        PLAIN,
    );
    for (id, far, why) in [
        ("missing", "shelf:gone-1", "holds no such item"),
        ("unreadable", "shelf:broken", "could not be read"),
        ("unreadable-project", "shelf:filed", "classification"),
    ] {
        store.record(
            "plan",
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo\ndepends_on: [{{id: \"{far}\", item: task}}]"),
            PLAIN,
        );
        let before = store.tree();
        let (kind, said) = store.refused(&["task", "copy", &format!("plan:{id}"), "--to", "site"]);
        assert_eq!(kind, "reference-unclassified", "{said}");
        assert!(said.contains(far) && said.contains(why), "{said}");
        assert_eq!(store.tree(), before, "nothing was written");
    }
}

#[test]
fn a_copy_whose_external_reference_cannot_be_classified_is_refused_before_any_write() {
    // `nowhere` is no configured source, so nothing can say whether its item is private.
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open\nstatus: todo\ndepends_on: [{id: \"nowhere:x-1\", item: task}]",
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["task", "copy", "plan:open", "--to", "site"]);
    assert_eq!(kind, "reference-unclassified", "{said}");
    assert!(said.contains("nowhere:x-1"), "{said}");
    assert_eq!(store.tree(), before, "nothing was written");
    // A destination declared private needs no answer, and takes it.
    store.ok(&["task", "copy", "plan:open", "--to", "vault"]);
}

#[test]
fn a_copy_withholds_an_external_reference_private_only_by_its_project_or_its_members() {
    // `shelf` declares nothing, and none of the three referenced items is private by its own
    // record: `legacy` is filed under a private project, `mixed` holds a private task, and
    // `documented` a private document.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.record(
        "shelf",
        "projects",
        "closed-plan",
        "title: Closed plan\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "legacy",
        "title: Legacy\nstatus: todo\nproject: closed-plan",
        PLAIN,
    );
    store.record(
        "shelf",
        "projects",
        "mixed",
        "title: Mixed\nstatus: todo",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "inner",
        "title: Inner\nstatus: todo\nproject: mixed\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "projects",
        "documented",
        "title: Documented\nstatus: todo",
        PLAIN,
    );
    store.record(
        "shelf",
        "documents",
        "notes",
        "title: Notes\nproject: documented\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "projects",
        "open-plan",
        "title: Open plan\nstatus: todo",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "plain",
        "title: Plain\nstatus: todo\nproject: open-plan",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "open",
        "title: Open\nstatus: todo\n\
         depends_on: [{id: \"shelf:legacy\", item: task}, {id: \"shelf:mixed\", item: project}, \
         {id: \"shelf:documented\", item: project}, {id: \"shelf:plain\", item: task}, \
         {id: \"shelf:open-plan\", item: project}]",
        PLAIN,
    );
    store.ok(&["task", "copy", "plan:open", "--to", "site"]);
    let landed = held_text(&store, "site");
    for private in ["shelf:legacy", "shelf:mixed", "shelf:documented"] {
        assert!(!landed.contains(private), "{private}: {landed}");
    }
    // References to public items under a public project are written as before.
    assert!(landed.contains("shelf:plain"), "{landed}");
    assert!(landed.contains("shelf:open-plan"), "{landed}");
    // A destination declared private keeps every one.
    store.ok(&["task", "copy", "plan:open", "--to", "vault"]);
    let kept = held_text(&store, "vault");
    for named in [
        "shelf:legacy",
        "shelf:mixed",
        "shelf:documented",
        "shelf:plain",
        "shelf:open-plan",
    ] {
        assert!(kept.contains(named), "{named}: {kept}");
    }
}

#[test]
fn a_narrow_write_to_a_record_a_public_source_does_not_hold_is_refused_writing_nothing() {
    let store = Store::new(Some(registered), json!({}));
    store.record("site", "tasks", "fine", "title: Fine\nstatus: todo", PLAIN);
    let before = store.tree();
    for (arguments, named) in [
        (
            vec!["task", "status", "set", "site:gone-1", "done"],
            "site:gone-1",
        ),
        (
            vec![
                "project",
                "metadata",
                "set",
                "site:gone-2",
                "team.note",
                "\"x\"",
            ],
            "gone-2",
        ),
        (
            vec![
                "document",
                "metadata",
                "set",
                "site:gone-3",
                "team.note",
                "\"x\"",
            ],
            "gone-3",
        ),
    ] {
        let output = store.run(&arguments);
        assert_ne!(output.status.code(), Some(0), "{arguments:?}");
        assert!(stderr(&output).contains(named), "{}", stderr(&output));
    }
    assert_eq!(store.tree(), before, "nothing was written");
}

#[test]
fn a_reference_a_caller_names_to_an_item_private_by_record_or_inheritance_is_refused() {
    // `shelf` declares nothing: what makes each item private is its record, its project or
    // its members, and what makes `broken` unclassifiable is a record that does not parse.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.public_project_p();
    store.record(
        "shelf",
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "projects",
        "closed-plan",
        "title: Closed plan\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "legacy",
        "title: Legacy\nstatus: todo\nproject: closed-plan",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "broken",
        "title: Broken\nstatus: todo\nclassification: secret",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "plain",
        "title: Plain\nstatus: todo",
        PLAIN,
    );
    store.record("site", "tasks", "fine", "title: Fine\nstatus: todo", PLAIN);
    let body = store.body(PLAIN);
    let before = store.tree();
    for (far, kind) in [
        ("shelf:secret", "private-reference"),
        ("shelf:legacy", "private-reference"),
        ("shelf:broken", "reference-unclassified"),
    ] {
        for arguments in [
            vec![
                "task",
                "create",
                "site",
                "--project",
                "p",
                "--title",
                "T",
                "--body-file",
                &body,
                "--depends-on",
                far,
            ],
            vec!["task", "update", "site:fine", "--delivers", far],
            vec!["task", "update", "site:fine", "--depends-on", far],
        ] {
            let (refused, said) = store.refused(&arguments);
            assert_eq!(refused, kind, "{arguments:?}: {said}");
            assert!(said.contains(far), "{said}");
        }
    }
    assert_eq!(store.tree(), before, "no refused write left anything");
    // A public item of the same source is named as before.
    store.ok(&["task", "update", "site:fine", "--depends-on", "shelf:plain"]);
    assert!(held_text(&store, "site").contains("shelf:plain"));
}

#[test]
fn a_narrow_write_to_an_item_private_by_inheritance_held_by_a_public_source_is_refused() {
    let store = Store::new(Some(registered), json!({}));
    // Held there by hand: none of the three is private by its own record.
    store.record(
        "site",
        "projects",
        "closed-plan",
        "title: Closed plan\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "filed",
        "title: Filed\nstatus: todo\nproject: closed-plan",
        PLAIN,
    );
    store.record(
        "site",
        "documents",
        "filed-notes",
        "title: Filed notes\nproject: closed-plan",
        PLAIN,
    );
    store.record(
        "site",
        "projects",
        "mixed",
        "title: Mixed\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "inner",
        "title: Inner\nstatus: todo\nproject: mixed\nclassification: private",
        PLAIN,
    );
    store.record(
        "site",
        "projects",
        "open-plan",
        "title: Open plan\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "open",
        "title: Open\nstatus: todo\nproject: open-plan",
        PLAIN,
    );
    let before = store.tree();
    for arguments in [
        vec!["task", "status", "set", "site:filed", "done"],
        vec![
            "document",
            "metadata",
            "set",
            "site:filed-notes",
            "team.note",
            "\"x\"",
        ],
        vec![
            "project",
            "metadata",
            "set",
            "site:mixed",
            "team.note",
            "\"x\"",
        ],
    ] {
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "not-private-destination", "{arguments:?}: {said}");
    }
    assert_eq!(store.tree(), before, "nothing was written");
    // A public task under a public project is written as before.
    store.ok(&["task", "status", "set", "site:open", "done"]);
}

#[test]
fn a_delivered_task_private_only_by_its_project_is_reported_failed_and_left_as_it_was() {
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({ "shelf": folder(sandbox, "shelf", "public") }),
    );
    // Held by hand under a private project in a public source; neither ticket's own record
    // says private.
    store.record(
        "site",
        "projects",
        "closed-plan",
        "title: Closed plan\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "unnamed",
        "title: Unnamed\nstatus: todo\nproject: closed-plan",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "named",
        "title: Named\nstatus: todo\nproject: closed-plan\ndelivered_by: [\"shelf:helper\"]",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "helper",
        "title: Helper\nstatus: todo\ndelivers: [\"site:unnamed\", \"site:named\"]",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let output = store.run(&[
        "task",
        "status",
        "set",
        "shelf:helper",
        "in-progress",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let set: Value = serde_json::from_str(&stdout(&output)).expect("a status answer");
    let delivered = set["delivered"]
        .as_array()
        .expect("one entry per delivered task");
    assert_eq!(delivered.len(), 2, "{set:#}");
    for entry in delivered {
        // `unnamed`'s refusal is its `delivered_by` write's; `named`'s, its status write's.
        assert_eq!(entry["outcome"], "failed", "{set:#}");
        assert_eq!(
            entry["failure"]["kind"], "not-private-destination",
            "{set:#}"
        );
    }
    assert_eq!(tree(&store.folder("site")), before, "both are as they were");
}

#[test]
fn a_project_replaced_over_one_holding_a_private_member_is_held_private() {
    let store = Store::new(Some(registered), json!({}));
    // Public by its own record, private by what it holds.
    store.record(
        "site",
        "projects",
        "mixed",
        "title: Mixed\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "inner",
        "title: Inner\nstatus: todo\nproject: mixed\nclassification: private",
        PLAIN,
    );
    let body = store.body(PLAIN);
    let before = store.tree();
    let (kind, said) = store.refused(&[
        "project",
        "create",
        "site",
        "--id",
        "mixed",
        "--title",
        "Replaced",
        "--body-file",
        &body,
    ]);
    assert_eq!(kind, "not-private-destination", "{said}");
    assert_eq!(store.tree(), before, "nothing was written");
}

#[test]
fn an_external_task_naming_a_project_its_source_does_not_hold_is_refused_before_any_write() {
    // `stray` names a project `shelf` does not hold, so what it inherits cannot be read: it is
    // not classified by ignoring that project. `alone` names none, and its own record decides.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.record(
        "shelf",
        "tasks",
        "stray",
        "title: Stray\nstatus: todo\nproject: gone-plan",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "alone",
        "title: Alone\nstatus: todo",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "dangling",
        "title: Dangling\nstatus: todo\ndepends_on: [{id: \"shelf:stray\", item: task}]",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "standalone",
        "title: Standalone\nstatus: todo\ndepends_on: [{id: \"shelf:alone\", item: task}]",
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["task", "copy", "plan:dangling", "--to", "site"]);
    assert_eq!(kind, "reference-unclassified", "{said}");
    assert!(
        said.contains("shelf:stray") && said.contains("gone-plan"),
        "{said}"
    );
    assert_eq!(store.tree(), before, "nothing was written");
    store.ok(&["task", "copy", "plan:standalone", "--to", "site"]);
    assert!(held_text(&store, "site").contains("shelf:alone"));
}

#[test]
fn a_task_filed_under_a_project_its_source_does_not_hold_reaches_only_a_private_destination() {
    // What `stray` inherits cannot be read, so it is unclassified: never copied as public, and
    // never remembered as public for `plan:dangling`'s reference to it later in the same copy.
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({"shelf": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("shelf")}}}),
    );
    store.record(
        "shelf",
        "tasks",
        "stray",
        "title: Stray\nstatus: todo\nproject: gone-plan",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "dangling",
        "title: Dangling\nstatus: todo\ndepends_on: [{id: \"shelf:stray\", item: task}]",
        PLAIN,
    );
    let before = store.tree();
    for ids in [vec!["shelf:stray"], vec!["shelf:stray", "plan:dangling"]] {
        let mut arguments = vec!["task", "copy"];
        arguments.extend(ids);
        arguments.extend(["--to", "site"]);
        let (kind, said) = store.refused(&arguments);
        assert_eq!(kind, "project-unclassified", "{said}");
        assert!(said.contains("shelf:gone-plan"), "{said}");
        assert_eq!(store.tree(), before, "nothing was written");
    }
    // A destination declared private takes it, held private.
    store.ok(&["task", "copy", "shelf:stray", "--to", "vault"]);
    assert!(held_text(&store, "vault").contains("Stray"));
}

#[test]
fn a_narrow_write_or_a_delivery_to_a_task_of_a_project_its_source_does_not_hold_writes_nothing() {
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "site",
        "tasks",
        "loose",
        "title: Loose\nstatus: todo\nproject: gone-plan",
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "worker",
        "title: Worker\nstatus: todo\ndelivers: [\"site:loose\"]",
        PLAIN,
    );
    let site = || tree(&store.folder("site"));
    let before = site();
    let (kind, said) = store.refused(&["task", "status", "set", "site:loose", "done"]);
    assert_eq!(kind, "project-unclassified", "{said}");
    assert!(said.contains("site:gone-plan"), "{said}");
    let (kind, said) = store.refused(&[
        "task",
        "metadata",
        "set",
        "site:loose",
        "acme.note",
        "\"x\"",
    ]);
    assert_eq!(kind, "project-unclassified", "{said}");
    assert_eq!(site(), before, "nothing was written to site");

    // The deliverer moves; the ticket it delivers is unclassified, so it is failed and left.
    let output = store.run(&[
        "task",
        "status",
        "set",
        "plan:worker",
        "in-progress",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let set: Value = serde_json::from_str(&stdout(&output)).expect("a status answer");
    assert_eq!(set["delivered"][0]["outcome"], "failed", "{set:#}");
    assert_eq!(site(), before, "nothing was written to site");
}

#[test]
fn a_document_created_public_under_a_private_project_is_held_private_and_refused_onto_a_public_one()
{
    // A document inherits its project's classification on create: an explicit `public` cannot
    // loosen it where it lands, and a public source refuses it before writing anything.
    let store = Store::new(Some(registered), json!({}));
    for folder in ["vault", "site"] {
        store.record(
            folder,
            "projects",
            "goal",
            "title: Goal\nstatus: todo\nclassification: private",
            PLAIN,
        );
    }
    let body = store.body(PLAIN);
    let create = |source: &'static str| {
        vec![
            "document",
            "create",
            source,
            "--project",
            "goal",
            "--id",
            "notes",
            "--title",
            "Notes",
            "--body-file",
            body.as_str(),
            "--classification",
            "public",
        ]
    };
    store.ok(&create("vault"));
    let held = std::fs::read_to_string(store.folder("vault").join("documents/notes.md"))
        .expect("the created document");
    assert!(held.contains("classification: private"), "{held}");

    let before = store.tree();
    let (kind, said) = store.refused(&create("site"));
    assert_eq!(kind, "not-private-destination", "{said}");
    assert_eq!(store.tree(), before, "nothing was written");
}

/// One item of the loopback board, in the public repository `openco/openwidget`, filed under
/// `parent`; a project when `project` is set, and declaring `classification` when one is given.
fn board_item(
    id: &str,
    parent: Option<&str>,
    project: bool,
    classification: Option<&str>,
) -> Value {
    let mut slot = serde_json::Map::new();
    if project {
        slot.insert("onetaskgraph.item_kind".to_owned(), json!("project"));
    }
    if let Some(classification) = classification {
        slot.insert(
            "onetaskgraph.classification".to_owned(),
            json!(classification),
        );
    }
    let body = if slot.is_empty() {
        PLAIN.to_owned()
    } else {
        format!(
            "{PLAIN}\n\n<!-- onetaskgraph.metadata\n{}\n-->",
            Value::Object(slot)
        )
    };
    json!({"item": format!("ITEM-{id}"), "id": id, "type": "Issue", "title": id, "body": body,
           "state": "OPEN", "reason": null, "parent": parent, "repo": "openco/openwidget",
           "status": "Todo", "origin": "", "labels": []})
}

/// The store, beside a loopback GitHub board `board` that declares nothing and holds the public
/// project `HOME-1` with the two tasks `W-1` and `W-2` filed under it.
fn store_over_board() -> (Store, crate::fixtures::GitHubBoardFields) {
    let mut handle = None;
    let store = Store::laid(Some(registered), |sandbox| {
        let (config, board) = crate::fixtures::github_projects_with_items(
            sandbox,
            vec![
                board_item("HOME-1", None, true, None),
                board_item("W-1", Some("HOME-1"), false, None),
                board_item("W-2", Some("HOME-1"), false, None),
            ],
        );
        handle = Some(board);
        json!({"board": {"plugin": "github-projects", "config": config}})
    });
    (store, handle.expect("the board"))
}

/// The board's read of one item by its id, and nothing else it sends: a listing of a project's
/// members carries `$after` between these.
const ITEM_READ: &str = "query($id:ID!,$first:Int!,$nestedFirst:Int!";

#[test]
fn a_public_classification_cached_before_its_project_changed_is_never_trusted() {
    // One copy reads `W-1`, then `HOME-1` and its members — classifying the project public and
    // keeping that for the rest of the command — then `W-2` itself, or `W-2` as what `plan:ref`
    // names. Before that the board deletes `HOME-1`, refuses to read it, or has it made private:
    // nothing filed under it may then reach `site` on the strength of what was read before. A
    // copy withholds a reference to an item it reads as private, rather than refusing, so the
    // last of these lands without it.
    type Make = fn(&crate::fixtures::GitHubBoardFields);
    let cases: [(&str, Make, [Option<&str>; 2]); 3] = [
        (
            "absent",
            |board| board.replace_after("HOME-1", 2, None),
            [Some("project-unclassified"), Some("reference-unclassified")],
        ),
        (
            "unreadable",
            |board| board.refuse_after(ITEM_READ, 3),
            [Some("refused"), Some("reference-unclassified")],
        ),
        (
            "made private",
            |board| {
                board.replace_after(
                    "HOME-1",
                    2,
                    Some(board_item("HOME-1", None, true, Some("private"))),
                );
            },
            [Some("not-private-destination"), None],
        ),
    ];
    for (gone, make, expected) in cases {
        for (ids, expected) in [
            (["board:W-1", "board:W-2"], expected[0]),
            (["board:W-1", "plan:ref"], expected[1]),
        ] {
            let (store, board) = store_over_board();
            store.record(
                "plan",
                "tasks",
                "ref",
                "title: Ref\nstatus: todo\ndepends_on: [{id: \"board:W-2\", item: task}]",
                PLAIN,
            );
            make(&board);
            let before = store.tree();
            let mut arguments = vec!["task", "copy"];
            arguments.extend(ids);
            arguments.extend(["--to", "site"]);
            let served_before = board.served().len();
            let Some(expected) = expected else {
                store.ok(&arguments);
                let landed = held_text(&store, "site");
                assert!(landed.contains("W-1") && landed.contains("Ref"), "{landed}");
                assert!(!landed.contains("board:W-2"), "{gone} {ids:?}: {landed}");
                let reads: Vec<Value> = board.served()[served_before..]
                    .iter()
                    .filter(|(document, _)| document.contains(ITEM_READ))
                    .map(|(_, variables)| variables["id"].clone())
                    .collect();
                assert_eq!(
                    reads.last(),
                    Some(&json!("HOME-1")),
                    "{gone} {ids:?}: {reads:?}"
                );
                continue;
            };
            let (kind, said) = store.refused(&arguments);
            assert_eq!(kind, expected, "{gone} {ids:?}: {said}");
            assert_eq!(store.tree(), before, "{gone} {ids:?}: nothing was written");
            let served = board.served();
            // What refused it is `HOME-1` asked for again, before its cached answer was used.
            let last = served
                .iter()
                .rfind(|(document, _)| document.contains(ITEM_READ))
                .map(|(_, variables)| variables["id"].clone());
            assert_eq!(last, Some(json!("HOME-1")), "{gone} {ids:?}");
            if gone == "absent" {
                assert!(said.contains("board:HOME-1"), "{gone} {ids:?}: {said}");
            }
            assert!(
                !served
                    .iter()
                    .any(|(document, _)| document.trim_start().starts_with("mutation")),
                "{gone} {ids:?}: the board was sent a mutation"
            );
        }
    }
    // The same copy over a board that keeps answering lands both, public as they read.
    let (store, _board) = store_over_board();
    store.ok(&["task", "copy", "board:W-1", "board:W-2", "--to", "site"]);
    let landed = held_text(&store, "site");
    assert!(landed.contains("W-1") && landed.contains("W-2"), "{landed}");
}

#[test]
fn a_create_under_a_project_held_private_by_its_members_or_not_held_at_all_writes_nothing() {
    // `mixed` reads public by its own record and is private by the task it holds; `gone` is not
    // there to read. Neither may take a new public task or document on `site`.
    let store = Store::new(Some(registered), json!({}));
    store.record(
        "site",
        "projects",
        "mixed",
        "title: Mixed\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "inner",
        "title: Inner\nstatus: todo\nproject: mixed\nclassification: private",
        PLAIN,
    );
    let body = store.body(PLAIN);
    let before = store.tree();
    for (project, expected) in [
        ("mixed", "not-private-destination"),
        ("gone", "project-unclassified"),
    ] {
        for kind in ["task", "document"] {
            let (refused, said) = store.refused(&[
                kind,
                "create",
                "site",
                "--project",
                project,
                "--title",
                "Filed",
                "--body-file",
                &body,
            ]);
            assert_eq!(refused, expected, "{kind} under {project}: {said}");
            assert_eq!(
                store.tree(),
                before,
                "{kind} under {project}: nothing was written"
            );
        }
    }
    // A source declared private takes either, held private under the project that holds a
    // private member.
    store.record(
        "vault",
        "projects",
        "mixed",
        "title: Mixed\nstatus: todo\nclassification: private",
        PLAIN,
    );
    store.ok(&[
        "task",
        "create",
        "vault",
        "--project",
        "mixed",
        "--title",
        "Filed",
        "--body-file",
        &body,
    ]);
}
