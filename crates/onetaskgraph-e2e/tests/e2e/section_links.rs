//! Section links — a link to one heading of a record — copied into each destination's own form,
//! as README "Section links" states each one.
//!
//! The authored project these journeys copy holds the spike's five headings in one task
//! (`## What`, `## Budgets`, `### Budget demo-quota-seconds`, `### Step 2.b: retry (once)` and a
//! second `## Budgets`) and links to them from every kind of body: another task, the project's
//! description and a design document, by a path relative to the linking file and by the
//! referent's absolute location, and from the task and the document to their own headings.
//!
//! Every expected link here is spelled out from the contract rather than computed by the code
//! under test: each slug is written literally, a GitHub link from the issue number its
//! counterpart reports, and a Linear link from the id the loopback workspace's stored editor
//! state gives the heading it names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use onetaskgraph_e2e_support::linear_editor;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, document, empty_folder, github_projects_with_board,
    linear_empty_workspace_held,
};

/// The repository a second board's issues live in, beside the first board's.
const SECOND_REPOSITORY: &str = "nickderobertis/onetaskgraph-second";

/// Every kind a section link is copied between.
const KINDS: [&str; 3] = ["local-md", "github-projects", "linear"];

/// One heading: its text, GitHub's (and the authored form's) slug, and Linear's.
type HeadingForms = (&'static str, &'static str, &'static str);

/// The headings of the target task, in order.
const HEADINGS: [HeadingForms; 5] = [
    ("What", "what", "what"),
    ("Budgets", "budgets", "budgets"),
    (
        "Budget demo-quota-seconds",
        "budget-demo-quota-seconds",
        "budget-demo-quota-seconds",
    ),
    (
        "Step 2.b: retry (once)",
        "step-2b-retry-once",
        "step-2b-retry-once",
    ),
    ("Budgets", "budgets-1", "budgets"),
];

/// The design document's own heading.
const OVERVIEW: HeadingForms = ("Overview", "overview", "overview");

/// The target task's content.
fn target_body() -> String {
    "## What\n\nThe target. Jump to [the second budgets](#budgets-1).\n\n## Budgets\n\nFirst \
     budgets.\n\n### Budget demo-quota-seconds\n\nThe demo quota.\n\n### Step 2.b: retry \
     (once)\n\nRetry once.\n\n## Budgets\n\nSecond budgets.\n"
        .to_owned()
}

/// The referring task's content, linking by a path beside it and by the absolute location.
fn referrer_body(target: &str) -> String {
    format!("See [the demo](T.md#budget-demo-quota-seconds) and [the first]({target}#budgets).\n")
}

/// The project's description.
fn project_body() -> String {
    "The plan: [what it is](../tasks/T.md#what) and [the retry](../tasks/T.md#step-2b-retry-once).\n"
        .to_owned()
}

/// The design document's content: a budget table linking each heading, a link to its own
/// heading, and one to a heading the target does not hold.
fn design_body(target: &str) -> String {
    format!(
        "## Overview\n\n| Budget | Detail |\n| --- | --- |\n| first | [first](../tasks/T.md#budgets) \
         |\n| second | [second](../tasks/T.md#budgets-1) |\n| retry | [retry]({target}#step-2b-retry-once) \
         |\n\nBack to the [overview](#overview); [missing](../tasks/T.md#nowhere).\n"
    )
}

/// Write the authored project into a folder named `name` of the sandbox, answering the
/// source configuration naming it and the absolute path of its target task.
fn authored(sandbox: &Sandbox, name: &str) -> (Value, String) {
    let config = empty_folder(sandbox, name);
    let root = std::fs::canonicalize(sandbox.subdirectory(name)).expect("the root");
    for folder in ["projects", "tasks", "documents"] {
        std::fs::create_dir_all(root.join(folder)).expect("a folder");
    }
    let target = root.join("tasks/T.md").display().to_string();
    let files = [
        (
            "projects/P.md",
            "title: Section plan\nstatus: todo",
            project_body(),
        ),
        (
            "tasks/T.md",
            "title: Target\nstatus: todo\nproject: P",
            target_body(),
        ),
        (
            "tasks/U.md",
            "title: Referrer\nstatus: todo\nproject: P",
            referrer_body(&target),
        ),
        (
            "documents/D.md",
            "title: Design\nproject: P",
            design_body(&target),
        ),
    ];
    for (path, front, body) in files {
        std::fs::write(root.join(path), format!("---\n{front}\n---\n{body}")).expect("a record");
    }
    (json!({"plugin": "local-md", "config": config}), target)
}

/// One store a journey copies into or out of, and what reads it back.
pub(crate) enum Store {
    /// A folder of Markdown.
    Local,
    /// The loopback board.
    Github(GitHubBoardFields),
    /// The loopback Linear workspace.
    Linear(LinearWorkspace),
}

/// A store of `kind` in `sandbox` — a folder of Markdown under `folder`, a board, or a Linear
/// workspace of its own — and the source configuration naming it. A board beside `beside`'s
/// holds its issues in [`SECOND_REPOSITORY`], so two boards never spell one issue twice.
pub(crate) fn store(
    sandbox: &Sandbox,
    kind: &str,
    folder: &str,
    beside: Option<&Store>,
) -> (Store, Value) {
    match (kind, beside) {
        ("local-md", _) => (
            Store::Local,
            json!({"plugin": "local-md", "config": empty_folder(sandbox, folder)}),
        ),
        ("github-projects", Some(Store::Github(_))) => {
            let (mut config, board) = github_projects_with_board(sandbox);
            board.use_canonical_issue_urls();
            board.use_repository(SECOND_REPOSITORY);
            config["repository"] = json!(SECOND_REPOSITORY);
            (
                Store::Github(board),
                json!({"plugin": "github-projects", "config": config}),
            )
        }
        ("github-projects", _) => {
            let (config, board) = github_projects_with_board(sandbox);
            board.use_canonical_issue_urls();
            (
                Store::Github(board),
                json!({"plugin": "github-projects", "config": config}),
            )
        }
        _ => {
            let (config, workspace) = linear_empty_workspace_held(sandbox);
            (
                Store::Linear(workspace),
                json!({"plugin": "linear", "config": config}),
            )
        }
    }
}

/// Configure `sources` in `sandbox`, with both credentials the loopback board server answers and
/// the one the loopback Linear workspace does.
pub(crate) fn configure(sandbox: &Sandbox, sources: &[(&str, &Value)]) {
    let mut held = serde_json::Map::new();
    for (name, config) in sources {
        held.insert((*name).to_owned(), (*config).clone());
    }
    sandbox.project_document(&document(&Value::Object(held)));
    sandbox.secrets_file(
        "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nGITHUB_PROJECTS_FIXTURE_TOKEN_B=test-token-b\n\
         LINEAR_API_KEY=fixture-key\n",
    );
}

impl Store {
    /// What this store has been sent so far: every request a hosted one served, or every file
    /// of the folder at `root` and its bytes.
    fn mark(&self, root: Option<&Path>) -> Mark {
        match self {
            Self::Local => Mark::Files(files_under(root.expect("a folder's root"))),
            Self::Github(board) => Mark::Served(board.served_count()),
            Self::Linear(workspace) => Mark::Served(workspace.served_count()),
        }
    }

    /// Every write this store was sent since `mark`: each mutation a hosted one served, or each
    /// file of the folder at `root` created, changed or removed.
    fn writes_since(&self, mark: &Mark, root: Option<&Path>) -> Vec<String> {
        match (self, mark) {
            (Self::Local, Mark::Files(before)) => {
                let after = files_under(root.expect("a folder's root"));
                let mut changed: Vec<String> = after
                    .iter()
                    .filter(|(path, bytes)| before.get(*path) != Some(bytes))
                    .map(|(path, _)| path.clone())
                    .collect();
                changed.extend(
                    before
                        .keys()
                        .filter(|path| !after.contains_key(*path))
                        .cloned(),
                );
                changed
            }
            (Self::Github(board), Mark::Served(at)) => mutations(board.served_since(*at)),
            (Self::Linear(workspace), Mark::Served(at)) => mutations(workspace.served_since(*at)),
            _ => unreachable!("a mark of this store"),
        }
    }

    /// The link a record landed at `from` writes to the `ordinal`th heading of its text,
    /// `heading`, of the record landed at `to` — `None` for its own — by this store kind's
    /// contract.
    fn link(
        &self,
        sandbox: &Sandbox,
        from: &Landed,
        to: Option<&Landed>,
        heading: HeadingForms,
        ordinal: usize,
    ) -> String {
        let (text, github, linear) = heading;
        match self {
            Self::Local => match to {
                None => format!("#{github}"),
                Some(to) => format!(
                    "{}#{github}",
                    relative(&from.location(sandbox), &to.location(sandbox))
                ),
            },
            Self::Github(_) => match to {
                None => format!("#user-content-{github}"),
                Some(to) => format!("{}#user-content-{github}", www(&to.location(sandbox))),
            },
            Self::Linear(workspace) => {
                let at = to.unwrap_or(from);
                format!(
                    "{}#heading-{linear}-{}",
                    at.location(sandbox),
                    stored_id8(workspace, at, text, ordinal)
                )
            }
        }
    }

    /// The record's body as this store itself holds it, the metadata slot left out.
    fn raw(&self, landed: &Landed) -> String {
        let raw = match self {
            Self::Local => return String::new(),
            Self::Github(board) => board
                .body(landed.native())
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            Self::Linear(workspace) => workspace
                .long_form(landed.collection(), landed.native())
                .unwrap_or_default(),
        };
        raw.split("\n\n<!-- onetaskgraph.metadata")
            .next()
            .unwrap_or_default()
            .to_owned()
    }
}

/// The first 8 hex characters of the id the workspace stored the `ordinal`th heading of text
/// `text` of `at` under.
fn stored_id8(workspace: &LinearWorkspace, at: &Landed, text: &str, ordinal: usize) -> String {
    let ids = linear_editor::stored(
        &workspace
            .editor_state(at.collection(), at.native())
            .unwrap_or_else(|| panic!("{} holds an editor state", at.id)),
    );
    let id = ids
        .iter()
        .filter(|(held, _)| held == text)
        .nth(ordinal)
        .map(|(_, id)| id.replace('-', ""))
        .unwrap_or_else(|| panic!("{} stores heading {text} #{ordinal}: {ids:?}", at.id));
    id[..8].to_owned()
}

/// What a store held before a command.
enum Mark {
    /// Each file under a folder, and its bytes.
    Files(BTreeMap<String, Vec<u8>>),
    /// How many requests a hosted store had served.
    Served(usize),
}

/// Every file under `root`, and its bytes.
fn files_under(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.insert(
                    path.display().to_string(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    found
}

/// The mutations among `served`.
fn mutations(served: Vec<(String, Value)>) -> Vec<String> {
    served
        .into_iter()
        .map(|(document, _)| document)
        .filter(|document| document.trim_start().starts_with("mutation"))
        .collect()
}

/// `location`'s GitHub issue link on the `www.github.com` host GitHub's form writes.
fn www(location: &str) -> String {
    let rest = location
        .strip_prefix("https://github.com/")
        .unwrap_or_else(|| panic!("a canonical GitHub issue link: {location}"));
    assert!(rest.contains("/issues/"), "{location}");
    format!("https://www.github.com/{rest}")
}

/// `to` written relative to the directory of `from`, with `/` between its parts.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = Path::new(from)
        .parent()
        .expect("a file in a folder")
        .to_str()
        .unwrap()
        .split('/')
        .collect();
    let to: Vec<&str> = to.split('/').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/")
}

/// One record of a store, by its qualified id.
#[derive(Clone)]
pub(crate) struct Landed {
    /// `task`, `project` or `document`.
    verb: &'static str,
    /// Its qualified id.
    id: String,
}

impl Landed {
    fn native(&self) -> &str {
        self.id.split_once(':').expect("a qualified id").1
    }

    fn collection(&self) -> &'static str {
        match self.verb {
            "task" => "tasks",
            "project" => "projects",
            _ => "documents",
        }
    }

    /// Where its store says it is.
    fn location(&self, sandbox: &Sandbox) -> String {
        let shown = shown(sandbox, self.verb, &self.id);
        shown["location"]["path"]
            .as_str()
            .or_else(|| shown["location"]["url"].as_str())
            .unwrap_or_else(|| panic!("{} says where it is: {shown}", self.id))
            .to_owned()
    }

    /// Its content, read back through the store.
    fn content(&self, sandbox: &Sandbox) -> String {
        shown(sandbox, self.verb, &self.id)["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

/// Standard output of a run that had to succeed.
fn ok(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = sandbox
        .command()
        .args(arguments)
        .output()
        .expect("the binary runs");
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// One record as `show --json` answers it.
fn shown(sandbox: &Sandbox, verb: &str, id: &str) -> Value {
    let rendered = ok(sandbox, &[verb, "show", id, "--json"]);
    let response: Value = serde_json::from_str(&rendered).expect("show emits JSON");
    response["items"][0]["item"].clone()
}

/// What one copy reported: each source id, and its outcome.
fn outcomes(rendered: &str) -> BTreeMap<String, Value> {
    let report: Value = serde_json::from_str(rendered).expect("a copy emits JSON");
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| (item["source"].as_str().unwrap().to_owned(), item.clone()))
        .collect()
}

/// The authored records of the folder `name`, by their keys.
fn authored_ids(name: &str) -> BTreeMap<&'static str, Landed> {
    [
        ("P", "project"),
        ("T", "task"),
        ("U", "task"),
        ("D", "document"),
    ]
    .into_iter()
    .map(|(key, verb)| {
        (
            key,
            Landed {
                verb,
                id: format!("{name}:{key}"),
            },
        )
    })
    .collect()
}

/// Copy the project `from["P"]`, its tasks, and then its document `from["D"]`, into `to`,
/// answering where each landed, keyed as `from` is, and every outcome by those keys.
fn copy_plan(
    sandbox: &Sandbox,
    from: &BTreeMap<&'static str, Landed>,
    to: &str,
) -> (
    BTreeMap<&'static str, Landed>,
    BTreeMap<&'static str, Value>,
) {
    let project = outcomes(&ok(
        sandbox,
        &["project", "copy", &from["P"].id, "--to", to, "--json"],
    ));
    let documents = outcomes(&ok(
        sandbox,
        &["document", "copy", &from["D"].id, "--to", to, "--json"],
    ));
    let mut landed = BTreeMap::new();
    let mut reported = BTreeMap::new();
    for (key, source) in from {
        let report = if source.verb == "document" {
            &documents
        } else {
            &project
        };
        let outcome = report
            .get(&source.id)
            .unwrap_or_else(|| panic!("{} was copied: {report:#?}", source.id))
            .clone();
        let id = outcome["destination"]
            .as_str()
            .unwrap_or_else(|| panic!("{} landed: {outcome:#}", source.id))
            .to_owned();
        landed.insert(
            *key,
            Landed {
                verb: source.verb,
                id,
            },
        );
        reported.insert(*key, outcome);
    }
    (landed, reported)
}

/// What every authored body reads as once copied into `store` and landed as `landed`: the
/// authored text with each section link — and nothing else — written in that store's form.
fn expected_bodies(
    sandbox: &Sandbox,
    store: &Store,
    landed: &BTreeMap<&'static str, Landed>,
    authored_target: &str,
) -> BTreeMap<&'static str, String> {
    let link = |from: &str, to: Option<&str>, index: usize| {
        store.link(
            sandbox,
            &landed[from],
            to.map(|to| &landed[to]),
            HEADINGS[index],
            usize::from(index == 4),
        )
    };
    let target_there = landed["T"].location(sandbox);
    // A link to a heading the target does not hold keeps its fragment as it is, on the link
    // to the target — relative to the linking file, into a folder of Markdown.
    let missing = match store {
        Store::Local => relative(&landed["D"].location(sandbox), &target_there),
        _ => target_there,
    };
    let linked = |body: String, pairs: Vec<(String, String)>| {
        pairs.into_iter().fold(body, |body, (from, to)| {
            body.replace(&format!("]({from})"), &format!("]({to})"))
        })
    };
    let mut bodies = BTreeMap::new();
    bodies.insert(
        "T",
        linked(
            target_body(),
            vec![("#budgets-1".into(), link("T", None, 4))],
        ),
    );
    bodies.insert(
        "U",
        linked(
            referrer_body(authored_target),
            vec![
                (
                    "T.md#budget-demo-quota-seconds".into(),
                    link("U", Some("T"), 2),
                ),
                (
                    format!("{authored_target}#budgets"),
                    link("U", Some("T"), 1),
                ),
            ],
        ),
    );
    bodies.insert(
        "P",
        linked(
            project_body(),
            vec![
                ("../tasks/T.md#what".into(), link("P", Some("T"), 0)),
                (
                    "../tasks/T.md#step-2b-retry-once".into(),
                    link("P", Some("T"), 3),
                ),
            ],
        ),
    );
    bodies.insert(
        "D",
        linked(
            design_body(authored_target),
            vec![
                ("../tasks/T.md#budgets".into(), link("D", Some("T"), 1)),
                ("../tasks/T.md#budgets-1".into(), link("D", Some("T"), 4)),
                (
                    format!("{authored_target}#step-2b-retry-once"),
                    link("D", Some("T"), 3),
                ),
                (
                    "#overview".into(),
                    store.link(sandbox, &landed["D"], None, OVERVIEW, 0),
                ),
                ("../tasks/T.md#nowhere".into(), format!("{missing}#nowhere")),
            ],
        ),
    );
    bodies
}

/// Hold every body `landed` names in `store` to the authored ones in that store's form, its
/// raw bodies to the store's own markup, and its report to the one fragment naming no heading.
fn holds_the_form(
    sandbox: &Sandbox,
    what: &str,
    store: &Store,
    landed: &BTreeMap<&'static str, Landed>,
    authored_target: &str,
) {
    let expected = expected_bodies(sandbox, store, landed, authored_target);
    for (key, body) in &expected {
        assert_eq!(
            landed[key].content(sandbox).lines().collect::<Vec<_>>(),
            body.lines().collect::<Vec<_>>(),
            "{what}: {key} reads back as authored, its section links in the destination's form"
        );
    }
    match store {
        Store::Github(_) => {
            // The issue page gives a heading no id: the body carries one before each.
            let raw = store.raw(&landed["T"]);
            for (text, slug) in [
                ("## What", "what"),
                ("## Budgets\n\nFirst", "budgets"),
                ("### Budget demo-quota-seconds", "budget-demo-quota-seconds"),
                ("### Step 2.b: retry (once)", "step-2b-retry-once"),
                ("## Budgets\n\nSecond", "budgets-1"),
            ] {
                assert!(
                    raw.contains(&format!("<a id=\"{slug}\"></a>\n{text}")),
                    "{what}: the anchor before {text}: {raw}"
                );
            }
            assert!(
                store
                    .raw(&landed["D"])
                    .contains("<a id=\"overview\"></a>\n## Overview"),
                "{what}"
            );
        }
        Store::Linear(workspace) => {
            for key in ["P", "T", "U", "D"] {
                let raw = store.raw(&landed[key]);
                assert!(
                    !raw.contains("<a "),
                    "{what}: {key} holds no anchor markup: {raw}"
                );
            }
            // The two `## Budgets` are told apart by the ids Linear stored them under.
            assert_ne!(
                stored_id8(workspace, &landed["T"], "Budgets", 0),
                stored_id8(workspace, &landed["T"], "Budgets", 1),
                "{what}"
            );
        }
        Store::Local => {
            for key in ["P", "T", "U", "D"] {
                let path = landed[key].location(sandbox);
                let file = std::fs::read_to_string(&path).expect("the record's file");
                assert!(
                    !file.contains("<a "),
                    "{what}: {path} holds no anchor markup"
                );
            }
        }
    }
}

/// Repeat the copy of `from` into `to`, holding `store` to receiving no write — `root` the
/// folder's, for a folder of Markdown — and every record to landing where it did.
fn repeats_writing_nothing(
    sandbox: &Sandbox,
    what: &str,
    from: &BTreeMap<&'static str, Landed>,
    to: &str,
    store: &Store,
    root: Option<&Path>,
    landed: &BTreeMap<&'static str, Landed>,
) {
    let mark = store.mark(root);
    let (again, _) = copy_plan(sandbox, from, to);
    assert_eq!(
        store.writes_since(&mark, root),
        Vec::<String>::new(),
        "{what}: a repeat copy of the unchanged project writes nothing"
    );
    for (key, record) in landed {
        assert_eq!(again[key].id, record.id, "{what}: {key} landed once");
    }
}

/// The folder a local store named `name` keeps its records in.
fn root_of(sandbox: &Sandbox, name: &str) -> PathBuf {
    std::fs::canonicalize(sandbox.subdirectory(name)).expect("a root")
}

/// A source a journey copies the authored project out of.
struct Source {
    /// The store it is.
    store: Store,
    /// Every source configuration the journey names, the authored folder's included.
    sources: Vec<(&'static str, Value)>,
    /// The absolute path of the authored target task.
    authored_target: String,
    /// Its records, once it holds them: the authored folder's at once, a tracker's once seeded.
    ids: BTreeMap<&'static str, Landed>,
}

/// A source of `kind` named `src` holding the authored project in that kind's own form: the
/// authored folder itself, or a tracker an earlier copy writes it into, which a journey seeds
/// through that copy and holds to the tracker's form before anything is copied out of it.
fn source_of(sandbox: &Sandbox, kind: &str) -> Source {
    let (authored_config, authored_target) = authored(sandbox, "authored");
    if kind == "local-md" {
        return Source {
            store: Store::Local,
            sources: vec![("authored", authored_config)],
            authored_target,
            ids: authored_ids("authored"),
        };
    }
    let (store, config) = store(sandbox, kind, "src", None);
    Source {
        store,
        sources: vec![("authored", authored_config), ("src", config)],
        authored_target,
        ids: BTreeMap::new(),
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback stand-ins this suite already drives for both hosted destinations. AGENTS.md puts a journey crossing more than one plugin in this suite.
#[test]
fn authored_section_links_land_in_each_destinations_form_and_a_repeat_copy_writes_nothing() {
    for kind in KINDS {
        let sandbox = Sandbox::new();
        let (authored_config, authored_target) = authored(&sandbox, "plans");
        let (destination, config) = store(&sandbox, kind, "dest", None);
        configure(&sandbox, &[("plans", &authored_config), ("dest", &config)]);
        let from = authored_ids("plans");

        let (landed, reported) = copy_plan(&sandbox, &from, "dest");
        holds_the_form(&sandbox, kind, &destination, &landed, &authored_target);
        // The fragment naming no heading of the target is carried and counted, not guessed.
        assert_eq!(
            reported["D"]["section_links_unresolved"],
            json!(1),
            "{kind}: {:#}",
            reported["D"]
        );
        let root = (kind == "local-md").then(|| root_of(&sandbox, "dest"));
        repeats_writing_nothing(
            &sandbox,
            kind,
            &from,
            "dest",
            &destination,
            root.as_deref(),
            &landed,
        );
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: folders of Markdown and the loopback stand-ins this suite already drives, two boards of one loopback server and two loopback Linear workspaces among them.
#[test]
fn every_pair_of_kinds_writes_the_others_section_links_in_its_own_form_and_repeats_write_nothing() {
    for from in KINDS {
        for to in KINDS {
            let what = format!("{from} -> {to}");
            let sandbox = Sandbox::new();
            let Source {
                store: source,
                mut sources,
                authored_target,
                mut ids,
            } = source_of(&sandbox, from);
            let (destination, config) = store(&sandbox, to, "dst", Some(&source));
            sources.push(("dst", config));
            let named: Vec<(&str, &Value)> = sources
                .iter()
                .map(|(name, config)| (*name, config))
                .collect();
            configure(&sandbox, &named);
            if from != "local-md" {
                // The source holds the project as a copy from a folder of Markdown wrote it.
                let (seeded, _) = copy_plan(&sandbox, &authored_ids("authored"), "src");
                holds_the_form(
                    &sandbox,
                    &format!("{what} (seeded)"),
                    &source,
                    &seeded,
                    &authored_target,
                );
                ids = seeded;
            }

            let (landed, reported) = copy_plan(&sandbox, &ids, "dst");
            holds_the_form(&sandbox, &what, &destination, &landed, &authored_target);
            assert_eq!(
                reported["D"]["section_links_unresolved"],
                json!(1),
                "{what}: {:#}",
                reported["D"]
            );
            let root = (to == "local-md").then(|| root_of(&sandbox, "dst"));
            repeats_writing_nothing(
                &sandbox,
                &what,
                &ids,
                "dst",
                &destination,
                root.as_deref(),
                &landed,
            );
        }
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: two folders of Markdown, one loopback board and one loopback Linear workspace.
#[test]
fn a_chain_of_whole_project_copies_ends_with_the_authored_links_and_every_repeat_writes_nothing() {
    let sandbox = Sandbox::new();
    let (authored_config, authored_target) = authored(&sandbox, "authored");
    let (board, board_config) = store(&sandbox, "github-projects", "board", None);
    let (linear, linear_config) = store(&sandbox, "linear", "linear", None);
    let (_, last_config) = store(&sandbox, "local-md", "last", None);
    configure(
        &sandbox,
        &[
            ("authored", &authored_config),
            ("board", &board_config),
            ("linear", &linear_config),
            ("last", &last_config),
        ],
    );
    let authored = authored_ids("authored");
    let (on_board, _) = copy_plan(&sandbox, &authored, "board");
    holds_the_form(&sandbox, "first hop", &board, &on_board, &authored_target);
    let (in_linear, _) = copy_plan(&sandbox, &on_board, "linear");
    holds_the_form(
        &sandbox,
        "second hop",
        &linear,
        &in_linear,
        &authored_target,
    );
    let (at_last, _) = copy_plan(&sandbox, &in_linear, "last");
    // The authored form again: each link resolves, relative to its record's file, to the
    // counterpart of the record the authored link named, its `#<slug>` byte-equal to the
    // authored one. The path names the last hop's file, by the manager's ruling.
    holds_the_form(
        &sandbox,
        "third hop",
        &Store::Local,
        &at_last,
        &authored_target,
    );
    for (key, body) in expected_bodies(&sandbox, &Store::Local, &at_last, &authored_target) {
        let fragments = |text: &str| -> Vec<String> {
            text.match_indices("](")
                .filter_map(|(at, _)| {
                    let rest = &text[at + 2..];
                    let end = rest.find(')')?;
                    rest[..end].split_once('#').map(|(_, slug)| slug.to_owned())
                })
                .collect()
        };
        let authored_body = match key {
            "T" => target_body(),
            "U" => referrer_body(&authored_target),
            "P" => project_body(),
            _ => design_body(&authored_target),
        };
        assert_eq!(
            fragments(&body),
            fragments(&authored_body),
            "{key}: every fragment is the authored one"
        );
    }

    let root = root_of(&sandbox, "last");
    repeats_writing_nothing(
        &sandbox,
        "first hop",
        &authored,
        "board",
        &board,
        None,
        &on_board,
    );
    repeats_writing_nothing(
        &sandbox,
        "second hop",
        &on_board,
        "linear",
        &linear,
        None,
        &in_linear,
    );
    repeats_writing_nothing(
        &sandbox,
        "third hop",
        &in_linear,
        "last",
        &Store::Local,
        Some(&root),
        &at_last,
    );
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: one loopback board, one loopback Linear workspace and a folder of Markdown.
#[test]
fn hand_written_tracker_links_are_read_by_what_they_name_and_one_outside_the_project_is_left() {
    // A board whose bodies a person edited by hand: a link on GitHub's canonical host, and one
    // to an issue the copied project does not hold.
    let sandbox = Sandbox::new();
    let (authored_config, _) = authored(&sandbox, "authored");
    let (board, board_config) = store(&sandbox, "github-projects", "board", None);
    let (linear, linear_config) = store(&sandbox, "linear", "linear", None);
    let (_, last_config) = store(&sandbox, "local-md", "last", None);
    configure(
        &sandbox,
        &[
            ("authored", &authored_config),
            ("board", &board_config),
            ("linear", &linear_config),
            ("last", &last_config),
        ],
    );
    let (on_board, _) = copy_plan(&sandbox, &authored_ids("authored"), "board");
    let Store::Github(fields) = &board else {
        unreachable!("a board")
    };
    let target = on_board["T"].location(&sandbox);
    let outside =
        "https://github.com/nickderobertis/onetaskgraph/issues/999999#user-content-budgets";
    let referrer = on_board["U"].native().to_owned();
    let held = fields.body(&referrer).as_str().unwrap().to_owned();
    let edited = held.replacen(
        "See ",
        &format!(
            "By hand: [canonical]({target}#user-content-budgets-1), [elsewhere]({outside}).\n\nSee "
        ),
        1,
    );
    fields.edit_body(&referrer, &edited);

    let (into_last, _) = copy_plan(&sandbox, &on_board, "last");
    let copied = into_last["U"].content(&sandbox);
    let local_target = relative(
        &into_last["U"].location(&sandbox),
        &into_last["T"].location(&sandbox),
    );
    assert!(
        copied.contains(&format!("[canonical]({local_target}#budgets-1)")),
        "the canonical-host link names the second budgets: {copied}"
    );
    assert!(
        copied.contains(&format!("[elsewhere]({outside})")),
        "a link to an issue outside the project is left as it is: {copied}"
    );

    // A Linear workspace whose link a person wrote with the right id and a wrong slug.
    let (in_linear, _) = copy_plan(&sandbox, &on_board, "linear");
    let Store::Linear(workspace) = &linear else {
        unreachable!("a workspace")
    };
    let id8 = stored_id8(workspace, &in_linear["T"], "Budgets", 1);
    let linear_target = in_linear["T"].location(&sandbox);
    let issue = in_linear["U"].native().to_owned();
    let long_form = workspace.long_form("tasks", &issue).unwrap();
    workspace.edit_description(
        &issue,
        &long_form.replacen(
            "See ",
            &format!("By hand: [wrong slug]({linear_target}#heading-not-its-slug-{id8}).\n\nSee "),
            1,
        ),
    );
    let (from_linear, _) = copy_plan(&sandbox, &in_linear, "last");
    let copied = from_linear["U"].content(&sandbox);
    let local_target = relative(
        &from_linear["U"].location(&sandbox),
        &from_linear["T"].location(&sandbox),
    );
    assert!(
        copied.contains(&format!("[wrong slug]({local_target}#budgets-1)")),
        "a Linear link is read by its id, whatever its slug says: {copied}"
    );
}

/// One folder of Markdown named `name` holding the project `P`, `target` filed under it at
/// `path` with `content`, and a design document `D` linking each of `slugs` there.
fn linking_folder(
    sandbox: &Sandbox,
    name: &str,
    target: (&str, &str),
    content: &str,
    slugs: &[&str],
) -> Value {
    let config = empty_folder(sandbox, name);
    let root = root_of(sandbox, name);
    let (folder, file) = target;
    for each in ["projects", "tasks", "documents"] {
        std::fs::create_dir_all(root.join(each)).expect("a folder");
    }
    let front = if folder == "tasks" {
        "title: Target\nstatus: todo\nproject: P"
    } else {
        "title: Target\nproject: P"
    };
    std::fs::write(
        root.join("projects/P.md"),
        "---\ntitle: Section plan\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("the project");
    std::fs::write(
        root.join(folder).join(format!("{file}.md")),
        format!("---\n{front}\n---\n{content}"),
    )
    .expect("the target");
    let links: String = slugs
        .iter()
        .enumerate()
        .map(|(index, slug)| format!("- [heading {index}](../{folder}/{file}.md#{slug})\n"))
        .collect();
    std::fs::write(
        root.join("documents/D.md"),
        format!("---\ntitle: Design\nproject: P\n---\n## Links\n\n{links}"),
    )
    .expect("the design document");
    json!({"plugin": "local-md", "config": config})
}

/// The `fragment` of each `[heading <n>](…)` link of `content`, in order.
fn heading_links(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| line.strip_prefix("- [heading "))
        .filter_map(|rest| rest.split_once("](").map(|(_, link)| link))
        .map(|link| link.trim_end_matches(')').to_owned())
        .collect()
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace, serving editor states captured from real Linear records.
#[test]
fn every_captured_linear_heading_is_linked_by_the_fragment_its_stored_id_gives_it() {
    for (sample, folder) in [
        ("TES2-773", "tasks"),
        ("section-links-spike-plain-document", "documents"),
    ] {
        let captured = linear_editor::captured(sample);
        let markdown = captured["markdown"]
            .as_str()
            .expect("the captured Markdown");
        let state = captured["state"].as_str().expect("the captured state");
        assert!(
            linear_editor::holds(state, markdown),
            "{sample}: the captured state holds the captured Markdown's headings"
        );
        let expected: Vec<String> = captured["headings"]
            .as_array()
            .expect("headings")
            .iter()
            .map(|heading| {
                let fragment = heading["fragment"].as_str().expect("a fragment");
                assert!(
                    ["copied", "derived"].contains(&heading["source"].as_str().unwrap()),
                    "{sample}: each fragment says where it came from"
                );
                assert!(!fragment.is_empty() && !heading["id"].as_str().unwrap().is_empty());
                fragment.to_owned()
            })
            .collect();
        assert_eq!(expected.len(), 5, "{sample}: the spike's five headings");
        let slugs: Vec<&str> = HEADINGS.iter().map(|(_, github, _)| *github).collect();

        let sandbox = Sandbox::new();
        let plans = linking_folder(&sandbox, "plans", (folder, "X"), markdown, &slugs);
        let (linear, config) = store(&sandbox, "linear", "linear", None);
        configure(&sandbox, &[("plans", &plans), ("linear", &config)]);
        let Store::Linear(workspace) = &linear else {
            unreachable!("a workspace")
        };
        ok(&sandbox, &["project", "copy", "plans:P", "--to", "linear"]);
        if folder == "documents" {
            ok(&sandbox, &["document", "copy", "plans:X", "--to", "linear"]);
        }
        let verb = if folder == "tasks" {
            "task"
        } else {
            "document"
        };
        let target = shown(&sandbox, verb, "plans:X");
        let copied = target["metadata"]["onetaskgraph.copies"]["linear"]
            .as_str()
            .unwrap_or_else(|| panic!("{sample}: the target records its copy: {target:#}"))
            .to_owned();
        let target = Landed { verb, id: copied };
        // The record as a person's edit in Linear's editor leaves it: holding the ids Linear
        // gave the captured record's headings.
        workspace.set_editor_state(folder, target.native(), state);

        let rendered = ok(
            &sandbox,
            &["document", "copy", "plans:D", "--to", "linear", "--json"],
        );
        let design = outcomes(&rendered)["plans:D"]["destination"]
            .as_str()
            .expect("the document landed")
            .to_owned();
        let design = Landed {
            verb: "document",
            id: design,
        };
        let url = target.location(&sandbox);
        assert_eq!(
            heading_links(&design.content(&sandbox)),
            expected
                .iter()
                .map(|fragment| format!("{url}#{fragment}"))
                .collect::<Vec<_>>(),
            "{sample}: each heading is linked by the fragment its fixture states"
        );
    }
}

/// A sandbox holding the authored project and an empty loopback Linear workspace, the latter
/// told `prepare` first.
fn into_linear(prepare: impl FnOnce(&LinearWorkspace)) -> (Sandbox, LinearWorkspace, String) {
    let sandbox = Sandbox::new();
    let (authored_config, target) = authored(&sandbox, "plans");
    let (linear, config) = store(&sandbox, "linear", "linear", None);
    configure(
        &sandbox,
        &[("plans", &authored_config), ("linear", &config)],
    );
    let Store::Linear(workspace) = linear else {
        unreachable!("a workspace")
    };
    prepare(&workspace);
    (sandbox, workspace, target)
}

/// Run `arguments` with `variables` set, answering standard output.
fn ok_with(sandbox: &Sandbox, variables: &[(&str, &str)], arguments: &[&str]) -> String {
    let mut command = sandbox.command();
    for (name, value) in variables {
        command.env(name, value);
    }
    let output = command.args(arguments).output().expect("the binary runs");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    stdout(&output)
}

/// Whether `document` reads the record whose id `variables` names, at the interface whose
/// root field is `root`.
fn reads(document: &str, variables: &Value, root: &str, id: &str) -> bool {
    document.trim_start().starts_with("query")
        && document.contains(&format!("{root}(id:$id)"))
        && variables["id"] == json!(id)
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace, whose request log is what is held.
#[test]
fn linear_heading_ids_come_from_each_writes_own_answer_and_cost_no_request() {
    // A project whose every link points at a record written earlier in the copy: the project
    // record links to nothing and the referring task follows the task it names.
    let backwards = |sandbox: &Sandbox| {
        let root = root_of(sandbox, "plans");
        std::fs::write(
            root.join("projects/P.md"),
            "---\ntitle: Section plan\nstatus: todo\n---\nThe plan.\n",
        )
        .expect("the project");
        std::fs::write(
            root.join("tasks/T.md"),
            format!(
                "---\ntitle: Target\nstatus: todo\nproject: P\n---\n{}",
                target_body().replace(" Jump to [the second budgets](#budgets-1).", "")
            ),
        )
        .expect("the target");
    };
    let mut counts = Vec::new();
    for form in ["heading-link", "record-url"] {
        let (sandbox, workspace, _) = into_linear(|_| {});
        backwards(&sandbox);
        let variables = [(
            onetaskgraph_core::section_links::LINEAR_SECTION_FORM_VARIABLE,
            form,
        )];
        let rendered = ok_with(
            &sandbox,
            &variables,
            &["project", "copy", "plans:P", "--to", "linear", "--json"],
        );
        let served = workspace.served();
        let first_write = served
            .iter()
            .position(|(document, _)| document.trim_start().starts_with("mutation"))
            .expect("the copy writes");
        let read_after = served[first_write..]
            .iter()
            .filter(|(document, _)| !document.trim_start().starts_with("mutation"))
            .map(|(document, _)| document.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            read_after,
            Vec::<String>::new(),
            "{form}: no read follows a write: every heading id is the write's own answer"
        );
        let referrer = outcomes(&rendered)["plans:U"]["destination"]
            .as_str()
            .expect("the referrer landed")
            .to_owned();
        let content = Landed {
            verb: "task",
            id: referrer,
        }
        .content(&sandbox);
        assert_eq!(
            content.contains("#heading-budget-demo-quota-seconds-"),
            form == "heading-link",
            "{form}: {content}"
        );
        counts.push(served.len());
    }
    assert_eq!(
        counts[0], counts[1],
        "Linear's heading link costs exactly the requests the record URL alone does"
    );

    // A same-record link, and a project linking a task written after it: finished once each
    // target has landed, in one update each, which the report counts.
    let (sandbox, workspace, _) = into_linear(|_| {});
    let reported = outcomes(&ok(
        &sandbox,
        &["project", "copy", "plans:P", "--to", "linear", "--json"],
    ));
    for key in ["plans:P", "plans:T"] {
        assert_eq!(
            reported[key]["section_link_updates"],
            json!(1),
            "{key}: {:#}",
            reported[key]
        );
    }
    assert_eq!(reported["plans:U"].get("section_link_updates"), None);
    let target = reported["plans:T"]["destination"].as_str().unwrap();
    let target = Landed {
        verb: "task",
        id: target.to_owned(),
    };
    let id8 = stored_id8(&workspace, &target, "Budgets", 1);
    assert!(
        target.content(&sandbox).contains(&format!(
            "{}#heading-budgets-{id8}",
            target.location(&sandbox)
        )),
        "the same-record link names the second budgets' stored id"
    );

    // Where Linear rewrote the record, the link is finished in the follow-up the copy already
    // sends for it, and no update is added.
    let (sandbox, workspace, _) = into_linear(|workspace| {
        workspace.rewrite_markdown_on_write(crate::fixtures::LinearMarkdown::Measured);
    });
    // A bullet Linear stores as `* `, so it rewrites the target and its follow-up is sent.
    let file = root_of(&sandbox, "plans").join("tasks/T.md");
    let held = std::fs::read_to_string(&file).expect("the target");
    std::fs::write(
        &file,
        format!("{held}\n- A bullet Linear stores otherwise.\n"),
    )
    .expect("the target");
    let reported = outcomes(&ok(
        &sandbox,
        &["project", "copy", "plans:P", "--to", "linear", "--json"],
    ));
    let target = Landed {
        verb: "task",
        id: reported["plans:T"]["destination"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    assert_eq!(reported["plans:T"].get("section_link_updates"), None);
    let id8 = stored_id8(&workspace, &target, "Budgets", 1);
    assert!(
        target
            .content(&sandbox)
            .contains(&format!("#heading-budgets-{id8}")),
        "{}",
        target.content(&sandbox)
    );

    // A later copy that rewrites only the referring document reads each target once.
    let mark = workspace.served_count();
    ok(&sandbox, &["document", "copy", "plans:D", "--to", "linear"]);
    let native = target.native().to_owned();
    let read = workspace
        .served_since(mark)
        .iter()
        .filter(|(document, variables)| reads(document, variables, "issue", &native))
        .count();
    assert_eq!(read, 1, "the target the design document links is read once");
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_linear_heading_its_record_does_not_store_is_linked_by_the_record_alone_and_counted() {
    // The served state holds no heading of that text at that position.
    let (sandbox, workspace, _) = into_linear(|_| {});
    let landed = outcomes(&ok(
        &sandbox,
        &["project", "copy", "plans:P", "--to", "linear", "--json"],
    ));
    let target = Landed {
        verb: "task",
        id: landed["plans:T"]["destination"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    workspace.set_editor_state(
        "tasks",
        target.native(),
        &linear_editor::restated(None, "## What\n", "only-what"),
    );
    let reported = outcomes(&ok(
        &sandbox,
        &["document", "copy", "plans:D", "--to", "linear", "--json"],
    ));
    let url = target.location(&sandbox);
    let design = Landed {
        verb: "document",
        id: reported["plans:D"]["destination"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    let content = design.content(&sandbox);
    assert!(
        content.contains(&format!("[first]({url})"))
            && content.contains(&format!("[second]({url})")),
        "{content}"
    );
    assert_eq!(
        reported["plans:D"]["section_links_unaddressable"],
        json!(3),
        "{:#}",
        reported["plans:D"]
    );

    // A write whose answer carries no state at all.
    let (sandbox, _, _) = into_linear(LinearWorkspace::answer_no_editor_state);
    let reported = outcomes(&ok(
        &sandbox,
        &["project", "copy", "plans:P", "--to", "linear", "--json"],
    ));
    let target = Landed {
        verb: "task",
        id: reported["plans:T"]["destination"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    let content = target.content(&sandbox);
    assert!(
        content.contains(&format!(
            "[the second budgets]({})",
            target.location(&sandbox)
        )),
        "{content}"
    );
    assert_eq!(
        reported["plans:T"]["section_links_unaddressable"],
        json!(1),
        "{:#}",
        reported["plans:T"]
    );
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback stand-ins this suite already drives for both hosted destinations.
#[test]
fn a_comment_carried_on_copy_has_its_section_links_written_in_the_destinations_form() {
    for kind in KINDS {
        let sandbox = Sandbox::new();
        let (authored_config, _) = authored(&sandbox, "plans");
        let referrer = root_of(&sandbox, "plans").join("tasks/U.md");
        let held = std::fs::read_to_string(&referrer).expect("the referrer");
        std::fs::write(
            &referrer,
            format!(
                "{held}\n## Comments\n\n<!-- onetaskgraph:comment id=\"c1\" \
                 created_at=\"2026-11-02T12:00:00Z\" updated_at=\"2026-11-02T12:00:00Z\" -->\n\
                 ### comment — 2026-11-02T12:00:00Z\n\nThe bar is [the second \
                 budgets](T.md#budgets-1).\n\n<!-- /onetaskgraph:comment -->\n"
            ),
        )
        .expect("the comment");
        let (destination, config) = store(&sandbox, kind, "dest", None);
        configure(&sandbox, &[("plans", &authored_config), ("dest", &config)]);
        let copy = [
            "project",
            "copy",
            "plans:P",
            "--to",
            "dest",
            "--comments",
            "--json",
        ];
        let reported = outcomes(&ok(&sandbox, &copy));
        let landed = |key: &str| Landed {
            verb: "task",
            id: reported[&format!("plans:{key}")]["destination"]
                .as_str()
                .unwrap()
                .to_owned(),
        };
        let (target, comments_on) = (landed("T"), landed("U"));
        let expected = destination.link(&sandbox, &comments_on, Some(&target), HEADINGS[4], 1);
        let listed: Value = serde_json::from_str(&ok(
            &sandbox,
            &["task", "comment", "list", &comments_on.id, "--json"],
        ))
        .expect("comments as JSON");
        let bodies: Vec<&str> = listed["comments"]
            .as_array()
            .expect("comments")
            .iter()
            .filter_map(|comment| comment["body"].as_str())
            .collect();
        assert!(
            bodies
                .iter()
                .any(|body| body.contains(&format!("[the second budgets]({expected})"))),
            "{kind}: the carried comment names the destination's heading: {bodies:#?}"
        );
        let root = (kind == "local-md").then(|| root_of(&sandbox, "dest"));
        let mark = destination.mark(root.as_deref());
        let before = root.as_deref().map(files_under);
        let before_bodies: Vec<String> = if root.is_none() {
            ["P", "T", "U"]
                .iter()
                .map(|key| masked(&destination.raw(&landed(key))))
                .collect()
        } else {
            Vec::new()
        };
        ok(&sandbox, &copy);
        let written = destination.writes_since(&mark, root.as_deref());
        if let (Some(root), Some(before)) = (root.as_deref(), before) {
            // The one exemption the manager ruled on evidence from base `ebce65c3`: from a folder
            // of Markdown, a second `--comments` copy may move a record's
            // `onetaskgraph.copied_version` line — the copy's own link write moved the source
            // file's token after it was recorded — and no other line of any file.
            let after = files_under(root);
            for path in &written {
                let lines = |held: &BTreeMap<String, Vec<u8>>| -> Vec<String> {
                    String::from_utf8_lossy(held.get(path).map_or(&[][..], Vec::as_slice))
                        .lines()
                        .filter(|line| {
                            !line
                                .trim_start()
                                .starts_with("onetaskgraph.copied_version:")
                        })
                        .map(str::to_owned)
                        .collect()
                };
                assert_eq!(
                    lines(&before),
                    lines(&after),
                    "{kind}: {path} changed beyond its copied_version line"
                );
            }
            let mark = destination.mark(Some(root));
            ok(&sandbox, &copy);
            assert_eq!(
                destination.writes_since(&mark, Some(root)),
                Vec::<String>::new(),
                "{kind}: a third copy writes no file"
            );
        } else {
            // The same exemption at a hosted destination: the second copy may move nothing of a
            // stored body but its record's `onetaskgraph.copied_version`, and a third copy sends
            // no mutation at all.
            let held: Vec<String> = ["P", "T", "U"]
                .iter()
                .map(|key| masked(&destination.raw(&landed(key))))
                .collect();
            assert_eq!(held, before_bodies, "{kind}: only copied_version moved");
            let mark = destination.mark(None);
            ok(&sandbox, &copy);
            assert_eq!(
                destination.writes_since(&mark, None),
                Vec::<String>::new(),
                "{kind}: a third copy writes nothing"
            );
            let _ = written;
        }
    }
}

/// `text` with every local-md change token (`md1|<64 hex>`) blanked: the one value the ruled
/// `--comments` record refresh may move.
fn masked(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("md1|") {
        out.push_str(&rest[..at + 4]);
        let tail = &rest[at + 4..];
        let hex = tail.chars().take_while(char::is_ascii_hexdigit).count();
        out.push_str("<token>");
        rest = &tail[hex..];
    }
    out.push_str(rest);
    out
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown, one loopback board and the loopback Linear workspace.
#[test]
fn links_linear_stores_in_angle_brackets_are_finished_and_a_fan_out_names_its_own_copies() {
    // What the live check observed of TES2: every link destination stored in angle brackets.
    // A link finished after its record was first written is finished there all the same.
    let sandbox = Sandbox::new();
    let (authored_config, _) = authored(&sandbox, "authored");
    let (board, board_config) = store(&sandbox, "github-projects", "board", None);
    let (linear, linear_config) = store(&sandbox, "linear", "linear", None);
    configure(
        &sandbox,
        &[
            ("authored", &authored_config),
            ("board", &board_config),
            ("linear", &linear_config),
        ],
    );
    let Store::Linear(workspace) = &linear else {
        unreachable!("a workspace")
    };
    workspace.rewrite_markdown_on_write(crate::fixtures::LinearMarkdown::Angled);
    let authored = authored_ids("authored");
    let (direct, _) = copy_plan(&sandbox, &authored, "linear");
    let angled = |at: &Landed, to: Option<&Landed>, heading: HeadingForms, ordinal: usize| {
        format!("](<{}>)", linear.link(&sandbox, at, to, heading, ordinal))
    };
    let target = direct["T"].content(&sandbox);
    assert!(
        target.contains(&angled(&direct["T"], None, HEADINGS[4], 1)),
        "the same-record link is finished inside its angle brackets: {target}"
    );
    let project = direct["P"].content(&sandbox);
    assert!(
        project.contains(&angled(&direct["P"], Some(&direct["T"]), HEADINGS[0], 0)),
        "{project}"
    );
    let design = direct["D"].content(&sandbox);
    assert!(
        design.contains(&angled(&direct["D"], None, OVERVIEW, 0)),
        "{design}"
    );

    // The same project reaching that workspace a second way, through the board: every link of
    // that hop names the records that hop writes, never the first hop's copies, which the
    // board's records name only through the origin they share with them.
    let (on_board, _) = copy_plan(&sandbox, &authored, "board");
    holds_the_form(
        &sandbox,
        "the board",
        &board,
        &on_board,
        &authored_target_of(&sandbox),
    );
    let (onward, _) = copy_plan(&sandbox, &on_board, "linear");
    assert_ne!(
        onward["T"].id, direct["T"].id,
        "the board's copy is a record of its own"
    );
    for key in ["P", "U", "D"] {
        let content = onward[key].content(&sandbox);
        assert!(
            content.contains(&onward["T"].location(&sandbox))
                && !content.contains(&direct["T"].location(&sandbox)),
            "{key} names the target this hop wrote: {content}"
        );
    }
}

/// The absolute path of the authored target task of the folder `authored`.
fn authored_target_of(sandbox: &Sandbox) -> String {
    root_of(sandbox, "authored")
        .join("tasks/T.md")
        .display()
        .to_string()
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_refused_section_link_update_takes_the_whole_copy_back() {
    // The target's same-record link is finished in an update after its create, and Linear
    // refuses that update: the copy fails, saying so, and the workspace holds nothing it wrote.
    let sandbox = Sandbox::new();
    let (authored_config, _) = authored(&sandbox, "plans");
    let config = json!({"plugin": "linear", "config": crate::fixtures::linear_empty_workspace_failing(
        &sandbox,
        &[onetaskgraph_linear::graphql::ISSUE_REWRITE],
    )});
    configure(
        &sandbox,
        &[("plans", &authored_config), ("linear", &config)],
    );
    let output = sandbox
        .command()
        .args(["project", "copy", "plans:P", "--to", "linear"])
        .output()
        .expect("the binary runs");
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let listed: Value = serde_json::from_str(&ok(
        &sandbox,
        &["task", "list", "--source", "linear", "--json"],
    ))
    .expect("a listing");
    assert_eq!(
        listed["items"],
        json!([]),
        "every task the copy created is taken back: {}",
        stderr(&output)
    );
    let projects: Value = serde_json::from_str(&ok(
        &sandbox,
        &["project", "list", "--source", "linear", "--json"],
    ))
    .expect("a listing");
    assert_eq!(projects["items"], json!([]), "and the project");
}
