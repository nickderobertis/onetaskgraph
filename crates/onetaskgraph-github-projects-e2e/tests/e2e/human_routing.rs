//! Where a human task projection lands: the `purpose: human-task` routes of the source a request
//! names, under the store's public boundary screened by the released onevcs commands — public
//! work to the public board, private and mixed work to the private one, and a private item with
//! nowhere private to go refused rather than written publicly.

use std::path::PathBuf;

use onetaskgraph_e2e_support::common::{Sandbox, stderr, stdout};
use onetaskgraph_e2e_support::fixtures::{GitHubBoardFields, document, github_projects_with_items};
use onetaskgraph_e2e_support::onevcs::{BoundaryCommand, BoundaryRecord, OnevcsHome};
use serde_json::{Value, json};

use super::human_items::{HOST, PROJECT, PUBLIC_REPOSITORY, native, project_item, projected};

/// The private repository private work names, and the private board creates its issues in.
const PRIVATE_REPOSITORY: &str = "synthco/private-widgets";

/// The project the private board files planless items under.
const PRIVATE_PROJECT: &str = "PROJECT-9";

/// A plan's project the public board already holds.
const PLAN_PROJECT: &str = "PROJECT-7";

/// A private title, the term no public request may carry.
const PRIVATE_TITLE: &str = "Rotate the private-widgets signing key";

/// A store of two loopback boards — `human-public` and `human-private` — whose writes are screened
/// by the released onevcs boundary commands against a home registering the public and the
/// private repository.
struct Routed {
    sandbox: Sandbox,
    public: GitHubBoardFields,
    private: GitHubBoardFields,
    home: OnevcsHome,
    record: BoundaryRecord,
    intents: PathBuf,
    public_config: Value,
    private_config: Value,
}

impl Routed {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (mut public_config, public) = github_projects_with_items(
            &sandbox,
            vec![
                project_item(PROJECT, PUBLIC_REPOSITORY),
                project_item(PLAN_PROJECT, PUBLIC_REPOSITORY),
            ],
        );
        public_config["repository"] = json!(PUBLIC_REPOSITORY);
        public.use_repository(PUBLIC_REPOSITORY);
        public.use_canonical_issue_urls();
        public.file_new_items_without_status(true);
        let (mut private_config, private) = github_projects_with_items(
            &sandbox,
            vec![project_item(PRIVATE_PROJECT, PRIVATE_REPOSITORY)],
        );
        private_config["repository"] = json!(PRIVATE_REPOSITORY);
        private.use_repository(PRIVATE_REPOSITORY);
        private.use_canonical_issue_urls();
        private.file_new_items_without_status(true);
        let (owner, name) = PUBLIC_REPOSITORY.split_once('/').unwrap();
        let mut home = OnevcsHome::new(&sandbox);
        home.public(owner, name);
        let (owner, name) = PRIVATE_REPOSITORY.split_once('/').unwrap();
        home.private(owner, name);
        let record = home.recorded(&sandbox);
        let intents = sandbox.subdirectory("intents");
        let routed = Self {
            sandbox,
            public,
            private,
            home,
            record,
            intents,
            public_config,
            private_config,
        };
        routed.configure(true);
        routed
    }

    /// Write the configuration, with the public source's private human-task route when
    /// `private_route` is — and, either way, a legacy route with no purpose sending the public
    /// repository's work to the private board, which a human task never takes.
    fn configure(&self, private_route: bool) {
        let mut routes = vec![
            json!({"repositories": [format!("github.com/{PUBLIC_REPOSITORY}")],
                                     "to": "human-private"}),
        ];
        if private_route {
            routes.push(json!({"classification": "private", "purpose": "human-task",
                               "to": "human-private"}));
        }
        let projection =
            |project: &str| json!({"host": HOST, "directory": self.intents, "project": project});
        let sources = json!({
            "human-public": {"plugin": "github-projects", "config": self.public_config,
                             "visibility": "public", "routes": routes,
                             "cache": {"dir": self.sandbox.subdirectory("cache-public")},
                             "task_projection": projection(PROJECT)},
            "human-private": {"plugin": "github-projects", "config": self.private_config,
                              "visibility": "private",
                              "cache": {"dir": self.sandbox.subdirectory("cache-private")},
                              "task_projection": projection(PRIVATE_PROJECT)}
        });
        let mut configured: Value = serde_json::from_str(&document(&sources)).unwrap();
        configured["write_policy"] = self.record.write_policy().clone();
        self.sandbox.project_document(&configured.to_string());
    }

    /// Project `request` through the binary.
    fn project(&self, request: &Value) -> std::process::Output {
        let path = self.sandbox.project().join(format!(
            "request-{}.json",
            request["item_id"].as_str().unwrap()
        ));
        std::fs::write(&path, request.to_string()).unwrap();
        self.sandbox
            .command()
            .args(["task", "project-human", "--input"])
            .arg(&path)
            .arg("--json")
            .env("ONETASKGRAPH_HOST", HOST)
            .env("ONEVCS_HOME", self.home.path())
            .output()
            .unwrap()
    }

    /// Every document `board` has been sent, joined with its variables, as one text.
    fn sent_to(board: &GitHubBoardFields) -> String {
        board
            .served()
            .iter()
            .map(|(query, variables)| format!("{query}\n{variables}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A request from `human-public` for `item_id` naming `repositories`, titled `title`.
fn routed(item_id: &str, title: &str, repositories: &[&str]) -> Value {
    json!({"schema_version": 1, "item_id": item_id,
        "owner": {"session": "session-1", "run": "run-1", "plan": null},
        "title": title, "body": "Decide before the release.", "priority": "urgent",
        "repositories": repositories.iter().map(|r| format!("github.com/{r}")).collect::<Vec<_>>(),
        "project": null, "source": "human-public", "state": "open", "reason": null})
}

#[test]
fn public_work_lands_on_the_public_board_and_private_or_mixed_work_only_on_the_private_one() {
    let store = Routed::new();

    let screened = store.record.calls().len();
    let public = projected(&store.project(&routed(
        "public-item",
        "Choose the next public example",
        &[PUBLIC_REPOSITORY],
    )));
    assert_eq!(public["source"], "human-public");
    assert_eq!(public["project"], PROJECT);
    assert!(
        public["url"]
            .as_str()
            .unwrap()
            .starts_with(&format!("https://github.com/{PUBLIC_REPOSITORY}/issues/")),
        "{public}"
    );
    assert_eq!(
        store.public.priority(&native(&public)).as_deref(),
        Some("Urgent")
    );
    assert!(
        store.record.calls()[screened..]
            .iter()
            .any(|call| call.command == BoundaryCommand::Check),
        "the public write was screened"
    );

    let public_before = store.public.served().len();
    for (item, repositories) in [
        ("private-item", vec![PRIVATE_REPOSITORY]),
        ("mixed-item", vec![PUBLIC_REPOSITORY, PRIVATE_REPOSITORY]),
    ] {
        let answer = projected(&store.project(&routed(item, PRIVATE_TITLE, &repositories)));
        assert_eq!(answer["source"], "human-private", "{item}");
        assert_eq!(answer["project"], PRIVATE_PROJECT, "{item}");
        let id = native(&answer);
        assert!(
            store.private.body(&id).as_str().is_some(),
            "{item} is on the private board"
        );
        assert_eq!(
            store.private.repository_of(&id).as_deref(),
            Some(PRIVATE_REPOSITORY)
        );
    }
    assert_eq!(
        store.public.served().len(),
        public_before,
        "private and mixed work sent the public board nothing"
    );
    assert!(!Routed::sent_to(&store.public).contains(PRIVATE_TITLE));
}

#[test]
fn private_work_with_no_private_route_is_refused_and_nothing_reaches_either_board() {
    let store = Routed::new();
    store.configure(false);
    for (item, repositories) in [
        ("private-unrouted", vec![PRIVATE_REPOSITORY]),
        (
            "mixed-unrouted",
            vec![PUBLIC_REPOSITORY, PRIVATE_REPOSITORY],
        ),
    ] {
        let refused = store.project(&routed(item, PRIVATE_TITLE, &repositories));
        assert_ne!(refused.status.code(), Some(0), "{item}");
        let failure: Value = serde_json::from_str(&stdout(&refused)).unwrap();
        assert_eq!(
            failure["failure"]["kind"], "not-private-destination",
            "{item}"
        );
        assert!(
            stderr(&refused).contains("human-public"),
            "{}",
            stderr(&refused)
        );
    }
    assert!(store.public.served().is_empty());
    assert!(store.private.served().is_empty());
}

#[test]
fn a_project_of_another_source_is_refused_and_a_plan_project_is_used_as_given() {
    let store = Routed::new();
    let mut elsewhere = routed(
        "misplaced-item",
        "Choose the next public example",
        &[PUBLIC_REPOSITORY],
    );
    elsewhere["project"] = json!(format!("human-private:{PRIVATE_PROJECT}"));
    let refused = store.project(&elsewhere);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("routes to human-public"),
        "{}",
        stderr(&refused)
    );
    assert!(store.public.served().is_empty());
    assert!(store.private.served().is_empty());

    let mut planned = routed(
        "planned-item",
        "Review the plan's next step",
        &[PUBLIC_REPOSITORY],
    );
    planned["owner"]["plan"] = json!("plan-7");
    planned["project"] = json!(format!("human-public:{PLAN_PROJECT}"));
    let answer = projected(&store.project(&planned));
    assert_eq!(answer["project"], PLAN_PROJECT);
    let id = native(&answer);
    let issue = store
        .public
        .issues()
        .into_iter()
        .find(|issue| issue["id"] == json!(id))
        .unwrap();
    assert_eq!(
        issue["parent"], PLAN_PROJECT,
        "filed under the plan's project"
    );
}

#[test]
fn a_legacy_route_still_routes_other_writers_and_never_a_human_task() {
    let store = Routed::new();
    let routed_legacy = store
        .sandbox
        .command()
        .args(["sources", "route", "human-public", "--repository"])
        .arg(format!("github.com/{PUBLIC_REPOSITORY}"))
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(
        routed_legacy.status.code(),
        Some(0),
        "{}",
        stderr(&routed_legacy)
    );
    let answer: Value = serde_json::from_str(&stdout(&routed_legacy)).unwrap();
    assert_eq!(answer["destination"], "human-private", "{answer}");
    assert_eq!(
        answer["reachable"],
        json!(["human-public", "human-private"])
    );

    // The same repository's human task takes no entry without its purpose: public, it stays on
    // the public board it was sent to.
    let answer = projected(&store.project(&routed(
        "legacy-item",
        "Choose the next public example",
        &[PUBLIC_REPOSITORY],
    )));
    assert_eq!(answer["source"], "human-public");
    assert!(store.private.served().is_empty());
}
