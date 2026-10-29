//! A private plan home on a GitHub board: a board declared private holds every canonical task of
//! a plan, those naming only a public repository included, each in a private repository.
//!
//! On a source declared `visibility: private`, an item naming exactly one repository is created
//! there only when that repository reads private; otherwise it is created where an item naming
//! none is — its project issue's repository, else the configured one — and still names its
//! public repository. Each destination's visibility is read once per copy, before the copy's
//! first write. Every journey spawns the binary against the loopback board, which serves the
//! Project's `public` field over GraphQL and each repository's visibility over REST, and asserts
//! on what the binary said, on what it reads back and on every request the board was sent.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, RepositoryAnswer, document, github_projects_with_board};

/// The repository the private board creates an issue in when nothing else decides.
const CONFIGURED: &str = "nickderobertis/onetaskgraph";
/// A repository the stand-in serves public, and the policy answers public.
const OPEN: &str = "nickderobertis/openwidget";
/// A repository the stand-in serves private.
const VAULT: &str = "nickderobertis/vault";
/// A second repository the stand-in serves private.
const LOCKER: &str = "nickderobertis/locker";
/// The term the caller's check refuses wherever it is found.
const PRIVATE_TERM: &str = "Quietharbor";

/// Answer a repository public exactly when it is [`OPEN`].
const VISIBILITY: &str = "import json,sys; r=json.load(sys.stdin)['repository']; \
     print(json.dumps({'visibility': 'public' if r == 'github.com/nickderobertis/openwidget' \
     else 'private'}))";
/// Refuse any write naming the private term, as onevcs refuses — exit 1, naming where — and
/// pass every other.
const CHECK: &str = "import json,sys; t=sys.stdin.read(); r='Quietharbor' in t; \
     print(json.dumps({'verdict': 'refuse', 'surface': 'text'} if r else {'verdict': 'pass'})); \
     sys.exit(1 if r else 0)";

/// One sandbox: the private folder `plan`, the loopback board `board` declared private, routing
/// [`OPEN`]'s work to the public board `site` — a loopback of its own, filing in [`OPEN`].
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    site: GitHubBoardFields,
    root: PathBuf,
}

/// What one run sent: to the private board, and to the public one.
struct Sent {
    board: Vec<(String, Value)>,
    site: Vec<(String, Value)>,
}

impl Setup {
    /// The mixed store: `site` configured, so public work is projected there.
    fn mixed() -> Self {
        Self::laid(true, |_| {})
    }

    /// The store with `projecting` deciding whether `site` and the route to it exist, and `lay`
    /// changing the configuration document before it is written.
    fn laid(projecting: bool, lay: impl FnOnce(&mut Value)) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plan");
        let (config, board) = github_projects_with_board(&sandbox);
        board.set_repository_visibility(OPEN, RepositoryAnswer::Public);
        let (mut public, site) = github_projects_with_board(&sandbox);
        public["repository"] = json!(OPEN);
        site.use_repository(OPEN);
        site.set_repository_visibility(OPEN, RepositoryAnswer::Public);
        let mut sources = json!({
            "plan": {"plugin": "local-md", "config": {"root": root}, "visibility": "private"},
            "board": {"plugin": "github-projects", "config": config, "visibility": "private"},
        });
        if projecting {
            sources["site"] =
                json!({"plugin": "github-projects", "config": public, "visibility": "public"});
            sources["board"]["routes"] =
                json!([{"repositories": [format!("github.com/{OPEN}")], "to": "site"}]);
        }
        let mut configured: Value =
            serde_json::from_str(&document(&sources)).expect("a configuration document");
        configured["write_policy"] = json!({
            "visibility_command": ["python3", "-c", VISIBILITY],
            "check_command": ["python3", "-c", CHECK],
        });
        lay(&mut configured);
        sandbox.project_document(&configured.to_string());
        Self {
            sandbox,
            board,
            site,
            root,
        }
    }

    fn record(&self, kind: &str, id: &str, front: &str, body: &str) {
        let path = self.root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\n{body}\n")).expect("a record");
    }

    /// The private plan `plan:home`: `secret`, naming a private repository, and `open`, naming
    /// only a public one and depending on `secret`.
    fn plan(&self) {
        self.record("projects", "home", "title: Home\nstatus: todo", "The plan.");
        self.record(
            "tasks",
            "secret",
            &format!("title: Secret step\nstatus: todo\nproject: home\nrepositories: [github.com/{VAULT}]"),
            "Private work.",
        );
        self.record(
            "tasks",
            "open",
            &format!(
                "title: Open step\nstatus: todo\nproject: home\n\
                 repositories: [github.com/{OPEN}]\ndepends_on: [secret]"
            ),
            "Public work.",
        );
    }

    /// Run the binary, answering what it printed and every request each board was sent.
    fn run(&self, arguments: &[&str]) -> (Output, Sent) {
        let (board, site) = (self.board.served().len(), self.site.served().len());
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        let sent = Sent {
            board: self.board.served()[board..].to_vec(),
            site: self.site.served()[site..].to_vec(),
        };
        (output, sent)
    }

    fn ok(&self, arguments: &[&str]) -> (String, Sent) {
        let (output, sent) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (stdout(&output), sent)
    }

    /// The failure kind and message of a run that had to be refused, having sent no mutation.
    fn refused(&self, arguments: &[&str]) -> (String, String, Sent) {
        let mut with_json = arguments.to_vec();
        with_json.push("--json");
        let (output, sent) = self.run(&with_json);
        assert_ne!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` was expected to be refused\n{}",
            arguments.join(" "),
            stdout(&output)
        );
        assert!(
            !sent
                .board
                .iter()
                .chain(&sent.site)
                .any(|(document, _)| is_mutation(document)),
            "a refused write sent a mutation: {:#?}",
            sent.board
        );
        let failure: Value = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|_| panic!("a failure document: {}", stderr(&output)));
        (
            failure["failure"]["kind"]
                .as_str()
                .expect("a kind")
                .to_owned(),
            failure["failure"]["message"]
                .as_str()
                .expect("a message")
                .to_owned(),
            sent,
        )
    }

    /// One item read back through the binary.
    fn shown(&self, kind: &str, id: &str) -> Value {
        let (shown, _) = self.ok(&[kind, "show", id, "--json"]);
        let shown: Value = serde_json::from_str(&shown).expect("a show");
        shown["items"][0]["item"].clone()
    }

    /// Where the copy of the plan's `kind` `id` landed on `to`, by the link it recorded.
    fn landed(&self, kind: &str, id: &str, to: &str) -> String {
        let item = self.shown(kind, &format!("plan:{id}"));
        item["metadata"]["onetaskgraph.copies"][to]
            .as_str()
            .unwrap_or_else(|| panic!("plan:{id} records where it landed on {to}: {item:#}"))
            .to_owned()
    }

    /// The repository the private board's issue `id` — qualified — lives in.
    fn issue_repository(&self, id: &str) -> String {
        let native = id.split_once(':').map_or(id, |(_, native)| native);
        self.board
            .repository_of(native)
            .unwrap_or_else(|| panic!("{id} is an issue in a repository"))
    }
}

fn is_mutation(document: &str) -> bool {
    document.trim_start().starts_with("mutation")
}

/// The node id of the repository each `createIssue` of `sent` targets, in the order sent.
fn created_in(sent: &[(String, Value)]) -> Vec<String> {
    let mut repositories = Vec::new();
    for (document, variables) in sent {
        if document == onetaskgraph_github_projects::graphql::CREATE_ISSUES {
            for slot in 0..onetaskgraph_github_projects::MUTATION_BATCH {
                if variables[format!("send{slot}")] == true {
                    repositories.push(
                        variables[format!("create{slot}")]["repositoryId"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
            }
        } else if document.contains("createIssue(input:$input)") {
            repositories.push(
                variables["input"]["repositoryId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
    }
    repositories
}

/// Each visibility read sent, as `project` for the board's or the repository's `owner/name`.
fn visibility_reads(sent: &[(String, Value)]) -> Vec<String> {
    sent.iter()
        .filter_map(|(document, _)| {
            if document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
                Some("project".to_owned())
            } else {
                document.strip_prefix("GET /repos/").map(str::to_owned)
            }
        })
        .collect()
}

/// Assert each visibility read of `sent` names a different destination, every one of them comes
/// before the first mutation, and the destinations read are exactly `expected`.
fn read_once_before_writing(sent: &[(String, Value)], expected: &[&str]) {
    let reads = visibility_reads(sent);
    let mut distinct = reads.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        reads.len(),
        "a destination was read more than once: {reads:?}"
    );
    let mut wanted: Vec<String> = expected.iter().map(|read| (*read).to_owned()).collect();
    wanted.sort_unstable();
    assert_eq!(distinct, wanted, "{sent:#?}");
    let first_mutation = sent
        .iter()
        .position(|(document, _)| is_mutation(document))
        .expect("the copy wrote");
    assert_eq!(
        visibility_reads(&sent[..first_mutation]).len(),
        reads.len(),
        "every visibility read is answered before the first mutation: {sent:#?}"
    );
}

#[test]
fn a_mixed_plan_lands_on_a_private_board_with_its_public_task_in_a_private_repository() {
    let setup = Setup::mixed();
    setup.plan();
    let (_, sent) = setup.ok(&["project", "copy", "plan:home", "--to", "board", "--json"]);
    // Every issue created on the private board is in a repository the stand-in serves private.
    let private_ids = [
        setup.board.repository_node_id(CONFIGURED),
        setup.board.repository_node_id(VAULT),
    ];
    let on_board = created_in(&sent.board);
    assert_eq!(
        on_board.len(),
        3,
        "the project and its two tasks: {:#?}",
        sent.board
    );
    assert!(
        on_board.iter().all(|id| private_ids.contains(id)),
        "{on_board:?} vs {private_ids:?}"
    );
    // The public task's canonical item names exactly its public repository, while its issue is
    // the configured repository's — it is filed under the project, whose issue is there.
    let open = setup.landed("task", "open", "board");
    let canonical = setup.shown("task", &open);
    assert_eq!(
        canonical["repositories"],
        json!([format!("github.com/{OPEN}")]),
        "{canonical:#}"
    );
    assert_eq!(setup.issue_repository(&open), CONFIGURED);
    assert_eq!(
        setup.issue_repository(&setup.landed("task", "secret", "board")),
        VAULT
    );
    // The only issues created in the public repository are on the public board: its `Public
    // work` project and the public task's view.
    let open_id = setup.site.repository_node_id(OPEN);
    assert_eq!(
        open_id, "REPO-1",
        "the public board files in the public repository"
    );
    assert_eq!(
        created_in(&sent.site),
        [open_id.clone(), open_id],
        "{:#?}",
        sent.site
    );
    for (document, variables) in &sent.site {
        for named in [
            VAULT,
            "vault",
            "Secret",
            "plan:",
            "home",
            "board:",
            "onetaskgraph.origin",
        ] {
            assert!(
                !variables.to_string().contains(named),
                "a request to the public board names {named:?}: {document}\n{variables}"
            );
        }
    }
    // The private project maps the canonical task to that view.
    let project = setup.shown("project", &setup.landed("project", "home", "board"));
    let map = &project["metadata"]["onetaskgraph.public_projections"];
    let entries = map["tasks"].as_array().expect("the map's tasks");
    assert_eq!(entries.len(), 1, "{map:#}");
    assert_eq!(entries[0]["task"], json!(open), "{map:#}");
    let view = entries[0]["projection"].as_str().expect("a view");
    assert!(view.starts_with("site:"), "{map:#}");
    let shown_view = setup.shown("task", view);
    assert_eq!(shown_view["title"], "Open step", "{shown_view:#}");
    assert_eq!(
        shown_view["repositories"],
        json!([format!("github.com/{OPEN}")])
    );
}

#[test]
fn a_copy_reads_each_destination_once_before_its_first_write_and_again_at_the_next_copy() {
    let setup = Setup::mixed();
    setup.plan();
    let copy = ["project", "copy", "plan:home", "--to", "board", "--json"];
    let (_, first) = setup.ok(&copy);
    // The board, the repository each task names — the public one only deciding where its task's
    // issue goes — and the configured repository the project's and the public task's issues
    // are created in.
    read_once_before_writing(&first.board, &["project", VAULT, OPEN, CONFIGURED]);
    // The public board is not declared private, so nothing reads who can read it.
    assert!(
        visibility_reads(&first.site).is_empty(),
        "{:#?}",
        first.site
    );
    // The repository the public task's issue was predicted into is the one its create targets.
    let open_create = first
        .board
        .iter()
        .find_map(|(document, variables)| {
            let created = if document == onetaskgraph_github_projects::graphql::CREATE_ISSUES {
                (0..onetaskgraph_github_projects::MUTATION_BATCH)
                    .map(|slot| &variables[format!("create{slot}")])
                    .find(|create| create["title"] == "Open step")
            } else {
                Some(&variables["input"]).filter(|input| input["title"] == "Open step")
            };
            created.map(|create| create["repositoryId"].clone())
        })
        .expect("the public task's canonical create");
    assert_eq!(
        open_create,
        json!(setup.board.repository_node_id(CONFIGURED))
    );

    // Every item changed: a second copy updates each where it is, reading the board and each
    // existing issue's own repository once — never the public repository that only placed a
    // create.
    for (kind, id) in [("projects", "home"), ("tasks", "secret"), ("tasks", "open")] {
        let path = setup.root.join(kind).join(format!("{id}.md"));
        let text = std::fs::read_to_string(&path).expect("a record");
        std::fs::write(
            &path,
            text.replace("work.", "work, revised.")
                .replace("The plan.", "The plan, revised."),
        )
        .expect("an edit");
    }
    let (_, second) = setup.ok(&copy);
    assert!(created_in(&second.board).is_empty(), "{:#?}", second.board);
    read_once_before_writing(&second.board, &["project", VAULT, CONFIGURED]);
}

#[test]
fn a_task_filed_under_no_project_is_held_to_the_configured_repository_its_create_targets() {
    let setup = Setup::mixed();
    setup.record(
        "tasks",
        "lone",
        &format!("title: Lone step\nstatus: todo\nrepositories: [github.com/{OPEN}]\nclassification: private"),
        "Kept private by its own record.",
    );
    let (_, sent) = setup.ok(&["task", "copy", "plan:lone", "--to", "board"]);
    read_once_before_writing(&sent.board, &["project", OPEN, CONFIGURED]);
    assert_eq!(
        created_in(&sent.board),
        [setup.board.repository_node_id(CONFIGURED)]
    );
    // Read in that order: the named repository, which decides nothing, then where it goes.
    assert_eq!(
        visibility_reads(&sent.board)
            .into_iter()
            .filter(|read| read != "project")
            .collect::<Vec<_>>(),
        [OPEN, CONFIGURED]
    );
    let lone = setup.shown("task", &setup.landed("task", "lone", "board"));
    assert_eq!(lone["repositories"], json!([format!("github.com/{OPEN}")]));
}

#[test]
fn a_private_only_plan_files_each_task_in_its_own_repository_reading_each_destination_once() {
    let setup = Setup::laid(false, |_| {});
    setup
        .board
        .set_repository_visibility(LOCKER, RepositoryAnswer::Private);
    setup.record("projects", "home", "title: Home\nstatus: todo", "The plan.");
    for (id, repository) in [("a", VAULT), ("b", LOCKER), ("c", VAULT), ("d", LOCKER)] {
        setup.record(
            "tasks",
            id,
            &format!("title: Step {id}\nstatus: todo\nproject: home\nrepositories: [github.com/{repository}]"),
            "Private work.",
        );
    }
    setup.record(
        "documents",
        "design",
        "title: Design\nproject: home",
        "How it fits.",
    );
    let (_, sent) = setup.ok(&["project", "copy", "plan:home", "--to", "board", "--json"]);
    read_once_before_writing(&sent.board, &["project", VAULT, LOCKER, CONFIGURED]);
    assert!(sent.site.is_empty(), "no public source is configured");
    for (id, repository) in [("a", VAULT), ("b", LOCKER), ("c", VAULT), ("d", LOCKER)] {
        assert_eq!(
            setup.issue_repository(&setup.landed("task", id, "board")),
            repository
        );
    }
    let project = setup.shown("project", &setup.landed("project", "home", "board"));
    assert!(
        project["metadata"]
            .get("onetaskgraph.public_projections")
            .is_none(),
        "{project:#}"
    );
    // The design document is its own copy, reading the board and its project's repository once.
    let (_, sent) = setup.ok(&["document", "copy", "plan:design", "--to", "board", "--json"]);
    read_once_before_writing(&sent.board, &["project", CONFIGURED]);
}

#[test]
fn a_repository_made_public_during_a_copy_is_seen_by_the_next_copy() {
    let setup = Setup::mixed();
    setup.plan();
    // Private while the copy reads it, public once its first write has landed.
    setup
        .board
        .set_repository_visibility_after(VAULT, RepositoryAnswer::Public, "mutation", 1);
    let copy = ["project", "copy", "plan:home", "--to", "board"];
    let (_, sent) = setup.ok(&copy);
    assert_eq!(created_in(&sent.board).len(), 3, "{:#?}", sent.board);
    assert_eq!(
        visibility_reads(&sent.board)
            .iter()
            .filter(|read| *read == VAULT)
            .count(),
        1
    );
    // The next copy that writes the task in that repository reads it again, and is refused
    // before it sends anything.
    let path = setup.root.join("tasks/secret.md");
    let text = std::fs::read_to_string(&path).expect("a record");
    std::fs::write(
        &path,
        text.replace("Private work.", "Private work, revised."),
    )
    .expect("an edit");
    let (kind, message, sent) = setup.refused(&copy);
    assert_eq!(kind, "destination-not-private", "{message}");
    assert!(
        visibility_reads(&sent.board).contains(&VAULT.to_owned()),
        "{:#?}",
        sent.board
    );
}

#[test]
fn an_unchanged_recopy_creates_nothing_and_a_changed_one_updates_the_same_issues() {
    let setup = Setup::mixed();
    setup.plan();
    let copy = ["project", "copy", "plan:home", "--to", "board", "--json"];
    setup.ok(&copy);
    let ids = || {
        ["home", "secret", "open"].map(|id| {
            let kind = if id == "home" { "project" } else { "task" };
            setup.landed(kind, id, "board")
        })
    };
    let landed = ids();
    let views = setup.site.served().len();
    let (_, sent) = setup.ok(&copy);
    for sent in [&sent.board, &sent.site] {
        assert!(created_in(sent).is_empty(), "{sent:#?}");
        assert!(
            !sent
                .iter()
                .any(|(document, _)| document.contains("createProjectV2")
                    || document.contains("addProjectV2ItemById")),
            "{sent:#?}"
        );
    }
    assert_eq!(ids(), landed);
    assert!(setup.site.served().len() >= views);
    // One task's content changes: the re-copy updates that same issue, and creates none.
    let path = setup.root.join("tasks/open.md");
    let text = std::fs::read_to_string(&path).expect("a record");
    std::fs::write(&path, text.replace("Public work.", "Public work, revised.")).expect("an edit");
    let (_, sent) = setup.ok(&copy);
    assert!(created_in(&sent.board).is_empty(), "{:#?}", sent.board);
    assert!(created_in(&sent.site).is_empty(), "{:#?}", sent.site);
    assert_eq!(ids(), landed);
    let canonical = setup.shown("task", &landed[2]);
    assert!(
        canonical["content"]
            .as_str()
            .is_some_and(|content| content.contains("Public work, revised.")),
        "{canonical:#}"
    );
    assert_eq!(setup.issue_repository(&landed[2]), CONFIGURED);
}

/// How many issues a board holds titled `title`, by listing its tasks through the binary.
fn titled(setup: &Setup, source: &str, title: &str) -> usize {
    let (listed, _) = setup.ok(&["task", "list", "--source", source, "--json"]);
    let listed: Value = serde_json::from_str(&listed).expect("a listing");
    listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["item"]["title"] == title)
        .count()
}

/// How many projects `source` holds titled `title`.
fn projects_titled(setup: &Setup, source: &str, title: &str) -> usize {
    let (listed, _) = setup.ok(&["project", "list", "--source", source, "--json"]);
    let listed: Value = serde_json::from_str(&listed).expect("a listing");
    listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["item"]["title"] == title)
        .count()
}

#[test]
fn a_copy_interrupted_after_a_canonical_issue_or_its_view_exists_runs_again_into_one_of_each() {
    // Failed once after the canonical issues exist, and once after the view does.
    for (board, operation) in [(true, "mutation"), (false, "query($id0:ID!,$id1:ID!")] {
        let setup = Setup::mixed();
        setup.plan();
        let failing = if board { &setup.board } else { &setup.site };
        // The private board's third mutation follows its canonical creates. The public
        // board's placement read follows both creates: Public work and the view already
        // exist. Its id0/id1 signature is unique to the created-board-items read here.
        failing.refuse_after(operation, if board { 2 } else { 0 });
        let copy = ["project", "copy", "plan:home", "--to", "board"];
        let (output, sent) = setup.run(&copy);
        assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
        if !board {
            assert_eq!(
                created_in(&sent.site).len(),
                2,
                "the project and view creates precede the refused placement read"
            );
            let last_create = sent
                .site
                .iter()
                .rposition(|(document, _)| document.contains("createIssue("))
                .expect("a view create");
            let refused_read = sent
                .site
                .iter()
                .rposition(|(document, _)| document.starts_with("query($id0:ID!,$id1:ID!"))
                .expect("the refused placement read");
            assert!(last_create < refused_read, "{:#?}", sent.site);
        }
        setup.ok(&copy);
        assert_eq!(titled(&setup, "board", "Open step"), 1, "board: {board}");
        assert_eq!(titled(&setup, "board", "Secret step"), 1, "board: {board}");
        assert_eq!(titled(&setup, "site", "Open step"), 1, "board: {board}");
        assert_eq!(
            projects_titled(&setup, "site", "Public work"),
            1,
            "board: {board}"
        );
    }
}

#[test]
fn a_private_board_whose_configured_repository_reads_public_refuses_the_public_task() {
    let setup = Setup::mixed();
    setup.plan();
    setup
        .board
        .set_repository_visibility(CONFIGURED, RepositoryAnswer::Public);
    let (kind, message, _) = setup.refused(&["project", "copy", "plan:home", "--to", "board"]);
    assert_eq!(kind, "destination-not-private", "{message}");
}

#[test]
fn a_private_board_whose_project_reads_public_refuses_the_copy() {
    let setup = Setup::mixed();
    setup.plan();
    setup.board.set_project_public(true);
    let (kind, message, _) = setup.refused(&["project", "copy", "plan:home", "--to", "board"]);
    assert_eq!(kind, "destination-not-private", "{message}");
}

#[test]
fn an_existing_canonical_issue_in_a_public_repository_is_refused_where_it_is() {
    let setup = Setup::mixed();
    setup.plan();
    setup.ok(&["project", "copy", "plan:home", "--to", "board"]);
    let open = setup.landed("task", "open", "board");
    let native = open.split_once(':').expect("qualified").1.to_owned();
    // Somebody moved its issue into the public repository.
    setup.board.transfer(&native, OPEN);
    let path = setup.root.join("tasks/open.md");
    let text = std::fs::read_to_string(&path).expect("a record");
    std::fs::write(&path, text.replace("Public work.", "Public work, revised.")).expect("an edit");
    let (kind, message, sent) = setup.refused(&["project", "copy", "plan:home", "--to", "board"]);
    assert_eq!(kind, "destination-not-private", "{message}");
    assert!(
        !sent
            .board
            .iter()
            .any(|(document, _)| document.contains("transferIssue")),
        "{:#?}",
        sent.board
    );
    assert_eq!(setup.board.repository_of(&native).as_deref(), Some(OPEN));
}

#[test]
fn a_named_repository_whose_visibility_is_unreadable_refuses_and_takes_no_fallback() {
    let setup = Setup::mixed();
    setup
        .board
        .set_repository_visibility(VAULT, RepositoryAnswer::Unseen);
    setup.record(
        "tasks",
        "lone",
        &format!("title: Lone\nstatus: todo\nrepositories: [github.com/{VAULT}]"),
        "Private work.",
    );
    let (kind, message, sent) = setup.refused(&["task", "copy", "plan:lone", "--to", "board"]);
    assert_eq!(kind, "visibility-unreadable", "{message}");
    assert!(message.contains(VAULT), "names the repository: {message}");
    assert_eq!(visibility_reads(&sent.board), [VAULT], "{:#?}", sent.board);
}

#[test]
fn a_private_task_copied_to_a_source_not_declared_private_is_refused() {
    let setup = Setup::mixed();
    setup.plan();
    let (kind, message, _) = setup.refused(&["task", "copy", "plan:secret", "--to", "site"]);
    assert_eq!(kind, "not-private-destination", "{message}");
}

#[test]
fn a_view_naming_a_private_term_is_refused_before_anything_is_written() {
    let setup = Setup::mixed();
    setup.plan();
    let path = setup.root.join("tasks/open.md");
    let text = std::fs::read_to_string(&path).expect("a record");
    std::fs::write(
        &path,
        text.replace("Public work.", &format!("Public work for {PRIVATE_TERM}.")),
    )
    .expect("an edit");
    let (kind, message, _) = setup.refused(&["project", "copy", "plan:home", "--to", "board"]);
    assert_eq!(kind, "boundary-refused", "{message}");
    assert!(titled(&setup, "board", "Open step") == 0);
}

#[test]
fn placement_is_unchanged_on_a_board_not_declared_private_and_for_a_private_repository() {
    // Not declared private: a task naming one public repository is filed there, and nothing is
    // read about who can read it.
    let setup = Setup::laid(false, |configured| {
        configured["sources"]["board"]
            .as_object_mut()
            .expect("the board")
            .remove("visibility");
        configured["sources"]["plan"]
            .as_object_mut()
            .expect("the plan")
            .remove("visibility");
    });
    setup.record(
        "tasks",
        "open",
        &format!("title: Open\nstatus: todo\nrepositories: [github.com/{OPEN}]"),
        "Public work.",
    );
    let (_, sent) = setup.ok(&["task", "copy", "plan:open", "--to", "board"]);
    assert_eq!(
        created_in(&sent.board),
        [setup.board.repository_node_id(OPEN)]
    );
    assert!(
        visibility_reads(&sent.board).is_empty(),
        "{:#?}",
        sent.board
    );
    // Declared private: a task naming one private repository is filed there.
    let setup = Setup::laid(false, |_| {});
    setup.record(
        "tasks",
        "secret",
        &format!("title: Secret\nstatus: todo\nrepositories: [github.com/{VAULT}]"),
        "Private work.",
    );
    let (_, sent) = setup.ok(&["task", "copy", "plan:secret", "--to", "board"]);
    assert_eq!(
        created_in(&sent.board),
        [setup.board.repository_node_id(VAULT)]
    );
}

#[test]
fn the_library_copy_reads_each_destination_again_at_each_copy_over_the_same_sources() {
    let setup = Setup::laid(false, |_| {});
    setup.record(
        "tasks",
        "secret",
        &format!("title: Secret\nstatus: todo\nrepositories: [github.com/{VAULT}]"),
        "Private work.",
    );
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        setup.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &setup.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    // One engine, its sources built once, as a process linking the store holds them.
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let request = onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(vec!["plan:secret".parse().expect("an id")])
            .expect("one item"),
        scope: onetaskgraph_core::CopyScope::Tasks,
        destination: onetaskgraph_plugin_api::SourceName::new("board").expect("a name"),
        target_task: None,
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: false,
        no_link: false,
        expect: None,
        status_policy: onetaskgraph_core::StatusPolicy::Source,
        statuses: Vec::new(),
    };
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let mut reads = Vec::new();
    for revision in ["Private work, once.", "Private work, twice."] {
        let path = setup.root.join("tasks/secret.md");
        let text = std::fs::read_to_string(&path).expect("a record");
        let body = text.rsplit_once("---\n").expect("front matter").1;
        std::fs::write(&path, text.replace(body, &format!("{revision}\n"))).expect("an edit");
        let before = setup.board.served().len();
        runtime
            .block_on(engine.copy(&request))
            .expect("the copy lands");
        let sent = setup.board.served()[before..].to_vec();
        read_once_before_writing(&sent, &["project", VAULT]);
        reads.push(visibility_reads(&sent));
    }
    assert_eq!(reads[0].len(), 2);
    assert_eq!(
        reads[1].len(),
        2,
        "the second copy reuses nothing of the first"
    );
}

#[test]
fn overlapping_library_copies_read_visibility_independently() {
    use std::future::Future;
    use std::task::Poll;

    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use onetaskgraph_e2e_support::loopback::{self, After, Upstream};

    // Hold the first board response at the wire boundary. Merely stopping polling
    // after observing its reads can leave a mutation in flight on the HTTP worker.
    let paused = Arc::new(AtomicBool::new(false));
    let (release, released) = std::sync::mpsc::channel();
    let released = Arc::new(std::sync::Mutex::new(released));
    let setup = Setup::laid(false, |configured| {
        let endpoint = configured["sources"]["board"]["config"]["endpoint"]
            .as_str()
            .expect("an endpoint");
        let address = endpoint
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .and_then(|address| address.parse().ok())
            .expect("a loopback address");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
        configured["sources"]["board"]["config"]["endpoint"] =
            json!(format!("http://{}/graphql", listener.local_addr().unwrap()));
        let paused = Arc::clone(&paused);
        loopback::serve_each(listener, move || {
            let mut upstream = Upstream::new(address);
            let paused = Arc::clone(&paused);
            let released = Arc::clone(&released);
            move |sent: &loopback::Request, stream: &mut std::net::TcpStream| {
                let response = upstream.forward(&sent.bytes());
                if serde_json::from_slice::<Value>(sent.body()).is_ok_and(|request| {
                    request["query"] == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY
                }) && !paused.swap(true, Ordering::SeqCst)
                {
                    released
                        .lock()
                        .expect("not poisoned")
                        .recv_timeout(std::time::Duration::from_secs(30))
                        .expect("the first copy is released");
                }
                // Cancellation closes the client before its held response is released.
                if stream.write_all(&response).is_ok() {
                    After::KeepOpen
                } else {
                    After::Close
                }
            }
        });
    });
    for id in ["first", "second"] {
        setup.record(
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo\nrepositories: [github.com/{VAULT}]"),
            "Private work.",
        );
    }
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        setup.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &setup.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let request = |id: &str| onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(vec![
            format!("plan:{id}").parse().expect("an id"),
        ])
        .expect("one item"),
        scope: onetaskgraph_core::CopyScope::Tasks,
        destination: onetaskgraph_plugin_api::SourceName::new("board").expect("a name"),
        target_task: None,
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: false,
        no_link: false,
        expect: None,
        status_policy: onetaskgraph_core::StatusPolicy::Source,
        statuses: Vec::new(),
    };
    let first_request = request("first");
    let second_request = request("second");
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            let mut first = Box::pin(engine.copy(&first_request));
            // Pause while the first operation waits for its board response.
            std::future::poll_fn(|cx| {
                if paused.load(Ordering::SeqCst) {
                    return Poll::Ready(());
                }
                assert!(
                    first.as_mut().poll(cx).is_pending(),
                    "first copy must still be running"
                );
                cx.waker().wake_by_ref();
                Poll::Pending
            })
            .await;
            let before = setup.board.served().len();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                engine.copy(&second_request),
            )
            .await
            .expect("overlapping copy must not wait on the paused copy's visibility cells")
            .expect("overlapping copy lands");
            let sent = setup.board.served()[before..].to_vec();
            read_once_before_writing(&sent, &["project", VAULT]);
            // Cancel the still-live first copy. Its task-local reads must disappear.
            drop(first);
            release.send(()).expect("release the cancelled copy");
            setup.board.set_project_public(true);
            let before_retry = setup.board.served().len();
            let failure = engine
                .copy(&first_request)
                .await
                .expect_err("the changed board refuses");
            assert!(
                matches!(
                    failure,
                    onetaskgraph_core::EngineError::DestinationNotPrivate { .. }
                ),
                "{failure}"
            );
            let retry = setup.board.served()[before_retry..].to_vec();
            assert!(
                visibility_reads(&retry)
                    .iter()
                    .any(|read| read == "project")
            );
            assert!(!retry.iter().any(|(document, _)| is_mutation(document)));
        });
    let reads = visibility_reads(&setup.board.served());
    assert_eq!(reads.iter().filter(|read| *read == "project").count(), 3);
    assert_eq!(reads.iter().filter(|read| *read == VAULT).count(), 3);
}

#[test]
fn projects_and_documents_naming_a_public_repository_keep_their_issues_private() {
    for (kind, folder) in [("project", "projects"), ("document", "documents")] {
        let setup = Setup::laid(false, |_| {});
        setup.record(
            folder,
            "named",
            &format!("title: Named {kind}\nrepositories: [github.com/{OPEN}]"),
            "Private coordination.",
        );
        setup
            .board
            .set_repository_visibility(OPEN, RepositoryAnswer::Unseen);
        let arguments = [kind, "copy", "plan:named", "--to", "board"];
        let (failure, message, _) = setup.refused(&arguments);
        assert_eq!(failure, "visibility-unreadable", "{message}");
        assert!(message.contains(OPEN), "{message}");
        setup
            .board
            .set_repository_visibility(OPEN, RepositoryAnswer::Public);
        let (_, sent) = setup.ok(&arguments);
        read_once_before_writing(&sent.board, &["project", OPEN, CONFIGURED]);
        assert_eq!(
            created_in(&sent.board),
            [setup.board.repository_node_id(CONFIGURED)]
        );
        let landed = setup.landed(kind, "named", "board");
        assert_eq!(setup.issue_repository(&landed), CONFIGURED);
        assert_eq!(
            setup.shown(kind, &landed)["repositories"],
            json!([format!("github.com/{OPEN}")])
        );
    }
}

#[test]
fn private_locate_shares_named_repository_reads_and_refuses_an_unreadable_one() {
    let setup = Setup::laid(false, |_| {});
    for id in ["one", "two"] {
        setup.record(
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo\nrepositories: [github.com/{OPEN}]"),
            "Private coordination.",
        );
    }
    let arguments = ["task", "locate", "plan:one", "plan:two", "--to", "board"];
    setup
        .board
        .set_repository_visibility(OPEN, RepositoryAnswer::Unseen);
    let (failure, message, _) = setup.refused(&arguments);
    assert_eq!(failure, "unavailable", "{message}");
    assert!(message.contains(OPEN), "{message}");
    setup
        .board
        .set_repository_visibility(OPEN, RepositoryAnswer::Public);
    let mut with_json = arguments.to_vec();
    with_json.push("--json");
    let (answer, sent) = setup.ok(&with_json);
    let answer: Value = serde_json::from_str(&answer).expect("a locate answer");
    assert_eq!(answer["items"].as_array().expect("located items").len(), 2);
    for item in answer["items"].as_array().unwrap() {
        assert_eq!(
            item["creates_in"],
            json!({"repository": format!("github.com/{CONFIGURED}")})
        );
    }
    assert_eq!(visibility_reads(&sent.board), [OPEN]);
    assert!(!sent.board.iter().any(|(document, _)| is_mutation(document)));
}

#[test]
fn overlapping_unscoped_holds_do_not_reuse_visibility() {
    use onetaskgraph_plugin_api::{SourceName, SourcePlugin, Visibility, WriteTarget};

    let mut config = Value::Null;
    let setup = Setup::laid(false, |document| {
        config = document["sources"]["board"]["config"].clone();
    });
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        setup.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &setup.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("board").expect("a name"),
            &config,
            &loaded.secrets,
        )
        .expect("the backend builds");
    let target = WriteTarget::New {
        repositories: &[],
        project: None,
    };
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            source.hold_visibility();
            assert_eq!(
                source.visibility(&target).await.expect("first read"),
                Visibility::Private
            );
            // A second operation starts while the first hold remains open. It must see
            // the repository's changed reality rather than the first operation's read.
            setup
                .board
                .set_repository_visibility(CONFIGURED, RepositoryAnswer::Public);
            let before = setup.board.served().len();
            source.hold_visibility();
            assert_eq!(
                source.visibility(&target).await.expect("second read"),
                Visibility::Public
            );
            source.release_visibility();
            source.release_visibility();
            let sent = setup.board.served()[before..].to_vec();
            assert_eq!(visibility_reads(&sent).len(), 2, "{sent:#?}");
            assert!(
                sent.iter().all(|(_, body)| !body["query"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("mutation")),
                "visibility checks never mutate"
            );
        });
}

#[test]
fn a_public_task_uses_its_private_project_repository_with_and_without_caching() {
    for cached in [false, true] {
        let setup = Setup::laid(true, |document| {
            document["sources"]["board"]["cache"] = json!({"enabled": cached});
        });
        setup.plan();
        setup.record(
            "projects",
            "home",
            &format!("title: Home\nrepositories: [github.com/{LOCKER}]"),
            "The plan.",
        );
        let copy = ["project", "copy", "plan:home", "--to", "board", "--json"];
        let (_, sent) = setup.ok(&copy);
        read_once_before_writing(&sent.board, &["project", OPEN, VAULT, LOCKER]);
        assert!(
            !visibility_reads(&sent.board)
                .iter()
                .any(|read| read == CONFIGURED)
        );
        let open = setup.landed("task", "open", "board");
        assert_eq!(setup.issue_repository(&open), LOCKER);
        assert_eq!(
            setup.shown("task", &open)["repositories"],
            json!([format!("github.com/{OPEN}")])
        );
        assert_eq!(
            created_in(&sent.board)
                .iter()
                .filter(|id| **id == setup.board.repository_node_id(LOCKER))
                .count(),
            2
        );
        // A cached source must still check the live destination on the next operation.
        setup.board.set_project_public(true);
        setup.record(
            "tasks",
            "open",
            &format!(
                "title: Open step\nstatus: todo\nproject: home\nrepositories: [github.com/{OPEN}]"
            ),
            "Public work, revised.",
        );
        let refusal_args: Vec<_> = copy.into_iter().filter(|arg| *arg != "--json").collect();
        let (kind, _, refused) = setup.refused(&refusal_args);
        assert_eq!(kind, "destination-not-private");
        assert!(
            !refused
                .board
                .iter()
                .any(|(document, _)| is_mutation(document))
        );
    }
}

#[test]
fn direct_project_and_document_creates_scope_their_private_placement_reads() {
    for kind in ["project", "document"] {
        let setup = Setup::laid(false, |_| {});
        setup.record(
            "projects",
            "home",
            "title: Home\nclassification: private",
            "The plan.",
        );
        setup.ok(&["project", "copy", "plan:home", "--to", "board"]);
        let project = setup.landed("project", "home", "board");
        let native = project.strip_prefix("board:").expect("a board project");
        let body = setup.root.join("body.md");
        std::fs::write(&body, "Private coordination.").expect("a body");
        let mut arguments = vec![
            kind,
            "create",
            "board",
            "--id",
            "new-item",
            "--title",
            "New item",
            "--classification",
            "private",
            "--repository",
            "github.com/nickderobertis/openwidget",
            "--body-file",
            body.to_str().expect("a path"),
            "--json",
        ];
        if kind == "document" {
            arguments.extend(["--project", native]);
        }
        let (answer, sent) = setup.ok(&arguments);
        assert!(answer.contains("New item"), "{answer}");
        read_once_before_writing(&sent.board, &["project", OPEN, CONFIGURED]);
        assert_eq!(
            created_in(&sent.board),
            [setup.board.repository_node_id(CONFIGURED)]
        );
        let issue = setup
            .board
            .issues()
            .into_iter()
            .find(|issue| {
                issue["title"]
                    .as_str()
                    .is_some_and(|title| title.ends_with("New item"))
            })
            .expect("the created issue");
        let id = issue["id"].as_str().expect("an issue id");
        assert_eq!(
            setup.shown(kind, &format!("board:{id}"))["repositories"],
            json!([format!("github.com/{OPEN}")])
        );
        setup.board.set_project_public(true);
        let refusal_args: Vec<_> = arguments
            .into_iter()
            .filter(|arg| *arg != "--json")
            .collect();
        let (refusal, _, sent) = setup.refused(&refusal_args);
        assert_eq!(refusal, "destination-not-private");
        assert!(
            visibility_reads(&sent.board)
                .iter()
                .any(|read| read == "project")
        );
    }
}

#[test]
fn a_project_copy_visibility_rate_limit_holds_off_later_reads() {
    let setup = Setup::laid(false, |document| {
        document["sources"]["board"]["config"]["pacing"]["retry_budget_ms"] = json!(0);
    });
    setup.record(
        "projects",
        "home",
        "title: Home\nclassification: private",
        "The plan.",
    );
    setup.record(
        "tasks",
        "step",
        &format!("title: Step\nstatus: todo\nproject: home\nrepositories: [github.com/{VAULT}]"),
        "Private work.",
    );
    setup.board.rate_limit_project_visibility(3_600);
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        setup.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &setup.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let request = onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(vec!["plan:home".parse().expect("an id")])
            .expect("one item"),
        scope: onetaskgraph_core::CopyScope::Projects { tasks: true },
        destination: onetaskgraph_plugin_api::SourceName::new("board").expect("a name"),
        target_task: None,
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: false,
        no_link: false,
        expect: None,
        status_policy: onetaskgraph_core::StatusPolicy::Source,
        statuses: Vec::new(),
    };
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            let failure = engine
                .copy(&request)
                .await
                .expect_err("the task's visibility read is limited");
            assert!(
                matches!(
                    failure,
                    onetaskgraph_core::EngineError::VisibilityUnreadable {
                        error: onetaskgraph_plugin_api::SourceError::RateLimited { .. },
                        ..
                    }
                ),
                "{failure}"
            );
            let sent = setup.board.served();
            assert!(
                visibility_reads(&sent).iter().any(|read| read == "project"),
                "the private project preflight reached its board visibility check"
            );
            assert!(!sent.iter().any(|(document, _)| is_mutation(document)));
            let before = sent.len();
            let id = "board:missing".parse().expect("an id");
            let refused = engine.project(&id).await.expect("a reported refusal");
            assert!(matches!(
                refused.errors[0].error,
                onetaskgraph_plugin_api::SourceError::RateLimited { .. }
            ));
            assert_eq!(
                setup.board.served().len(),
                before,
                "the recorded hold sends nothing"
            );
        });
}
