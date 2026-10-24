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

// Public projections of a private coordination project: the canonical plan in a private
// destination, neutral views of its public work where its routes send them.
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] The same edge `mod boundary` sits behind in `main.rs`, for its reason: no credential and no network, folders of Markdown, a relay over a pipe and the loopback board, in seconds; what they prove — the copy's projections and the status mirror — is the engine's, which AGENTS.md forbids any plugin crate to depend on, so no plugin's edge is narrower, and the board journeys need the GitHub plugin this suite already links.
mod projections;

/// A private repository, its owner and name as a term check matches them.
const A: &str = "github.com/hiddenco/quietharbor";
const A_TEXT: &str = "Mirror the retry loop hiddenco/quietharbor uses.";
/// A second private repository, under another owner.
const B: &str = "github.com/lanternco/brightwater";
const B_TEXT: &str = "Mirror the retry loop lanternco/brightwater uses.";
/// A public repository.
const OPEN: &str = "github.com/openco/openwidget";
/// A second public repository, under an owner no route names.
const SHOP: &str = "github.com/shopco/storefront";
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

/// A public record's text, less the one reserved key a public projection project carries: the
/// random allocation it was created behind, which names nothing private.
fn public_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("onetaskgraph.allocation:", "allocation:")
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
        .public("openco", "openwidget")
        .public("shopco", "storefront");
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
fn a_comment_a_copy_would_carry_is_held_to_the_boundary_with_the_task_it_is_on() {
    let store = Store::new(Some(registered), json!({}));
    // One comment each, written in the folder's own section: one naming a private repository
    // in what it says, and one whose author does.
    let commented = |id: &str, author: &str, said: &str| {
        store.record(
            "plan",
            "tasks",
            id,
            "title: Clean\nstatus: todo",
            &format!(
                "{PLAIN}\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" \
                 author=\"{author}\" created_at=\"2026-09-13T15:11:07Z\" \
                 updated_at=\"2026-09-13T15:11:07Z\" -->\n### {author} — 2026-09-13T15:11:07Z\n\n\
                 {said}\n\n<!-- /onetaskgraph:comment -->"
            ),
        );
    };
    commented("said", "ada", A_TEXT);
    commented("signed", "hiddenco/quietharbor", PLAIN);
    let before = store.tree();
    for (task, where_) in [("plan:said", "text"), ("plan:signed", "metadata")] {
        let (kind, said) = store.refused(&["task", "copy", task, "--to", "site", "--comments"]);
        assert_eq!(kind, "boundary-refused", "{task}: {said}");
        assert!(
            said.contains(&format!("term of a private repository in its {where_}"))
                && !said.contains("quietharbor"),
            "{task}: the refusal is neutral: {said}"
        );
    }
    assert_eq!(store.tree(), before, "no refused copy left anything");
    // Without --comments no comment is carried, so none is held to the boundary.
    store.ok(&["task", "copy", "plan:said", "--to", "site"]);
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
    // The public half is seen in public only through a neutral view where its repository
    // routes it: one `Public work` project and one task, naming nothing of the private plan.
    let elsewhere = tree(&store.folder("elsewhere"));
    assert_eq!(elsewhere.len(), 2, "{:?}", elsewhere.keys());
    for (path, bytes) in &elsewhere {
        let text = public_text(bytes);
        for named in ["goal", "Private half", "plan:", "vault", "onetaskgraph."] {
            assert!(!text.contains(named), "{path} names {named}:\n{text}");
        }
    }
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
    // `site`, where the public task's own placement is, holds a neutral view of it alone.
    let on_site = tree(&store.folder("site"));
    assert_eq!(
        on_site.len(),
        2,
        "a project and one view: {:?}",
        on_site.keys()
    );
    for (path, bytes) in &on_site {
        let text = public_text(bytes);
        for named in ["goal", "Design", "plan:", "vault", "onetaskgraph."] {
            assert!(!text.contains(named), "{path} names {named}:\n{text}");
        }
    }
    let project = one_file(&store, "vault", "projects");
    assert!(project.contains("classification: private"), "{project}");
    // The task lands beside its private project as public as its own record says: a private
    // project tightens no task it holds.
    let task = one_file(&store, "vault", "tasks");
    assert!(!task.contains("classification: private"), "{task}");
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
    // A public task of that project, copied on its own, is public: a task inherits nothing from
    // its project. It lands without naming the private project it is filed under.
    store.ok(&["task", "copy", "plan:open", "--to", "site"]);
    let landed = one_file(&store, "site", "tasks");
    assert!(landed.contains("Open half"), "{landed}");
    assert!(!landed.contains("goal"), "{landed}");
    assert!(!landed.contains("classification: private"), "{landed}");
    // Its private sibling is still refused.
    let (kind, _) = store.refused(&["task", "copy", "plan:closed", "--to", "site"]);
    assert_eq!(kind, "not-private-destination");
}

/// A plan authored in the public `draft` folder, titled `title`, copied to `site` with
/// `routes`: `open` of the public `openco` repository, `quiet` of the private `hiddenco` one,
/// `review` naming no repository and depending on both, and the plan's design document.
fn private_plan(store: &Store, title: &str) {
    std::fs::create_dir_all(store.folder("draft")).expect("the draft folder");
    store.record(
        "draft",
        "projects",
        "goal",
        &format!("title: {title}\nstatus: todo"),
        PLAIN,
    );
    store.record(
        "draft",
        "tasks",
        "open",
        &format!("title: Open work\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    store.record(
        "draft",
        "tasks",
        "quiet",
        &format!("title: Quiet work\nstatus: todo\nproject: goal\nrepositories: [{A}]"),
        A_TEXT,
    );
    store.record(
        "draft",
        "tasks",
        "review",
        "title: Review it\nstatus: todo\nproject: goal\ndepends_on: [open, quiet]",
        PLAIN,
    );
    store.record(
        "draft",
        "documents",
        "design",
        "title: Design\nproject: goal",
        PLAIN,
    );
}

/// The store [`private_plan`] is copied in: `site` routing `hiddenco` work to `vault`, and
/// `extra` laid over it.
fn routing_store(extra: impl FnOnce(&Sandbox) -> Value) -> Store {
    Store::laid(Some(registered), |sandbox| {
        merged(
            json!({
                "draft": folder(sandbox, "draft", "public"),
                "site": {"routes": [{"repositories": ["github.com/hiddenco/*"], "to": "vault"}]},
            }),
            extra(sandbox),
        )
    })
}

#[test]
fn a_mixed_plan_homes_whole_in_the_private_source_and_its_public_work_is_seen_through_a_view() {
    let store = routing_store(|_| json!({}));
    private_plan(&store, "One goal");
    let report: Value = serde_json::from_str(&store.ok(&[
        "project",
        "copy",
        "draft:goal",
        "--to",
        "site",
        "--json",
    ]))
    .expect("a copy report");
    // Every task lands canonically beside the private home, the public one included.
    let landed = landed_ids(&report.to_string());
    for item in ["draft:goal", "draft:open", "draft:quiet", "draft:review"] {
        assert!(landed[item].starts_with("vault:"), "{item}: {report:#}");
    }
    // The document goes with its home.
    let document =
        landed_ids(&store.ok(&["document", "copy", "draft:design", "--to", "site", "--json"]));
    assert!(
        document["draft:design"].starts_with("vault:"),
        "{document:?}"
    );

    // `site` holds a neutral project and one view of the public task, and nothing else.
    let home = &landed["draft:goal"];
    assert!(
        shown_project(&store, home)["metadata"]
            .get("onetaskgraph.members")
            .is_none(),
        "a view is no member"
    );
    let map = shown_project(&store, home)["metadata"]["onetaskgraph.public_projections"].clone();
    let entries = map["tasks"].as_array().expect("the map's entries");
    assert_eq!(entries.len(), 1, "{map:#}");
    assert_eq!(entries[0]["task"], landed["draft:open"].as_str(), "{map:#}");
    let view = entries[0]["projection"]
        .as_str()
        .expect("a view")
        .to_owned();
    assert!(view.starts_with("site:"), "{map:#}");
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", &view, "--json"])).expect("the view");
    assert_eq!(shown["items"][0]["item"]["title"], "Open work");
    let on_site = files_under(&store, "site");
    assert_eq!(
        on_site.len(),
        2,
        "the view's project and the view: {on_site:?}"
    );
    // Nothing there names the private source, its work or where its home is.
    for (path, bytes) in tree(&store.folder("site")) {
        let text = public_text(&bytes);
        for named in [
            "vault",
            "Quiet work",
            "One goal",
            "hiddenco",
            "draft:",
            "onetaskgraph.",
            "classification: private",
        ] {
            assert!(!text.contains(named), "{path} names {named}:\n{text}");
        }
    }

    // A second copy writes nothing anywhere.
    let before = store.tree();
    let again = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert_eq!(again, landed);
    assert_eq!(
        store.tree(),
        before,
        "a repeat leaves every folder as it was"
    );
}

/// One project read through the binary.
fn shown_project(store: &Store, id: &str) -> Value {
    let answer: Value =
        serde_json::from_str(&store.ok(&["project", "show", id, "--json"])).expect("a project");
    answer["items"][0]["item"].clone()
}

#[test]
fn a_plan_whose_private_work_routes_to_two_private_sources_is_refused_naming_both() {
    let store = routing_store(|_| {
        json!({"site": {"routes": [
            {"repositories": ["github.com/hiddenco/*"], "to": "vault"},
            {"repositories": ["github.com/lanternco/*"], "to": "plan"},
        ]}})
    });
    private_plan(&store, "One goal");
    store.record(
        "draft",
        "tasks",
        "bright",
        &format!("title: Bright work\nstatus: todo\nproject: goal\nrepositories: [{B}]"),
        B_TEXT,
    );
    // A second `hiddenco` task, and the project's own `hiddenco` repository, vote beside the
    // first.
    store.record(
        "draft",
        "tasks",
        "hush",
        &format!("title: Hushed work\nstatus: todo\nproject: goal\nrepositories: [{A}]"),
        A_TEXT,
    );
    store.record(
        "draft",
        "projects",
        "goal",
        &format!("title: One goal\nstatus: todo\nrepositories: [{A}]"),
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(kind, "split-private-home", "{said}");
    // One `<source> for <ids>` entry per destination, each naming every voter placing there.
    let entries: BTreeMap<&str, Vec<&str>> = said
        .lines()
        .next()
        .and_then(|first| first.split_once("no one home: "))
        .map(|(_, split)| split)
        .unwrap_or_default()
        .split("; ")
        .filter_map(|entry| entry.split_once(" for "))
        .map(|(source, ids)| {
            let mut ids: Vec<&str> = ids.split(", ").collect();
            ids.sort_unstable();
            (source, ids)
        })
        .collect();
    assert_eq!(
        entries,
        BTreeMap::from([
            ("plan", vec!["draft:bright"]),
            ("vault", vec!["draft:goal", "draft:hush", "draft:quiet"]),
        ]),
        "{said}"
    );
    assert_eq!(store.tree(), before, "nothing was written anywhere");
}

/// [`routing_store`] with `site` routing `hiddenco` work to `vault` and `lanternco` work to the
/// private `archive`, and `draft` declared `draft_visibility`.
fn two_private_routes(draft_visibility: &'static str) -> Store {
    routing_store(move |sandbox| {
        json!({
            "draft": {"visibility": draft_visibility},
            "archive": folder(sandbox, "archive", "private"),
            "site": {"routes": [
                {"repositories": ["github.com/hiddenco/*"], "to": "vault"},
                {"repositories": ["github.com/lanternco/*"], "to": "archive"},
            ]},
        })
    })
}

/// Add to [`private_plan`] the task `bright`, of the private `lanternco` repository, whose
/// route sends it to `archive` — a second private source beside the `vault` `quiet` routes to.
fn add_bright(store: &Store) {
    store.record(
        "draft",
        "tasks",
        "bright",
        &format!("title: Bright work\nstatus: todo\nproject: goal\nrepositories: [{B}]"),
        B_TEXT,
    );
}

#[test]
fn an_existing_private_home_stays_the_home_when_its_private_work_later_routes_two_ways() {
    // Authored in a private folder, so the link to its private home is recorded on it.
    let store = two_private_routes("private");
    std::fs::create_dir_all(store.folder("archive")).expect("the archive folder");
    private_plan(&store, "One goal");
    let first = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let home = first["draft:goal"].clone();
    assert!(home.starts_with("vault:"), "{first:?}");
    let site = tree(&store.folder("site"));

    // A second private repository's work now routes to `archive`: a new home would have no one
    // private source, but this plan has a home, and it stays where it is — with every task of
    // the plan canonically beside it.
    add_bright(&store);
    let again = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert_eq!(again["draft:goal"], home, "{again:?}");
    assert!(again["draft:bright"].starts_with("vault:"), "{again:?}");
    let projects: Value =
        serde_json::from_str(&store.ok(&["project", "list", "--source", "vault", "--json"]))
            .expect("a listing");
    assert_eq!(
        projects["items"].as_array().expect("projects").len(),
        1,
        "no second home: {projects:#}"
    );
    assert_eq!(tree(&store.folder("site")), site, "site is as it was");
}

#[test]
fn an_existing_public_home_of_a_plan_whose_private_work_routes_two_ways_is_refused_unmoved() {
    let store = two_private_routes("public");
    private_plan(&store, "One goal");
    // Copied while it was public: its home is on `site`.
    let set_aside: Vec<(PathBuf, Vec<u8>)> = ["quiet", "review"]
        .iter()
        .map(|task| {
            let path = store.folder("draft").join(format!("tasks/{task}.md"));
            let held = std::fs::read(&path).expect("the task");
            std::fs::remove_file(&path).expect("the task set aside");
            (path, held)
        })
        .collect();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert!(landed["draft:goal"].starts_with("site:"), "{landed:?}");
    for (path, held) in set_aside {
        std::fs::write(path, held).expect("the task restored");
    }
    add_bright(&store);
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(kind, "not-private-destination", "{said}");
    assert!(said.contains("draft:goal"), "{said}");
    assert_eq!(
        store.tree(),
        before,
        "the existing home and everything else are unchanged"
    );
}

#[test]
fn a_project_s_own_private_repository_votes_its_home_into_the_private_source() {
    let store = routing_store(|_| json!({}));
    private_plan(&store, "One goal");
    // The project names the private repository, and no task does any more.
    std::fs::remove_file(store.folder("draft").join("tasks/quiet.md")).expect("the task removed");
    store.record(
        "draft",
        "tasks",
        "review",
        "title: Review it\nstatus: todo\nproject: goal\ndepends_on: [open]",
        PLAIN,
    );
    store.record(
        "draft",
        "projects",
        "goal",
        &format!("title: One goal\nstatus: todo\nrepositories: [{A}]"),
        PLAIN,
    );
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert!(landed["draft:goal"].starts_with("vault:"), "{landed:?}");
    assert!(landed["draft:review"].starts_with("vault:"), "{landed:?}");
    // The public task lands canonically beside its private home, and is seen on `site` through
    // a neutral view filed under no member of it.
    assert!(landed["draft:open"].starts_with("vault:"), "{landed:?}");
    assert!(
        shown_project(&store, &landed["draft:goal"])["metadata"]
            .get("onetaskgraph.members")
            .is_none()
    );
    assert_eq!(
        files_under(&store, "site").len(),
        2,
        "a neutral project and the public task's view"
    );
}

#[test]
fn a_private_plan_whose_home_already_sits_in_a_public_source_is_refused_and_left_there() {
    let store = routing_store(|_| json!({}));
    private_plan(&store, "One goal");
    // Copied while it was public: its home is on `site`.
    let quiet = store.folder("draft").join("tasks/quiet.md");
    let held = std::fs::read(&quiet).expect("the private task");
    std::fs::remove_file(&quiet).expect("the private task set aside");
    let review = store.folder("draft").join("tasks/review.md");
    let reviewed = std::fs::read(&review).expect("the review");
    std::fs::remove_file(&review).expect("its dependent set aside with it");
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert!(landed["draft:goal"].starts_with("site:"), "{landed:?}");
    // Its private work is back, so its voters would send a new home to `vault`; the one it has
    // stays where it is, and holds no private work.
    std::fs::write(&quiet, held).expect("the private task restored");
    std::fs::write(&review, reviewed).expect("its dependent restored");
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(kind, "not-private-destination", "{said}");
    assert!(said.contains("draft:goal"), "{said}");
    assert_eq!(
        store.tree(),
        before,
        "the existing home and everything else are unchanged"
    );
}

/// A visibility command answering `public` for an `openco` repository and `private` for any
/// other.
const OPENCO_PUBLIC: &str = "import json, sys\n\
    asked = sys.stdin.read()\n\
    print(json.dumps({'visibility': 'public' if 'openco' in asked else 'private'}))\n";

/// A check command refusing a write whose payload names `Plan marker` by its title, and
/// passing any other.
const MARKER_REFUSED: &str = "import json, sys\n\
    if 'Plan marker' in sys.stdin.read():\n\
    \x20   print(json.dumps({'verdict': 'refuse', 'surface': 'title'}))\n\
    \x20   sys.exit(1)\n\
    print(json.dumps({'verdict': 'pass'}))\n";

#[test]
fn a_public_view_its_check_refuses_is_refused_before_anything_of_the_plan_is_written() {
    let marker_store = || {
        let sandbox = Sandbox::new();
        let extra = json!({
            "draft": folder(&sandbox, "draft", "public"),
            "site": {"routes": [{"repositories": ["github.com/hiddenco/*"], "to": "vault"}]},
        });
        Store::configured(
            sandbox,
            None,
            Some(json!({
                "visibility_command": ["python3", "-c", OPENCO_PUBLIC],
                "check_command": ["python3", "-c", MARKER_REFUSED],
            })),
            extra,
        )
    };
    // The home carries the marker in its title, which no public view ever does: the plan lands,
    // its public task seen on `site` through a neutral view.
    let store = marker_store();
    private_plan(&store, "Plan marker");
    store.ok(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(
        files_under(&store, "site").len(),
        2,
        "a project and one view"
    );

    // A public task whose own title carries it is refused before anything is written anywhere.
    let store = marker_store();
    private_plan(&store, "One goal");
    store.record(
        "draft",
        "tasks",
        "open",
        &format!("title: Plan marker work\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let (kind, said) = store.refused(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(
        said.contains("public projection of draft:open"),
        "the refusal names the view and the task it is of: {said}"
    );
    assert!(
        files_under(&store, "site").is_empty(),
        "site holds nothing of the plan"
    );
    assert!(
        files_under(&store, "vault").is_empty(),
        "and nothing of the plan reached its home either"
    );
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
fn a_document_of_a_private_source_is_filed_under_the_home_its_project_s_link_names() {
    // `site` routes `openco` work to `elsewhere`, so a walk for a home would read it too.
    let store = Store::new(
        Some(registered),
        json!({
            "site": {"routes": [{"repositories": ["github.com/openco/*"], "to": "elsewhere"}]},
            "elsewhere": {"plugin": "local-md", "config": {"root": "elsewhere"}, "visibility": "public"},
        }),
    );
    std::fs::create_dir_all(store.folder("elsewhere")).expect("the folder");
    store.record(
        "vault",
        "projects",
        "public-plan",
        "title: Public plan\nstatus: todo",
        PLAIN,
    );
    store.ok(&["project", "copy", "vault:public-plan", "--to", "site"]);
    let home = one_file(&store, "site", "projects");
    assert!(
        !home.contains("vault") && !home.contains("onetaskgraph.origin"),
        "{home}"
    );
    // A folder keeps the id it is given; a board assigns its own. The home is moved to an id
    // of its own, and the link with it, so filing under the source project's id would miss it.
    let source_project = store.folder("vault").join("projects/public-plan.md");
    let project = std::fs::read_to_string(&source_project).expect("the source project");
    assert!(project.contains("site:public-plan"), "{project}");
    std::fs::write(
        &source_project,
        project.replace("site:public-plan", "site:board-7"),
    )
    .expect("the link moved");
    let link = store.folder("site").join("projects/board-7.md");
    std::fs::rename(store.folder("site").join("projects/public-plan.md"), &link)
        .expect("the home moved");
    // A walk of `elsewhere` now fails, on a record it cannot read: the home is found by the
    // link, or the copy is refused.
    store.record("elsewhere", "projects", "unreadable", "title: [", PLAIN);
    store.record(
        "vault",
        "documents",
        "design",
        "title: \"Design: public-plan\"\nproject: public-plan",
        PLAIN,
    );
    store.ok(&["document", "copy", "vault:design", "--to", "site"]);
    let document = one_file(&store, "site", "documents");
    assert!(document.contains("project: board-7"), "{document}");
    assert!(
        !document.contains("vault") && !document.contains("onetaskgraph.origin"),
        "{document}"
    );
    // A second copy updates the document the first filed rather than filing another.
    let source = std::fs::read_to_string(store.folder("vault").join("documents/design.md"))
        .expect("the source document");
    std::fs::write(
        store.folder("vault").join("documents/design.md"),
        source.replace(PLAIN, "Retry with a capped exponential backoff, revised."),
    )
    .expect("the source edited");
    store.ok(&["document", "copy", "vault:design", "--to", "site"]);
    let document = one_file(&store, "site", "documents");
    assert!(document.contains("revised"), "{document}");
    assert!(document.contains("project: board-7"), "{document}");
    // A link naming a home `site` no longer holds is refused naming that project.
    std::fs::remove_file(store.folder("elsewhere").join("projects/unreadable.md"))
        .expect("the unreadable record removed");
    std::fs::remove_file(&link).expect("the home removed");
    let (kind, problem) = store.refused(&["document", "copy", "vault:design", "--to", "site"]);
    assert_eq!(kind, "stale-link", "{problem}");
    assert!(
        problem.contains("vault:public-plan was last copied to site:board-7"),
        "{problem}"
    );
}

#[test]
fn a_task_authored_public_under_a_private_project_keeps_its_own_classification() {
    // A task is as private as its own declaration and repositories say: an explicitly public
    // task under a private project is public — copied on its own it lands on a public source,
    // naming no private project — and one created public there is recorded public.
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
    store.ok(&["task", "copy", "plan:inside", "--to", "site"]);
    let landed = one_file(&store, "site", "tasks");
    assert!(landed.contains("Inside"), "{landed}");
    assert!(!landed.contains("secret-goal"), "{landed}");
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
        !created.contains("classification: private"),
        "a private project does not tighten the task: {created}"
    );
    // Its project is still private, by its own record.
    let (kind, _) = store.refused(&["project", "copy", "plan:secret-goal", "--to", "site"]);
    assert_eq!(kind, "not-private-destination");
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
    // A policy that cannot answer never approves the write: the new task is public by its own
    // record — the task the inactive store wrote under `p` makes `p` private, which tightens no
    // task filed under it — so it is put to the check, and the check is unavailable.
    assert_eq!(kind_of(&refused), "boundary-unavailable");
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
    // The copy did not carry the hidden task, so the edge naming it directly is withheld and
    // recorded nowhere: only a member of the copied set records who depends on it.
    assert!(!hidden.contains("depended_on_by"), "{hidden}");
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
        vec!["project", "status", "set", "site:home", "in-progress"],
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
    for (id, far, why) in [
        ("missing", "shelf:gone-1", "holds no such item"),
        ("unreadable", "shelf:broken", "could not be read"),
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
    // `documented` a private document. The two projects are private by what they hold; the
    // task is not, because a task inherits nothing from its project.
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
    for private in ["shelf:mixed", "shelf:documented"] {
        assert!(!landed.contains(private), "{private}: {landed}");
    }
    // References to public items under a public project are written as before, and so is one
    // to a task whose own record says nothing private: a task inherits nothing from the private
    // project it is filed under.
    assert!(landed.contains("shelf:plain"), "{landed}");
    assert!(landed.contains("shelf:legacy"), "{landed}");
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
    // `shelf` declares nothing: what makes `secret` private is its record, and what makes
    // `broken` unclassifiable is a record that does not parse. `legacy` is filed under a private
    // project, which makes a task private by nothing, so it is named like any public task.
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
    // A public item of the same source is named as before, under a private project or none.
    store.ok(&["task", "update", "site:fine", "--depends-on", "shelf:plain"]);
    assert!(held_text(&store, "site").contains("shelf:plain"));
    store.ok(&[
        "task",
        "update",
        "site:fine",
        "--depends-on",
        "shelf:legacy",
    ]);
    assert!(held_text(&store, "site").contains("shelf:legacy"));
}

#[test]
fn a_narrow_write_to_an_item_private_by_inheritance_held_by_a_public_source_is_refused() {
    let store = Store::new(Some(registered), json!({}));
    // Held there by hand: none of the four is private by its own record. The document and the
    // project are private by what they inherit; the task is not, because a task inherits
    // nothing from its project.
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
    // A public task is written as before, under a public project and under a private one alike.
    store.ok(&["task", "status", "set", "site:open", "done"]);
    store.ok(&["task", "status", "set", "site:filed", "done"]);
}

#[test]
fn a_project_status_set_to_a_private_project_held_by_a_public_source_is_refused() {
    let store = Store::new(Some(registered), json!({}));
    // Held there by hand: one project private by its own record, one only by a task in it.
    store.record(
        "site",
        "projects",
        "closed-plan",
        "title: Closed plan\nstatus: todo\nclassification: private",
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
    let before = store.tree();
    for project in ["site:closed-plan", "site:mixed"] {
        let (kind, said) = store.refused(&["project", "status", "set", project, "done"]);
        assert_eq!(kind, "not-private-destination", "{project}: {said}");
    }
    assert_eq!(store.tree(), before, "nothing was written");
    // A public project with no private member is written as before.
    store.ok(&["project", "status", "set", "site:open-plan", "done"]);
    assert!(
        held_text(&store, "site").contains("title: Open plan\nstatus: done"),
        "{}",
        held_text(&store, "site")
    );
}

#[test]
fn a_delivered_task_under_a_private_project_is_held_to_its_own_record_alone() {
    let store = Store::laid(
        Some(registered),
        |sandbox| json!({ "shelf": folder(sandbox, "shelf", "public") }),
    );
    // Held by hand under a private project in a public source. `unnamed`'s own record says
    // nothing private, so it is public — a task inherits nothing from its project — and is kept
    // in step; `named`'s own record says private, so a public source never takes its write.
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
        "title: Named\nstatus: todo\nproject: closed-plan\nclassification: private\n\
         delivered_by: [\"shelf:helper\"]",
        PLAIN,
    );
    store.record(
        "shelf",
        "tasks",
        "helper",
        "title: Helper\nstatus: todo\ndelivers: [\"site:unnamed\", \"site:named\"]",
        PLAIN,
    );
    let named_before = std::fs::read(store.folder("site").join("tasks/named.md"))
        .expect("the private ticket is on disk");
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
    let outcome = |id: &str| {
        delivered
            .iter()
            .find(|entry| entry["ticket"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("an entry for {id}: {set:#}"))
            .clone()
    };
    assert_ne!(outcome("site:unnamed")["outcome"], "failed", "{set:#}");
    assert_eq!(outcome("site:named")["outcome"], "failed", "{set:#}");
    assert_eq!(
        outcome("site:named")["failure"]["kind"],
        "not-private-destination",
        "{set:#}"
    );
    assert_eq!(
        std::fs::read(store.folder("site").join("tasks/named.md")).expect("still on disk"),
        named_before,
        "the private ticket is as it was"
    );
    let unnamed = store.ok(&["task", "show", "site:unnamed", "--json"]);
    let unnamed: Value = serde_json::from_str(&unnamed).expect("a task");
    assert_eq!(
        unnamed["items"][0]["item"]["delivered_by"],
        json!(["shelf:helper"]),
        "{unnamed:#}"
    );
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
fn an_external_task_naming_a_project_its_source_does_not_hold_is_classified_by_its_own_record() {
    // `stray` names a project `shelf` does not hold. A task inherits nothing from its project,
    // so that project need not be read: `stray`'s own record says nothing private, and a
    // reference to it is written exactly as one to `alone`, which names no project.
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
    store.ok(&["task", "copy", "plan:dangling", "--to", "site"]);
    store.ok(&["task", "copy", "plan:standalone", "--to", "site"]);
    let landed = held_text(&store, "site");
    assert!(landed.contains("shelf:stray"), "{landed}");
    assert!(landed.contains("shelf:alone"), "{landed}");
}

#[test]
fn a_task_filed_under_a_project_nobody_can_classify_lands_publicly_without_naming_it() {
    // What `gone-plan` is cannot be read, but a task is as private as its own record says, so
    // `stray` is public. The project id is withheld from the public copy rather than carried
    // as an opaque value naming somewhere nobody could show was public.
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
    store.ok(&[
        "task",
        "copy",
        "shelf:stray",
        "plan:dangling",
        "--to",
        "site",
    ]);
    let landed = held_text(&store, "site");
    assert!(
        landed.contains("Stray") && landed.contains("Dangling"),
        "{landed}"
    );
    assert!(!landed.contains("gone-plan"), "{landed}");
    // A destination declared private keeps what the source said.
    store.ok(&["task", "copy", "shelf:stray", "--to", "vault"]);
    assert!(held_text(&store, "vault").contains("gone-plan"));
}

#[test]
fn a_narrow_write_or_a_delivery_to_a_task_of_a_project_its_source_does_not_hold_is_held_to_its_own_record()
 {
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
    // `loose` is public by its own record, whatever `gone-plan` is: each write goes ahead.
    store.ok(&["task", "status", "set", "site:loose", "done"]);
    store.ok(&[
        "task",
        "metadata",
        "set",
        "site:loose",
        "acme.note",
        "\"x\"",
    ]);
    let loose = held_text(&store, "site");
    assert!(
        loose.contains("status: done") && loose.contains("acme.note"),
        "{loose}"
    );
    store.ok(&["task", "status", "set", "site:loose", "todo"]);
    let output = store.run(&[
        "task",
        "status",
        "set",
        "plan:worker",
        "in-progress",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let set: Value = serde_json::from_str(&stdout(&output)).expect("a status answer");
    assert_ne!(set["delivered"][0]["outcome"], "failed", "{set:#}");
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

#[test]
fn a_task_s_classification_never_waits_on_its_project_however_that_project_changes() {
    // One copy reads `W-1`, then `W-2` itself, or `W-2` as what `plan:ref` names. Before that
    // the board deletes `HOME-1`, which both are filed under, or has it made private. Neither
    // changes who may read `W-2`: a task inherits nothing from its project, so both land on
    // `site` public as their own records read, and `plan:ref` keeps naming `W-2`.
    type Make = fn(&crate::fixtures::GitHubBoardFields);
    let cases: [(&str, Make); 2] = [
        ("absent", |board| board.replace_after("HOME-1", 2, None)),
        ("made private", |board| {
            board.replace_after(
                "HOME-1",
                2,
                Some(board_item("HOME-1", None, true, Some("private"))),
            );
        }),
    ];
    for (gone, make) in cases {
        for ids in [["board:W-1", "board:W-2"], ["board:W-1", "plan:ref"]] {
            let (store, board) = store_over_board();
            store.record(
                "plan",
                "tasks",
                "ref",
                "title: Ref\nstatus: todo\ndepends_on: [{id: \"board:W-2\", item: task}]",
                PLAIN,
            );
            make(&board);
            let mut arguments = vec!["task", "copy"];
            arguments.extend(ids);
            arguments.extend(["--to", "site"]);
            store.ok(&arguments);
            let landed = held_text(&store, "site");
            assert!(landed.contains("W-1"), "{gone} {ids:?}: {landed}");
            if ids[1] == "plan:ref" {
                assert!(landed.contains("board:W-2"), "{gone} {ids:?}: {landed}");
            } else {
                assert!(landed.contains("W-2"), "{gone} {ids:?}: {landed}");
            }
            assert!(
                !landed.contains("classification: private"),
                "{gone} {ids:?}: {landed}"
            );
        }
    }
}

#[test]
fn a_create_under_a_project_private_by_its_members_takes_a_public_task_and_refuses_the_rest() {
    // `mixed` reads public by its own record and is private by the task it holds; `gone` is not
    // there to read. Neither may take a new document on `site`, which is as private as the
    // project it is filed under; `gone` may not take a task either, because whether that project
    // holds it cannot be read.
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
    for (project, expected, kinds) in [
        ("mixed", "not-private-destination", &["document"][..]),
        ("gone", "project-unclassified", &["task", "document"][..]),
    ] {
        for &kind in kinds {
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
    // A task is as private as its own record says, and a private project does not tighten it:
    // a new public task under `mixed` lands on `site` and reads public.
    store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "mixed",
        "--title",
        "Filed",
        "--body-file",
        &body,
    ]);
    let filed = files_under(&store, "site")
        .into_iter()
        .find(|path| path.contains("tasks") && !path.contains("inner"))
        .map(|path| std::fs::read_to_string(path).expect("a readable record"))
        .expect("the new task was written");
    assert!(
        !filed.contains("classification: private"),
        "a public task under a private project stays public:\n{filed}"
    );
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

/// `base` with `extra`'s keys laid over it.
fn merged(mut base: Value, extra: Value) -> Value {
    merge(&mut base, extra);
    base
}

/// A plan authored in the public `draft` folder whose `openco` task `site` routes to `vault`:
/// a home on `site` with a member on `vault`, and an edge each way across the two. Its
/// `shopco` task is what keeps the home on `site`: a task naming no repository has no say in
/// where the home goes, so with the `openco` task alone voting it would follow it to `vault`.
fn routed_plan() -> Store {
    routed_plan_in("draft")
}

/// [`routed_plan`], authored in the folder `author` — `draft`, or the private `plan`, which
/// keeps the links a copy records — beside a second private folder, `archive`, and a second
/// public one, `mirror`, routing the same work to it.
fn routed_plan_in(author: &str) -> Store {
    routed_plan_laid(author, json!({}))
}

/// [`routed_plan_in`], with `vault`'s entry carrying `vault` beside what it has.
fn routed_plan_laid(author: &str, vault: Value) -> Store {
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "vault": vault,
            "draft": folder(sandbox, "draft", "public"),
            "archive": folder(sandbox, "archive", "private"),
            "site": {"routes": [{"repositories": ["github.com/openco/*"], "to": "vault"}]},
            "mirror": merged(
                folder(sandbox, "mirror", "public"),
                json!({"routes": [{"repositories": ["github.com/openco/*"], "to": "archive"}]}),
            ),
        })
    });
    store.record(
        author,
        "projects",
        "goal",
        "title: Goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        author,
        "tasks",
        "library",
        "title: Library\nstatus: todo\nproject: goal",
        PLAIN,
    );
    store.record(
        author,
        "tasks",
        "consumer",
        &format!(
            "title: Quiet consumer\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]\n\
             depends_on: [library]"
        ),
        PLAIN,
    );
    store.record(
        author,
        "tasks",
        "rollout",
        &format!(
            "title: Rollout\nstatus: todo\nproject: goal\nrepositories: [{SHOP}]\n\
             depends_on: [consumer]"
        ),
        PLAIN,
    );
    store
}

/// Where each draft item of a project copy landed, by its draft id.
fn landed_ids(report: &str) -> BTreeMap<String, String> {
    let report: Value = serde_json::from_str(report).expect("a copy report");
    report["items"]
        .as_array()
        .expect("the copied items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source id").to_owned(),
                item["destination"]
                    .as_str()
                    .expect("a destination id")
                    .to_owned(),
            )
        })
        .collect()
}

/// The `(from, to)` pairs `task deps` answers for `id` in `direction`.
fn edges_of(store: &Store, id: &str, direction: &str) -> Vec<(String, String)> {
    let answer: Value =
        serde_json::from_str(&store.ok(&["task", "deps", id, "--direction", direction, "--json"]))
            .expect("a dependency answer");
    answer["items"]
        .as_array()
        .expect("the edges")
        .iter()
        .map(|edge| {
            (
                edge["from"]["id"].as_str().expect("a near end").to_owned(),
                edge["to"]["id"].as_str().expect("a far end").to_owned(),
            )
        })
        .collect()
}

/// The same routed plan with `vault` declaring a `public_alias`: the public task holds its
/// dependency on the private member itself, as the alias and the member's native id, is marked
/// as holding every edge of its own, and nothing is recorded on the private side — and the edge
/// still reads from the public end, resolved to `vault` by the alias.
#[test]
fn a_public_task_names_an_aliased_private_member_task_opaquely() {
    const ALIAS: &str = "quiet-tracker-2026-alias";
    let store = routed_plan_laid("draft", json!({"public_alias": ALIAS}));
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let (rollout, consumer) = (&landed["draft:rollout"], &landed["draft:consumer"]);
    assert!(consumer.starts_with("vault:"), "{landed:?}");
    let consumer_native = consumer.strip_prefix("vault:").expect("a vault id");
    let task_file = |id: &str| {
        let (folder, native) = id.split_once(':').expect("a qualified id");
        std::fs::read_to_string(
            store
                .folder(folder)
                .join("tasks")
                .join(format!("{native}.md")),
        )
        .expect("a task file")
    };

    let public = task_file(rollout);
    assert!(
        public.contains(&format!("alias/{ALIAS}")) && public.contains(consumer_native),
        "the dependency is on the public task, opaquely:\n{public}"
    );
    assert!(
        public.contains("onetaskgraph.edges_on_record: 1"),
        "{public}"
    );
    for named in ["vault", "Quiet consumer", "draft:consumer"] {
        assert!(!public.contains(named), "names {named}:\n{public}");
    }
    assert!(
        !task_file(consumer).contains("depended_on_by"),
        "{}",
        task_file(consumer)
    );
    assert_eq!(
        edges_of(&store, rollout, "depends-on"),
        vec![(rollout.clone(), consumer.clone())]
    );
}

#[test]
fn a_public_task_s_dependency_on_a_private_member_task_is_kept_on_the_private_side() {
    let store = routed_plan();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let (home, library, consumer, rollout) = (
        &landed["draft:goal"],
        &landed["draft:library"],
        &landed["draft:consumer"],
        &landed["draft:rollout"],
    );
    assert!(
        home.starts_with("site:") && rollout.starts_with("site:"),
        "{landed:?}"
    );
    assert!(
        consumer.starts_with("vault:"),
        "the openco task is routed: {landed:?}"
    );

    // `vault` has neither an identifier of its own nor a `public_alias`, so the edge is
    // withheld from the public task and recorded on the private one, and the public task is not
    // marked as holding every edge of its own.
    let task_file = |id: &str| {
        let (folder, native) = id.split_once(':').expect("a qualified id");
        std::fs::read_to_string(
            store
                .folder(folder)
                .join("tasks")
                .join(format!("{native}.md")),
        )
        .expect("a task file")
    };
    assert!(
        !task_file(rollout).contains("edges_on_record"),
        "{}",
        task_file(rollout)
    );
    assert!(
        task_file(consumer).contains("depended_on_by"),
        "{}",
        task_file(consumer)
    );

    // The edge reads from either end, and the private task's own edge to a public one is
    // where it always was.
    let kept = vec![(rollout.clone(), consumer.clone())];
    assert_eq!(edges_of(&store, rollout, "depends-on"), kept);
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), kept);
    assert_eq!(
        edges_of(&store, consumer, "depends-on"),
        vec![(consumer.clone(), library.clone())]
    );
    // And so does the home's graph, the private task drawn outside it.
    let graph: Value =
        serde_json::from_str(&store.ok(&["project", "graph", home, "--format", "json"]))
            .expect("a graph");
    assert!(
        graph["edges"]
            .as_array()
            .expect("the graph's edges")
            .contains(&json!({"from": consumer, "to": rollout})),
        "{graph:#}"
    );

    // Nothing on `site` names the private task: not its id, its title, where it is, its
    // origin, or the project it is filed under. The home's own member list is the one record
    // there that names `vault`, and it names the member project alone.
    let consumer_native = consumer.strip_prefix("vault:").expect("a vault id");
    let vault_root = store.folder("vault").display().to_string();
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8(bytes).expect("a text record");
        for named in [
            consumer.as_str(),
            "Quiet consumer",
            "draft:consumer",
            &vault_root,
        ] {
            assert!(!text.contains(named), "{path} names {named}:\n{text}");
        }
        if path.contains("tasks") {
            assert!(!text.contains("vault"), "{path}:\n{text}");
            assert!(!text.contains(consumer_native), "{path}:\n{text}");
        }
    }

    // A second copy of the same plan writes nothing to `site` and keeps exactly one edge.
    let site = tree(&store.folder("site"));
    let vault = tree(&store.folder("vault"));
    let again = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert_eq!(again, landed, "the second copy finds every counterpart");
    assert_eq!(
        tree(&store.folder("site")),
        site,
        "site is as the first copy left it"
    );
    assert_eq!(tree(&store.folder("vault")), vault, "and so is vault");
    assert_eq!(edges_of(&store, rollout, "depends-on"), kept);
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), kept);

    // The dependents the private task records are the store's, kept in step after the copy,
    // so they are outside the digest its copy record holds: a copy by query skips it.
    let report: Value = serde_json::from_str(&store.ok(&[
        "task", "copy", "--to", "site", "--source", "draft", "--json",
    ]))
    .expect("a copy report");
    let consumer_outcome = report["items"]
        .as_array()
        .expect("the copied items")
        .iter()
        .find(|item| item["source"] == "draft:consumer")
        .expect("the consumer's outcome");
    assert_eq!(consumer_outcome["skipped"], true, "{consumer_outcome:#}");
    assert_eq!(tree(&store.folder("vault")), vault, "vault is as it was");

    // An entry naming a dependent this copy does not land is the private task's own, and
    // outlives every copy; the entries for what the copy lands are the copy's to decide. A
    // member copy records one for a dependent that then leaves the plan.
    store.record(
        "draft",
        "tasks",
        "announce",
        "title: Announce\nstatus: todo\nproject: goal\ndepends_on: [consumer]",
        PLAIN,
    );
    let added = landed_ids(&store.ok(&[
        "project",
        "copy",
        "draft:goal",
        "--member",
        "draft:announce",
        "--member",
        "draft:consumer",
        "--to",
        "site",
        "--json",
    ]));
    let announce = &added["draft:announce"];
    std::fs::remove_file(store.folder("draft").join("tasks/announce.md"))
        .expect("the dependent leaves the plan");
    let mut dependents = edges_of(&store, consumer, "depended-on-by");
    dependents.sort();
    let mut wanted = vec![
        (announce.clone(), consumer.clone()),
        (rollout.clone(), consumer.clone()),
    ];
    wanted.sort();
    assert_eq!(dependents, wanted);

    // Dropping the edge at the draft takes it off the private task on the next copy.
    let draft = store.folder("draft").join("tasks/rollout.md");
    let authored = std::fs::read_to_string(&draft).expect("the draft rollout");
    std::fs::write(
        &draft,
        authored.replace("depends_on: [consumer]", "depends_on: []"),
    )
    .expect("the edge dropped");
    store.ok(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(edges_of(&store, rollout, "depends-on"), Vec::new());
    assert_eq!(
        edges_of(&store, consumer, "depended-on-by"),
        vec![(announce.clone(), consumer.clone())],
        "only the entry the copy decides is taken off"
    );
}

#[test]
fn a_dependency_read_a_private_source_cannot_answer_fails_rather_than_dropping_the_edge() {
    let store = routed_plan();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let (consumer, rollout) = (&landed["draft:consumer"], &landed["draft:rollout"]);

    // A record `vault` cannot read leaves the read of a public task's edges failed, naming
    // the source, rather than answered without what `vault` holds.
    store.record("vault", "tasks", "unreadable", "title: [", PLAIN);
    for arguments in [
        vec!["task", "deps", rollout.as_str()],
        vec!["project", "graph", landed["draft:goal"].as_str()],
    ] {
        let output = store.run(&arguments);
        assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
        assert!(stderr(&output).contains("vault"), "{}", stderr(&output));
    }
    std::fs::remove_file(store.folder("vault").join("tasks/unreadable.md"))
        .expect("the unreadable record removed");

    // A record the forward read's match finds, holding the public task's id as another kind,
    // fails that read rather than being drawn as a task edge.
    store.record(
        "vault",
        "tasks",
        "misspelled",
        &format!(
            "title: Misspelled\nstatus: todo\nmetadata:\n  onetaskgraph.depended_on_by:\n    \
             '{rollout}':\n      id: {rollout}\n      kind: project"
        ),
        PLAIN,
    );
    for arguments in [
        vec!["task", "deps", rollout.as_str()],
        vec!["project", "graph", landed["draft:goal"].as_str()],
    ] {
        let output = store.run(&arguments);
        assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
        assert!(
            stderr(&output).contains("onetaskgraph.depended_on_by"),
            "{}",
            stderr(&output)
        );
    }
    std::fs::remove_file(store.folder("vault").join("tasks/misspelled.md"))
        .expect("the misspelled record removed");

    // And a recorded entry that does not say what it names is refused, never read as none.
    let path = store.folder("vault").join("tasks").join(format!(
        "{}.md",
        consumer.strip_prefix("vault:").expect("a vault id")
    ));
    let held = std::fs::read_to_string(&path).expect("the consumer");
    std::fs::write(
        &path,
        held.replace(&format!("id: {rollout}"), "id: site:someone-else"),
    )
    .expect("the entry spoiled");
    let output = store.run(&["task", "deps", consumer, "--direction", "depended-on-by"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("onetaskgraph.depended_on_by"),
        "{}",
        stderr(&output)
    );

    // So is a record that is not an object of dependents, an entry that is no endpoint at all,
    // and one naming its dependent by an unqualified id: each fails the read of the private
    // task's dependents, and the copy that would land on it, rather than reading as none.
    let entry = format!("    '{rollout}':\n      id: {rollout}\n      kind: task\n");
    assert!(held.contains(&entry), "{held}");
    let site = tree(&store.folder("site"));
    for (spoiled, why) in [
        (
            held.replace(
                &format!("  onetaskgraph.depended_on_by:\n{entry}"),
                &format!("  onetaskgraph.depended_on_by: ['{rollout}']\n"),
            ),
            "not an object",
        ),
        (
            held.replace(&entry, &format!("    '{rollout}': rollout\n")),
            "no endpoint",
        ),
        (
            held.replace(
                &entry,
                "    rollout:\n      id: rollout\n      kind: task\n",
            ),
            "unqualified",
        ),
    ] {
        assert_ne!(spoiled, held, "{why}: the entry spoiled");
        std::fs::write(&path, &spoiled).expect("the entry spoiled");
        for arguments in [
            vec![
                "task",
                "deps",
                consumer.as_str(),
                "--direction",
                "depended-on-by",
            ],
            vec!["project", "copy", "draft:goal", "--to", "site"],
        ] {
            let output = store.run(&arguments);
            assert_ne!(
                output.status.code(),
                Some(0),
                "{why}: {arguments:?}\n{}",
                stdout(&output)
            );
            assert!(
                stderr(&output).contains("onetaskgraph.depended_on_by"),
                "{why}: {arguments:?}\n{}",
                stderr(&output)
            );
        }
        assert_eq!(
            tree(&store.folder("site")),
            site,
            "{why}: the refused copy wrote nothing to site"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the consumer"),
            spoiled,
            "{why}: the refused copy left the private task as it found it"
        );
    }
}

/// `store`'s configuration with the source `name` added, declared private.
fn add_private_source(store: &Store, name: &str, source: Value) {
    let path = store.sandbox.project().join("onetaskgraph.yaml");
    let mut document: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the project document"))
            .expect("JSON");
    document["sources"][name] = merged(source, json!({"visibility": "private"}));
    std::fs::write(&path, document.to_string()).expect("the project document");
}

/// `store`'s configuration with `ledger` added: a Linear source declared private whose
/// credential nothing sets, so every read of it fails as a host without that key fails.
fn add_unauthenticated_private_source(store: &Store) {
    add_private_source(
        store,
        "ledger",
        json!({
            "plugin": "linear",
            "config": {"api_key_env": "ONETASKGRAPH_E2E_ABSENT_KEY", "team": "LED"},
        }),
    );
}

#[test]
fn a_dependency_read_answers_without_a_private_source_it_cannot_authenticate_to() {
    let store = routed_plan();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let (home, consumer, rollout) = (
        landed["draft:goal"].as_str(),
        landed["draft:consumer"].as_str(),
        landed["draft:rollout"].as_str(),
    );
    let complete_mermaid = store.ok(&["project", "graph", home]);
    let complete_json = store.ok(&["project", "graph", home, "--format", "json"]);
    assert!(
        !complete_json.contains("unanswered"),
        "a complete graph's document is as it was: {complete_json}"
    );
    let forward = vec![(rollout.to_owned(), consumer.to_owned())];
    assert_eq!(edges_of(&store, rollout, "depends-on"), forward);

    add_unauthenticated_private_source(&store);

    // `task deps` of the public task still answers what `vault` records and succeeds: `ledger`
    // is named in the answer's `unanswered` and as a notice on standard error, never in its
    // `errors`.
    let output = store.run(&["task", "deps", rollout, "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a dependency answer");
    assert_eq!(answer["items"][0]["to"]["id"], consumer, "{answer:#}");
    assert_eq!(answer["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(answer["errors"], json!([]), "{answer:#}");
    let unanswered = answer["unanswered"]
        .as_array()
        .expect("the sources not asked");
    assert_eq!(unanswered.len(), 1, "{answer:#}");
    assert_eq!(unanswered[0]["source"], "ledger");
    assert_eq!(unanswered[0]["error"]["kind"], "auth");
    assert!(
        stderr(&output).contains("note: private source ledger was not asked"),
        "{}",
        stderr(&output)
    );
    let output = store.run(&["task", "deps", rollout]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains(consumer), "{}", stdout(&output));
    assert!(stderr(&output).contains("ledger"), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("--complete"),
        "{}",
        stderr(&output)
    );
    // The other end asks no private source but its own, and answers as it did.
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), forward);
    assert_eq!(edges_of(&store, rollout, "depended-on-by"), Vec::new());

    // `--complete` counts `ledger` as a source the read lost: it is in `errors`, and the
    // answer is partial.
    let output = store.run(&["task", "deps", rollout, "--complete", "--json"]);
    assert_eq!(output.status.code(), Some(4), "{}", stdout(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a dependency answer");
    assert_eq!(answer["errors"][0]["source"], "ledger", "{answer:#}");
    assert!(answer.get("unanswered").is_none(), "{answer:#}");
    assert!(stderr(&output).contains("source ledger could not answer"));
    let output = store.run(&["task", "deps", rollout, "--complete", "--allow-partial"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));

    // `project graph` draws every edge it drew before, names `ledger` on standard error and in
    // the document's `unanswered`, and exits 0.
    let output = store.run(&["project", "graph", home]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), complete_mermaid);
    assert!(stderr(&output).contains("note: private source ledger was not asked"));
    let output = store.run(&["project", "graph", home, "--format", "json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let mut graph: Value = serde_json::from_str(&stdout(&output)).expect("a graph");
    let unanswered = graph
        .as_object_mut()
        .expect("a document")
        .remove("unanswered")
        .expect("the sources that could not answer");
    assert_eq!(unanswered[0]["source"], "ledger", "{unanswered:#}");
    assert_eq!(unanswered[0]["error"]["kind"], "auth", "{unanswered:#}");
    assert_eq!(unanswered.as_array().map(Vec::len), Some(1));
    let complete: Value = serde_json::from_str(&complete_json).expect("a graph");
    assert_eq!(
        graph, complete,
        "every other part of the document is as it was"
    );

    // `--complete` refuses the graph, printing none of it.
    let output = store.run(&["project", "graph", home, "--complete"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(stderr(&output).contains("ledger"), "{}", stderr(&output));

    // A private board nothing answers at, and one whose rate limiter refuses every request,
    // could not be asked either: each is named beside `ledger`, by its own kind, and every
    // other edge still stands.
    add_private_source(
        &store,
        "dark",
        json!({"plugin": "github-projects",
               "config": crate::fixtures::github_projects_unreachable(&store.sandbox)}),
    );
    add_private_source(
        &store,
        "busy",
        json!({"plugin": "github-projects",
               "config": crate::fixtures::github_projects_rate_limited(&store.sandbox, None)}),
    );
    let output = store.run(&["task", "deps", rollout, "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a dependency answer");
    assert_eq!(answer["items"][0]["to"]["id"], consumer, "{answer:#}");
    assert_eq!(answer["errors"], json!([]), "{answer:#}");
    let mut kinds: Vec<(String, String)> = answer["unanswered"]
        .as_array()
        .expect("the sources not asked")
        .iter()
        .map(|failure| {
            (
                failure["source"].as_str().expect("a source").to_owned(),
                failure["error"]["kind"]
                    .as_str()
                    .expect("a kind")
                    .to_owned(),
            )
        })
        .collect();
    kinds.sort();
    let unasked = [
        ("busy", "rate-limited"),
        ("dark", "unavailable"),
        ("ledger", "auth"),
    ]
    .map(|(source, kind)| (source.to_owned(), kind.to_owned()));
    assert_eq!(kinds, unasked, "{answer:#}");
    let output = store.run(&["project", "graph", home, "--format", "json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let mut graph: Value = serde_json::from_str(&stdout(&output)).expect("a graph");
    let mut kinds: Vec<(String, String)> = graph
        .as_object_mut()
        .expect("a document")
        .remove("unanswered")
        .expect("the sources that could not answer")
        .as_array()
        .expect("a list")
        .iter()
        .map(|failure| {
            (
                failure["source"].as_str().expect("a source").to_owned(),
                failure["error"]["kind"]
                    .as_str()
                    .expect("a kind")
                    .to_owned(),
            )
        })
        .collect();
    kinds.sort();
    assert_eq!(kinds, unasked);
    assert_eq!(
        graph, complete,
        "every other part of the document is as it was"
    );

    // A read of an item that lives in `ledger` still fails, naming it.
    for arguments in [
        vec!["task", "deps", "ledger:LED-1"],
        vec![
            "task",
            "deps",
            "ledger:LED-1",
            "--direction",
            "depended-on-by",
        ],
        vec!["project", "graph", "ledger:plan"],
    ] {
        let output = store.run(&arguments);
        assert_ne!(
            output.status.code(),
            Some(0),
            "{arguments:?}\n{}",
            stdout(&output)
        );
        assert!(
            stderr(&output).contains("ledger"),
            "{arguments:?}\n{}",
            stderr(&output)
        );
    }

    // And a record a private source answers with that does not say what it names still fails
    // the read beside the source that could not be asked.
    store.record("vault", "tasks", "unreadable", "title: [", PLAIN);
    for arguments in [
        vec!["task", "deps", rollout],
        vec!["project", "graph", home],
    ] {
        let output = store.run(&arguments);
        assert_ne!(
            output.status.code(),
            Some(0),
            "{arguments:?}\n{}",
            stdout(&output)
        );
        assert!(
            stderr(&output).contains("vault"),
            "{arguments:?}\n{}",
            stderr(&output)
        );
    }
}

/// The same, through the engine a program links: the graph a plan is loaded from says it is
/// incomplete and which source it lacks, and a request for a complete one is refused.
#[tokio::test]
async fn a_linked_engine_loads_a_plan_without_a_private_source_it_cannot_authenticate_to() {
    let store = routed_plan();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let home: onetaskgraph_core::GlobalId = landed["draft:goal"].parse().expect("a home id");
    let rollout: onetaskgraph_core::GlobalId = landed["draft:rollout"].parse().expect("a task id");
    let consumer = landed["draft:consumer"].as_str();
    let request = |completeness| onetaskgraph_core::ProjectGraphRequest {
        project: home.clone(),
        direction: None,
        group_by: None,
        completeness,
    };
    let whole = linked_engine(&store)
        .project_graph(&request(onetaskgraph_core::Completeness::Required))
        .await
        .expect("every source answers");
    assert!(whole.is_complete());

    add_unauthenticated_private_source(&store);
    let engine = linked_engine(&store);
    let graph = engine
        .project_graph(&request(onetaskgraph_core::Completeness::NamingUnasked))
        .await
        .expect("a graph without ledger");
    assert!(!graph.is_complete());
    let unanswered: Vec<&str> = graph
        .unanswered()
        .iter()
        .filter_map(|entry| entry.source())
        .map(|failure| failure.source.as_str())
        .collect();
    assert_eq!(unanswered, ["ledger"]);
    assert_eq!(
        graph.mermaid(),
        whole.mermaid(),
        "every other edge is drawn"
    );
    assert!(graph.mermaid().contains("-->"), "{}", graph.mermaid());

    let refused = engine
        .project_graph(&request(onetaskgraph_core::Completeness::Required))
        .await
        .expect_err("a complete graph is refused");
    assert!(
        matches!(&refused, onetaskgraph_core::EngineError::SourceFailed { name, .. } if name == "ledger"),
        "{refused:?}"
    );

    let answer = engine
        .task_dependencies(&onetaskgraph_core::DependencyRequest {
            id: rollout.clone(),
            direction: onetaskgraph_plugin_api::Direction::DependsOn,
            completeness: onetaskgraph_core::Completeness::NamingUnasked,
            paging: onetaskgraph_core::Paging {
                limit: std::num::NonZeroU32::new(50).expect("non-zero"),
                token: None,
            },
        })
        .await
        .expect("an answer");
    assert_eq!(answer.items.len(), 1);
    assert_eq!(answer.items[0].to.id.to_string(), consumer);
    assert!(answer.errors.is_empty(), "{:?}", answer.errors);
    let unasked: Vec<&str> = answer
        .unanswered
        .iter()
        .filter_map(|entry| entry.source())
        .map(|failure| failure.source.as_str())
        .collect();
    assert_eq!(unasked, ["ledger"]);

    // Asked for every source's part, the same read reports `ledger` as a failure instead.
    let whole = engine
        .task_dependencies(&onetaskgraph_core::DependencyRequest {
            id: rollout.clone(),
            direction: onetaskgraph_plugin_api::Direction::DependsOn,
            completeness: onetaskgraph_core::Completeness::Required,
            paging: onetaskgraph_core::Paging {
                limit: std::num::NonZeroU32::new(50).expect("non-zero"),
                token: None,
            },
        })
        .await
        .expect("an answer");
    assert_eq!(whole.items, answer.items);
    assert!(whole.unanswered.is_empty());
    let failed: Vec<&str> = whole
        .errors
        .iter()
        .map(|failure| failure.source.as_str())
        .collect();
    assert_eq!(failed, ["ledger"]);
}

/// Every `(from, to)` pair `task deps` answers for `id` in `direction`, one edge to a page,
/// walked to exhaustion.
fn paged_edges(store: &Store, id: &str, direction: &str) -> Vec<(String, String)> {
    let mut paged = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..20 {
        let mut arguments = vec![
            "task",
            "deps",
            id,
            "--direction",
            direction,
            "--limit",
            "1",
            "--json",
        ];
        if let Some(token) = &token {
            arguments.extend(["--page", token.as_str()]);
        }
        let page: Value = serde_json::from_str(&store.ok(&arguments)).expect("a page");
        let items = page["items"].as_array().expect("the edges");
        assert!(items.len() <= 1, "{page:#}");
        paged.extend(items.iter().map(|edge| {
            (
                edge["from"]["id"].as_str().expect("a near end").to_owned(),
                edge["to"]["id"].as_str().expect("a far end").to_owned(),
            )
        }));
        match page["next"].as_str() {
            Some(next) => token = Some(next.to_owned()),
            None => return paged,
        }
    }
    panic!("{id}'s edges did not end within twenty pages: {paged:?}");
}

#[test]
fn a_public_task_s_edges_page_with_what_every_private_source_records_of_it() {
    // `vault` applies the match itself; `memory` declares it unsupported, so the engine
    // narrows its tasks, one to a page.
    let recorded = |id: &str| json!({id: {"id": id, "kind": "task"}});
    let store = Store::laid(Some(registered), |_| {
        json!({
            "memory": {
                "plugin": "in-memory",
                "visibility": "private",
                "config": {
                    "capabilities": {"filter_by_metadata": "unsupported", "max_page_size": 1},
                    "tasks": [
                        {"id": "M-0", "title": "Unrelated", "status": {"category": "todo", "name": "Todo"}, "labels": []},
                        {"id": "M-1", "title": "Recorder", "status": {"category": "todo", "name": "Todo"}, "labels": [],
                         "metadata": {"onetaskgraph.depended_on_by": recorded("site:rollout")}},
                        {"id": "M-2", "title": "Decoy", "status": {"category": "todo", "name": "Todo"}, "labels": [],
                         "metadata": {"onetaskgraph.depended_on_by": recorded("site:other")}},
                    ],
                },
            },
        })
    });
    store.record(
        "site",
        "tasks",
        "library",
        "title: Library\nstatus: todo",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "rollout",
        // Landed by a copy that kept its edges on the private side, which is the one task a
        // dependency read asks a private source about.
        "title: Rollout\nstatus: todo\ndepends_on: [library]\nmetadata:\n  onetaskgraph.copied_destination_version: sha256:00",
        PLAIN,
    );
    store.record(
        "vault",
        "tasks",
        "hidden",
        "title: Hidden\nstatus: todo\nmetadata:\n  onetaskgraph.depended_on_by:\n    \
         'site:rollout':\n      id: site:rollout\n      kind: task\n    \
         'site:other':\n      id: site:other\n      kind: task",
        PLAIN,
    );

    // A task beside it depends on it natively, so its reverse read has both kinds to merge.
    store.record(
        "vault",
        "tasks",
        "sibling",
        "title: Sibling\nstatus: todo\ndepends_on: [hidden]",
        PLAIN,
    );

    // The source's own edge and one edge from each private source, merged into one answer.
    let mut whole = edges_of(&store, "site:rollout", "depends-on");
    whole.sort();
    let rollout = || "site:rollout".to_owned();
    assert_eq!(
        whole,
        vec![
            (rollout(), "memory:M-1".to_owned()),
            (rollout(), "site:library".to_owned()),
            (rollout(), "vault:hidden".to_owned()),
        ]
    );

    // A page of one at a time walks to exhaustion with every edge once.
    let mut paged = paged_edges(&store, "site:rollout", "depends-on");
    paged.sort();
    assert_eq!(paged, whole, "paged and whole agree");

    // Each private task reads what it records from its own end, paged too.
    assert_eq!(
        edges_of(&store, "memory:M-1", "depended-on-by"),
        vec![(rollout(), "memory:M-1".to_owned())]
    );
    // What it records merges with what its own source holds, a page at a time.
    let mut reverse = paged_edges(&store, "vault:hidden", "depended-on-by");
    reverse.sort();
    let hidden = || "vault:hidden".to_owned();
    assert_eq!(
        reverse,
        vec![
            ("site:other".to_owned(), hidden()),
            (rollout(), hidden()),
            ("vault:sibling".to_owned(), hidden()),
        ]
    );
    let mut whole_reverse = edges_of(&store, "vault:hidden", "depended-on-by");
    whole_reverse.sort();
    assert_eq!(whole_reverse, reverse, "paged and whole agree");

    // A private task its source does not hold records nothing, and its reverse read answers
    // exactly what a public source's does for a task it does not hold.
    assert_eq!(
        edges_of(&store, "vault:nothing", "depended-on-by"),
        Vec::new()
    );
    assert_eq!(
        edges_of(&store, "site:nothing", "depended-on-by"),
        Vec::new()
    );
}

// A folder's permissions are Unix's to refuse a write with.
#[cfg(unix)]
#[test]
fn a_copy_whose_record_on_the_private_task_cannot_be_written_leaves_both_folders_as_it_found_them()
{
    use std::os::unix::fs::PermissionsExt as _;

    let store = routed_plan();
    // The first copy without the edge, so the second has only the record to add on `vault`
    // beside a change to `site`.
    let draft = store.folder("draft").join("tasks/rollout.md");
    let authored = std::fs::read_to_string(&draft).expect("the draft rollout");
    std::fs::write(
        &draft,
        authored.replace("depends_on: [consumer]", "depends_on: []"),
    )
    .expect("the edge left out");
    store.ok(&["project", "copy", "draft:goal", "--to", "site"]);
    std::fs::write(
        &draft,
        authored.replace("title: Rollout", "title: Rollout, revised"),
    )
    .expect("the edge and a new title");

    let site = tree(&store.folder("site"));
    let vault = tree(&store.folder("vault"));
    let tasks = store.folder("vault").join("tasks");
    let sealed: Vec<(std::path::PathBuf, u32, u32)> = std::fs::read_dir(&tasks)
        .expect("vault's tasks")
        .map(|entry| (entry.expect("a task file").path(), 0o444, 0o644))
        .chain([(tasks.clone(), 0o555, 0o755)])
        .collect();
    for (path, mode, _) in &sealed {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(*mode)).expect("sealed");
    }
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site"]);
    for (path, _, mode) in &sealed {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(*mode)).expect("unsealed");
    }

    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stderr(&output).contains("vault"), "{}", stderr(&output));
    assert_eq!(
        tree(&store.folder("site")),
        site,
        "the revised title is taken back"
    );
    assert_eq!(
        tree(&store.folder("vault")),
        vault,
        "and vault holds no record"
    );
}

#[test]
fn a_member_copy_records_its_dependency_on_a_private_task_an_earlier_copy_landed() {
    // Authored in `plan`, which keeps the link to where its consumer landed, so a member copy
    // resolves that counterpart without carrying it.
    let store = routed_plan_in("plan");
    let landed = landed_ids(&store.ok(&["project", "copy", "plan:goal", "--to", "site", "--json"]));
    let (consumer, rollout) = (&landed["plan:consumer"], &landed["plan:rollout"]);
    store.record(
        "plan",
        "tasks",
        "announce",
        "title: Announce\nstatus: todo\nproject: goal\ndepends_on: [consumer]",
        PLAIN,
    );

    // Only the new task is copied: the private task it depends on is the counterpart the
    // first copy recorded, and records the new dependent beside the one it already held.
    let added = landed_ids(&store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--member",
        "plan:announce",
        "--to",
        "site",
        "--json",
    ]));
    let announce = &added["plan:announce"];
    assert!(announce.starts_with("site:"), "{added:?}");
    assert_eq!(
        edges_of(&store, announce, "depends-on"),
        vec![(announce.clone(), consumer.clone())]
    );
    let mut dependents = edges_of(&store, consumer, "depended-on-by");
    dependents.sort();
    let mut wanted = vec![
        (announce.clone(), consumer.clone()),
        (rollout.clone(), consumer.clone()),
    ];
    wanted.sort();
    assert_eq!(dependents, wanted);
    let announce_file = std::fs::read_to_string(store.folder("site").join("tasks").join(format!(
        "{}.md",
        announce.strip_prefix("site:").expect("a site id")
    )))
    .expect("the announce copy");
    assert!(!announce_file.contains("vault"), "{announce_file}");

    // A copy of the private task elsewhere carries none of what it records: those entries
    // name the dependents of the task they are on, not of its copy.
    store.ok(&["task", "copy", consumer, "--to", "archive"]);
    let copied = held_text(&store, "archive");
    assert!(copied.contains("Quiet consumer"), "{copied}");
    assert!(!copied.contains("depended_on_by"), "{copied}");
}

#[test]
fn a_member_copy_of_a_dependent_alone_takes_its_dropped_edge_off_the_private_task() {
    let store = routed_plan_in("plan");
    let landed = landed_ids(&store.ok(&["project", "copy", "plan:goal", "--to", "site", "--json"]));
    let (consumer, rollout) = (&landed["plan:consumer"], &landed["plan:rollout"]);
    assert_eq!(
        edges_of(&store, consumer, "depended-on-by"),
        vec![(rollout.clone(), consumer.clone())]
    );

    // The edge is dropped at the draft, and only its dependent is copied again: the private
    // task it named is not in the copy, and still has the entry taken off.
    let draft = store.folder("plan").join("tasks/rollout.md");
    let authored = std::fs::read_to_string(&draft).expect("the draft rollout");
    std::fs::write(
        &draft,
        authored.replace("depends_on: [consumer]", "depends_on: []"),
    )
    .expect("the edge dropped");
    let vault = store.folder("vault");
    let consumer_file = vault.join("tasks").join(format!(
        "{}.md",
        consumer.strip_prefix("vault:").expect("a vault id")
    ));
    let before = std::fs::read_to_string(&consumer_file).expect("the consumer");
    assert!(before.contains("depended_on_by"), "{before}");
    let again = landed_ids(&store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--member",
        "plan:rollout",
        "--to",
        "site",
        "--json",
    ]));
    assert!(!again.contains_key("plan:consumer"), "{again:?}");
    assert_eq!(&again["plan:rollout"], rollout);
    assert_eq!(edges_of(&store, rollout, "depends-on"), Vec::new());
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), Vec::new());
    let after = std::fs::read_to_string(&consumer_file).expect("the consumer");
    assert!(!after.contains("depended_on_by"), "{after}");
    assert!(after.contains("Quiet consumer"), "{after}");
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8(bytes).expect("a text record");
        if path.contains("tasks") {
            assert!(!text.contains("vault"), "{path}:\n{text}");
        }
    }

    // A third copy of the dependent alone finds nothing left to take off, and writes nothing
    // to the private side.
    let held = tree(&vault);
    store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--member",
        "plan:rollout",
        "--to",
        "site",
    ]);
    assert_eq!(tree(&vault), held, "vault is as the second copy left it");
}

#[test]
fn a_copy_onto_a_private_task_whose_record_cannot_be_read_is_refused_and_taken_back() {
    let store = routed_plan();
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let consumer = &landed["draft:consumer"];
    let path = store.folder("vault").join("tasks").join(format!(
        "{}.md",
        consumer.strip_prefix("vault:").expect("a vault id")
    ));
    let held = std::fs::read_to_string(&path).expect("the consumer");
    assert!(held.contains("      kind: task"), "{held}");
    std::fs::write(
        &path,
        held.replace("      kind: task", "      kind: project"),
    )
    .expect("the record spoiled");
    let draft = store.folder("draft").join("tasks/rollout.md");
    let authored = std::fs::read_to_string(&draft).expect("the draft rollout");
    std::fs::write(
        &draft,
        authored.replace("title: Rollout", "title: Rollout, revised"),
    )
    .expect("a new title");

    let site = tree(&store.folder("site"));
    let vault = tree(&store.folder("vault"));
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("onetaskgraph.depended_on_by"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        tree(&store.folder("site")),
        site,
        "the revised title is taken back"
    );
    assert_eq!(
        tree(&store.folder("vault")),
        vault,
        "and vault is as it was"
    );
}

#[test]
fn a_project_graph_draws_every_page_of_what_a_private_source_records_of_its_tasks() {
    // More recorders than one page of a project-sized read, in a source that ignores the
    // match, one task to a source page, so the engine narrows every page itself.
    let recorders: Vec<Value> = (0..55)
        .map(|index| {
            json!({
                "id": format!("M-{index:02}"),
                "title": format!("Recorder {index:02}"),
                "status": {"category": "todo", "name": "Todo"},
                "labels": [],
                "metadata": {"onetaskgraph.depended_on_by": {
                    "site:rollout": {"id": "site:rollout", "kind": "task"},
                }},
            })
        })
        .chain([json!({
            "id": "M-decoy",
            "title": "Decoy",
            "status": {"category": "todo", "name": "Todo"},
            "labels": [],
            "metadata": {"onetaskgraph.depended_on_by": {
                "site:other": {"id": "site:other", "kind": "task"},
            }},
        })])
        .collect();
    let store = Store::laid(Some(registered), |_| {
        json!({
            "memory": {
                "plugin": "in-memory",
                "visibility": "private",
                "config": {
                    "capabilities": {"filter_by_metadata": "unsupported", "max_page_size": 1},
                    "tasks": recorders,
                },
            },
        })
    });
    store.public_project_p();
    store.record(
        "site",
        "tasks",
        "library",
        "title: Library\nstatus: todo\nproject: p",
        PLAIN,
    );
    store.record(
        "site",
        "tasks",
        "rollout",
        // Landed by a copy that kept its edges on the private side, which is the one task a
        // graph asks a private source about.
        "title: Rollout\nstatus: todo\nproject: p\ndepends_on: [library]\nmetadata:\n  onetaskgraph.copied_destination_version: sha256:00",
        PLAIN,
    );

    let graph: Value =
        serde_json::from_str(&store.ok(&["project", "graph", "site:p", "--format", "json"]))
            .expect("a graph");
    let mut froms: Vec<String> = graph["edges"]
        .as_array()
        .expect("the graph's edges")
        .iter()
        .filter(|edge| edge["to"] == "site:rollout")
        .map(|edge| edge["from"].as_str().expect("a prerequisite").to_owned())
        .collect();
    froms.sort();
    let mut wanted: Vec<String> = (0..55)
        .map(|index| format!("memory:M-{index:02}"))
        .collect();
    wanted.push("site:library".to_owned());
    wanted.sort();
    assert_eq!(froms, wanted, "{graph:#}");
}

#[test]
fn a_dry_run_of_a_routed_plan_records_nothing_on_either_side() {
    let store = routed_plan();
    let site = tree(&store.folder("site"));
    let vault = tree(&store.folder("vault"));
    store.ok(&["project", "copy", "draft:goal", "--to", "site", "--dry-run"]);
    assert_eq!(tree(&store.folder("site")), site, "site is untouched");
    assert_eq!(tree(&store.folder("vault")), vault, "and so is vault");

    // Nor over a plan already copied without the edge, which the next real copy would add.
    let draft = store.folder("draft").join("tasks/rollout.md");
    let authored = std::fs::read_to_string(&draft).expect("the draft rollout");
    std::fs::write(
        &draft,
        authored.replace("depends_on: [consumer]", "depends_on: []"),
    )
    .expect("the edge left out");
    let landed =
        landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    std::fs::write(&draft, &authored).expect("the edge put back");
    let site = tree(&store.folder("site"));
    let vault = tree(&store.folder("vault"));
    store.ok(&["project", "copy", "draft:goal", "--to", "site", "--dry-run"]);
    assert_eq!(tree(&store.folder("site")), site, "site is untouched");
    assert_eq!(tree(&store.folder("vault")), vault, "and so is vault");
    assert_eq!(
        edges_of(&store, &landed["draft:consumer"], "depended-on-by"),
        Vec::new()
    );
    store.ok(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(
        edges_of(&store, &landed["draft:consumer"], "depended-on-by"),
        vec![(
            landed["draft:rollout"].clone(),
            landed["draft:consumer"].clone()
        )]
    );
}

#[test]
fn a_member_copy_whose_private_prerequisite_is_gone_is_refused_naming_it() {
    let store = routed_plan_in("plan");
    let landed = landed_ids(&store.ok(&["project", "copy", "plan:goal", "--to", "site", "--json"]));
    let consumer = &landed["plan:consumer"];
    store.record(
        "plan",
        "tasks",
        "announce",
        "title: Announce\nstatus: todo\nproject: goal\ndepends_on: [consumer]",
        PLAIN,
    );
    std::fs::remove_file(store.folder("vault").join("tasks").join(format!(
        "{}.md",
        consumer.strip_prefix("vault:").expect("a vault id")
    )))
    .expect("the private task removed");
    let site = tree(&store.folder("site"));
    let output = store.run(&[
        "project",
        "copy",
        "plan:goal",
        "--member",
        "plan:announce",
        "--to",
        "site",
    ]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains(&format!("no task with the id {consumer}")),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        tree(&store.folder("site")),
        site,
        "nothing of the copy stays on site"
    );
}

#[test]
fn a_routed_plan_copied_on_from_its_public_home_keeps_its_dependency_on_the_private_member() {
    let store = routed_plan();
    let first = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    assert_eq!(
        members_of_home(&store, &first["draft:goal"]).len(),
        1,
        "the home on site names its member on vault"
    );

    // The home copied on carries its member's tasks: the home to `mirror`, the member's
    // `openco` task routed by `mirror` to the private `archive`.
    let copy = [
        "project",
        "copy",
        first["draft:goal"].as_str(),
        "--to",
        "mirror",
        "--json",
    ];
    let landed = landed_ids(&store.ok(&copy));
    let (home, library, consumer, rollout) = (
        &landed[&first["draft:goal"]],
        &landed[&first["draft:library"]],
        &landed[&first["draft:consumer"]],
        &landed[&first["draft:rollout"]],
    );
    assert!(
        home.starts_with("mirror:") && rollout.starts_with("mirror:"),
        "{landed:?}"
    );
    assert!(consumer.starts_with("archive:"), "{landed:?}");

    // The edge the first copy kept on `vault` arrives between the copies, from either end, and
    // the private task's own edge names the public task's copy.
    let kept = vec![(rollout.clone(), consumer.clone())];
    assert_eq!(edges_of(&store, rollout, "depends-on"), kept);
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), kept);
    assert_eq!(
        edges_of(&store, consumer, "depends-on"),
        vec![(consumer.clone(), library.clone())]
    );
    let graph: Value =
        serde_json::from_str(&store.ok(&["project", "graph", home, "--format", "json"]))
            .expect("a graph");
    assert!(
        graph["edges"]
            .as_array()
            .expect("the graph's edges")
            .contains(&json!({"from": consumer, "to": rollout})),
        "{graph:#}"
    );

    // Nothing on `mirror` names either private task, its source, its title or where it is.
    // The home's own member list is the one record there naming `archive`, and it names the
    // member project alone, as it did before this edge was kept (left as it is, by ruling).
    let members = members_of_home(&store, home);
    assert_eq!(members.len(), 1, "{members:?}");
    assert!(members[0].starts_with("archive:"), "{members:?}");
    let members_entry = format!("  onetaskgraph.members:\n  - {}\n", members[0]);
    let private = [
        consumer.as_str(),
        first["draft:consumer"].as_str(),
        "Quiet consumer",
        "draft:consumer",
        "archive",
        "vault",
    ];
    let roots = [
        store.folder("archive").display().to_string(),
        store.folder("vault").display().to_string(),
    ];
    for (path, bytes) in tree(&store.folder("mirror")) {
        let text = String::from_utf8(bytes)
            .expect("a text record")
            .replace(&members_entry, "");
        for named in private
            .iter()
            .copied()
            .chain(roots.iter().map(String::as_str))
        {
            assert!(!text.contains(named), "{path} names {named}:\n{text}");
        }
    }

    // Copying the same home again writes nothing anywhere and keeps exactly one edge.
    let before = store.tree();
    assert_eq!(
        landed_ids(&store.ok(&copy)),
        landed,
        "every counterpart is found"
    );
    assert_eq!(store.tree(), before, "nothing is written a second time");
    assert_eq!(edges_of(&store, rollout, "depends-on"), kept);
    assert_eq!(edges_of(&store, consumer, "depended-on-by"), kept);
}

#[test]
fn a_member_whose_home_withheld_its_origin_is_taken_over_by_a_copy_to_its_source() {
    // Authored in the private `plan`, so the public home on `site` records no origin naming it
    // and the link `plan` keeps is what ties them.
    let store = routed_plan_in("plan");
    let landed = landed_ids(&store.ok(&["project", "copy", "plan:goal", "--to", "site", "--json"]));
    let home = &landed["plan:goal"];
    assert!(home.starts_with("site:"), "{landed:?}");
    let shown = |id: &str| -> Value {
        let answer: Value =
            serde_json::from_str(&store.ok(&["project", "show", id, "--json"])).expect("a project");
        answer["items"][0]["item"]["metadata"].clone()
    };
    assert!(
        shown(home).get("onetaskgraph.origin").is_none(),
        "{:#}",
        shown(home)
    );
    let members = members_of_home(&store, home);
    assert_eq!(members.len(), 1, "{members:?}");
    let member = &members[0];

    let report: Value = serde_json::from_str(&store.ok(&[
        "project",
        "copy",
        "plan:goal",
        "--to",
        "vault",
        "--json",
    ]))
    .expect("a copy report");
    let goal = report["items"]
        .as_array()
        .expect("the copied items")
        .iter()
        .find(|item| item["source"] == "plan:goal")
        .expect("the project's outcome");
    assert_eq!(goal["destination"], json!(member), "{report:#}");
    assert_eq!(goal["via"], "scan", "{report:#}");
    let projects: Value =
        serde_json::from_str(&store.ok(&["project", "list", "--source", "vault", "--json"]))
            .expect("a listing");
    assert_eq!(
        projects["items"].as_array().expect("projects").len(),
        1,
        "{projects:#}"
    );
    let held = shown(member);
    assert!(held.get("onetaskgraph.member_of").is_none(), "{held:#}");
    assert_eq!(held["onetaskgraph.origin"], "plan:goal", "{held:#}");
    assert_eq!(
        shown("plan:goal")["onetaskgraph.copies"]["vault"],
        json!(member)
    );
    assert!(
        shown(home).get("onetaskgraph.members").is_none(),
        "the former home names it no longer: {:#}",
        shown(home)
    );
}

/// The member projects `home` records, read back through the binary.
fn members_of_home(store: &Store, home: &str) -> Vec<String> {
    let shown: Value = serde_json::from_str(&store.ok(&["project", "show", home, "--json"]))
        .expect("a project answer");
    shown["items"][0]["item"]["metadata"]["onetaskgraph.members"]
        .as_array()
        .expect("the home names its members")
        .iter()
        .map(|member| member.as_str().expect("a qualified id").to_owned())
        .collect()
}

#[test]
fn copying_a_public_home_on_is_refused_when_its_private_member_cannot_say_what_it_records() {
    let store = routed_plan();
    let first = landed_ids(&store.ok(&["project", "copy", "draft:goal", "--to", "site", "--json"]));
    let rollout = first["draft:rollout"].clone();
    let copy = [
        "project",
        "copy",
        first["draft:goal"].as_str(),
        "--to",
        "mirror",
    ];

    // A record `vault` cannot read, and one the match finds holding the public task's id as
    // another kind: each refuses the copy before it writes, rather than copying it without
    // the edge `vault` keeps.
    let misspelled = format!(
        "title: Misspelled\nstatus: todo\nmetadata:\n  onetaskgraph.depended_on_by:\n    \
         '{rollout}':\n      id: {rollout}\n      kind: project"
    );
    for (record, front, said) in [
        ("unreadable", "title: [", "vault"),
        (
            "misspelled",
            misspelled.as_str(),
            "onetaskgraph.depended_on_by",
        ),
    ] {
        store.record("vault", "tasks", record, front, PLAIN);
        let before = store.tree();
        let output = store.run(&copy);
        assert_ne!(
            output.status.code(),
            Some(0),
            "{record}: {}",
            stdout(&output)
        );
        assert!(
            stderr(&output).contains(said),
            "{record}: {}",
            stderr(&output)
        );
        assert_eq!(store.tree(), before, "{record}: nothing was written");
        std::fs::remove_file(store.folder("vault").join(format!("tasks/{record}.md")))
            .expect("the record removed");
    }

    // With the record put right the same copy lands, carrying the edge.
    let landed = landed_ids(&store.ok(&[
        "project",
        "copy",
        first["draft:goal"].as_str(),
        "--to",
        "mirror",
        "--json",
    ]));
    assert_eq!(
        edges_of(&store, &landed[&rollout], "depends-on"),
        vec![(
            landed[&rollout].clone(),
            landed[&first["draft:consumer"]].clone()
        )]
    );
}
