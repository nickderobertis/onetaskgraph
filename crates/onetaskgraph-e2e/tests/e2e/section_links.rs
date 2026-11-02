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

/// The repository every issue of the loopback board lives in.
const REPOSITORY: &str = "nickderobertis/onetaskgraph";

/// The headings of the target task, in order: their text, GitHub's (and the authored form's)
/// slug, and Linear's.
const HEADINGS: [(&str, &str, &str); 5] = [
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
const OVERVIEW: (&str, &str, &str) = ("Overview", "overview", "overview");

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

/// Write the authored project into a folder named `name` of the sandbox, answering its root.
fn authored(sandbox: &Sandbox, name: &str) -> PathBuf {
    let root = sandbox.subdirectory(name);
    for folder in ["projects", "tasks", "documents"] {
        std::fs::create_dir_all(root.join(folder)).expect("a folder");
    }
    let root = std::fs::canonicalize(&root).expect("the root");
    let target = root.join("tasks/T.md");
    let target = target.to_str().expect("a UTF-8 path");
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
            referrer_body(target),
        ),
        (
            "documents/D.md",
            "title: Design\nproject: P",
            design_body(target),
        ),
    ];
    for (path, front, body) in files {
        std::fs::write(root.join(path), format!("---\n{front}\n---\n{body}")).expect("a record");
    }
    root
}

/// One destination kind a journey copies into, and what reads it back.
pub(crate) enum Destination {
    /// A folder of Markdown, at its root.
    Local(PathBuf),
    /// The loopback board.
    Github(GitHubBoardFields),
    /// The loopback Linear workspace.
    Linear(LinearWorkspace),
}

impl Destination {
    /// A destination of `kind` in `sandbox`, and the source configuration naming it.
    pub(crate) fn new(sandbox: &Sandbox, kind: &str, folder: &str) -> (Self, Value) {
        match kind {
            "local-md" => {
                let config = empty_folder(sandbox, folder);
                let root = std::fs::canonicalize(sandbox.subdirectory(folder)).expect("a root");
                (
                    Self::Local(root),
                    json!({"plugin": "local-md", "config": config}),
                )
            }
            "github-projects" => {
                let (config, board) = github_projects_with_board(sandbox);
                board.use_canonical_issue_urls();
                sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
                (
                    Self::Github(board),
                    json!({"plugin": "github-projects", "config": config}),
                )
            }
            _ => {
                let (config, workspace) = linear_empty_workspace_held(sandbox);
                (
                    Self::Linear(workspace),
                    json!({"plugin": "linear", "config": config}),
                )
            }
        }
    }

    /// What this destination has been sent so far: every request a hosted one served, or every
    /// file of a folder and its bytes.
    fn mark(&self) -> Mark {
        match self {
            Self::Local(root) => Mark::Files(files_under(root)),
            Self::Github(board) => Mark::Served(board.served_count()),
            Self::Linear(workspace) => Mark::Served(workspace.served_count()),
        }
    }

    /// Every write this destination was sent since `mark`: each mutation a hosted one served,
    /// or each file of a folder created, changed or removed.
    fn writes_since(&self, mark: &Mark) -> Vec<String> {
        match (self, mark) {
            (Self::Local(root), Mark::Files(before)) => {
                let after = files_under(root);
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
            _ => unreachable!("a mark of this destination"),
        }
    }

    /// The link a record landed at `from` writes to `heading` of the record landed at `to` —
    /// `None` for its own — by this destination kind's contract.
    fn link(
        &self,
        sandbox: &Sandbox,
        from: &Landed,
        to: Option<&Landed>,
        heading: (&str, &str, &str),
        ordinal: usize,
    ) -> String {
        let (text, github, linear) = heading;
        match self {
            Self::Local(_) => {
                let slug = github;
                match to {
                    None => format!("#{slug}"),
                    Some(to) => format!(
                        "{}#{slug}",
                        relative(&from.location(sandbox), &to.location(sandbox))
                    ),
                }
            }
            Self::Github(_) => match to {
                None => format!("#user-content-{github}"),
                Some(to) => format!("{}#user-content-{github}", www(&to.location(sandbox))),
            },
            Self::Linear(workspace) => {
                let at = to.unwrap_or(from);
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
                    .unwrap_or_else(|| {
                        panic!("{} stores heading {text} #{ordinal}: {ids:?}", at.id)
                    });
                format!("{}#heading-{linear}-{}", at.location(sandbox), &id[..8])
            }
        }
    }

    /// The record's body as this destination itself stores it, the metadata slot left out.
    fn raw(&self, landed: &Landed) -> String {
        let raw = match self {
            Self::Local(_) => return String::new(),
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

/// What a destination held before a command.
enum Mark {
    /// Each file under a folder, and its bytes.
    Files(BTreeMap<String, Vec<u8>>),
    /// How many requests a hosted destination had served.
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
    assert!(
        rest.starts_with(&format!("{REPOSITORY}/issues/")),
        "{location}"
    );
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

/// One record a copy landed.
pub(crate) struct Landed {
    /// `task`, `project` or `document`.
    verb: &'static str,
    /// Its qualified id at the destination.
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

    /// Where its destination says it is.
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

/// Copy the project `P` of `from`, its tasks, and then its document `D`, into `to`, answering
/// where each landed and the document copy's outcomes.
pub(crate) fn copy_plan(
    sandbox: &Sandbox,
    from: &str,
    to: &str,
) -> (BTreeMap<&'static str, Landed>, BTreeMap<String, Value>) {
    let project = outcomes(&ok(
        sandbox,
        &[
            "project",
            "copy",
            &format!("{from}:P"),
            "--to",
            to,
            "--json",
        ],
    ));
    let documents = outcomes(&ok(
        sandbox,
        &[
            "document",
            "copy",
            &format!("{from}:D"),
            "--to",
            to,
            "--json",
        ],
    ));
    let at = |held: &BTreeMap<String, Value>, id: &str| {
        held[&format!("{from}:{id}")]["destination"]
            .as_str()
            .unwrap_or_else(|| panic!("{from}:{id} landed: {held:#?}"))
            .to_owned()
    };
    let mut landed = BTreeMap::new();
    for (key, verb) in [("P", "project"), ("T", "task"), ("U", "task")] {
        landed.insert(
            key,
            Landed {
                verb,
                id: at(&project, key),
            },
        );
    }
    landed.insert(
        "D",
        Landed {
            verb: "document",
            id: at(&documents, "D"),
        },
    );
    (landed, documents)
}

/// What every authored body reads as once copied into `destination`: the authored text with
/// each section link — and nothing else — written in that destination's form.
fn expected_bodies(
    sandbox: &Sandbox,
    destination: &Destination,
    landed: &BTreeMap<&'static str, Landed>,
    authored_target: &str,
) -> BTreeMap<&'static str, String> {
    let link = |from: &str, to: Option<&str>, index: usize| {
        let heading = HEADINGS[index];
        let ordinal = usize::from(index == 4);
        destination.link(
            sandbox,
            &landed[from],
            to.map(|to| &landed[to]),
            heading,
            ordinal,
        )
    };
    let own = |from: &str, heading| destination.link(sandbox, &landed[from], None, heading, 0);
    let target_there = landed["T"].location(sandbox);
    let target_link = match destination {
        // A link to a heading the target does not hold keeps its fragment as it is, on the
        // link to the target itself — relative, as it was spelled, into a folder of Markdown.
        Destination::Local(_) => relative(&landed["D"].location(sandbox), &target_there),
        _ => target_there.clone(),
    };
    let mut bodies = BTreeMap::new();
    bodies.insert(
        "T",
        target_body().replace("](#budgets-1)", &format!("]({})", link("T", None, 4))),
    );
    bodies.insert(
        "U",
        referrer_body(authored_target)
            .replace(
                "](T.md#budget-demo-quota-seconds)",
                &format!("]({})", link("U", Some("T"), 2)),
            )
            .replace(
                &format!("]({authored_target}#budgets)"),
                &format!("]({})", link("U", Some("T"), 1)),
            ),
    );
    bodies.insert(
        "P",
        project_body()
            .replace(
                "](../tasks/T.md#what)",
                &format!("]({})", link("P", Some("T"), 0)),
            )
            .replace(
                "](../tasks/T.md#step-2b-retry-once)",
                &format!("]({})", link("P", Some("T"), 3)),
            ),
    );
    bodies.insert(
        "D",
        design_body(authored_target)
            .replace(
                "](../tasks/T.md#budgets)",
                &format!("]({})", link("D", Some("T"), 1)),
            )
            .replace(
                "](../tasks/T.md#budgets-1)",
                &format!("]({})", link("D", Some("T"), 4)),
            )
            .replace(
                &format!("]({authored_target}#step-2b-retry-once)"),
                &format!("]({})", link("D", Some("T"), 3)),
            )
            .replace("](#overview)", &format!("]({})", own("D", OVERVIEW)))
            .replace(
                "](../tasks/T.md#nowhere)",
                &format!("]({target_link}#nowhere)"),
            ),
    );
    bodies
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback stand-ins this suite already drives for both hosted destinations. AGENTS.md puts a journey crossing more than one plugin in this suite.
#[test]
fn authored_section_links_land_in_each_destinations_form_and_a_repeat_copy_writes_nothing() {
    for kind in ["local-md", "github-projects", "linear"] {
        let sandbox = Sandbox::new();
        let source = authored(&sandbox, "plans");
        let (destination, config) = Destination::new(&sandbox, kind, "dest");
        sandbox.project_document(&document(&json!({
            "plans": {"plugin": "local-md", "config": empty_folder(&sandbox, "plans")},
            "dest": config,
        })));
        let authored_target = source.join("tasks/T.md").display().to_string();

        let (landed, documents) = copy_plan(&sandbox, "plans", "dest");
        let expected = expected_bodies(&sandbox, &destination, &landed, &authored_target);
        for (key, body) in &expected {
            assert_eq!(
                landed[key].content(&sandbox).lines().collect::<Vec<_>>(),
                body.lines().collect::<Vec<_>>(),
                "{kind}: {key} reads back as authored, its section links in the destination's \
                 form"
            );
        }
        // The fragment naming no heading of the target is carried and counted, not guessed.
        assert_eq!(
            documents["plans:D"]["section_links_unresolved"],
            json!(1),
            "{kind}: {:#}",
            documents["plans:D"]
        );

        match &destination {
            Destination::Github(_) => {
                // The issue page gives a heading no id: the body carries one before each.
                let raw = destination.raw(&landed["T"]);
                for (text, slug) in [
                    ("## What", "what"),
                    ("## Budgets\n\nFirst", "budgets"),
                    ("### Budget demo-quota-seconds", "budget-demo-quota-seconds"),
                    ("### Step 2.b: retry (once)", "step-2b-retry-once"),
                    ("## Budgets\n\nSecond", "budgets-1"),
                ] {
                    assert!(
                        raw.contains(&format!("<a id=\"{slug}\"></a>\n{text}")),
                        "the anchor before {text}: {raw}"
                    );
                }
                assert!(
                    destination
                        .raw(&landed["D"])
                        .contains("<a id=\"overview\"></a>\n## Overview")
                );
            }
            Destination::Linear(_) => {
                for key in ["P", "T", "U", "D"] {
                    let raw = destination.raw(&landed[key]);
                    assert!(!raw.contains("<a "), "{key} holds no anchor markup: {raw}");
                }
                // The two `## Budgets` are told apart by the ids Linear stored them under.
                let first =
                    destination.link(&sandbox, &landed["D"], Some(&landed["T"]), HEADINGS[1], 0);
                let second =
                    destination.link(&sandbox, &landed["D"], Some(&landed["T"]), HEADINGS[4], 1);
                assert_ne!(first, second);
            }
            Destination::Local(_) => {}
        }

        // Nothing changed at the source, so nothing is written at the destination.
        let mark = destination.mark();
        let (again, _) = copy_plan(&sandbox, "plans", "dest");
        assert_eq!(
            destination.writes_since(&mark),
            Vec::<String>::new(),
            "{kind}: a repeat copy of the unchanged project writes nothing"
        );
        for key in ["P", "T", "U", "D"] {
            assert_eq!(again[key].id, landed[key].id, "{kind}: {key} landed once");
        }
    }
}
