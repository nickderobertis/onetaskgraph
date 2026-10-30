//! A human task routed across plugins by its source's `purpose: human-task` routes: an
//! organization's work sent from a GitHub board to that organization's Linear workspace, into
//! the plan's existing project there, and nothing of it written to the board.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    document, github_projects_with_items, linear_default_mapping, linear_workspace,
};

const HOST: &str = "routing-host";

#[test]
fn an_organizations_human_task_routes_from_a_board_to_its_plan_project_in_linear() {
    let sandbox = Sandbox::new();
    let (mut board_config, board) = github_projects_with_items(
        &sandbox,
        vec![
            json!({"item":"ITEM-PROJECT-1","id":"PROJECT-1","type":"Issue","title":"Human tasks",
            "body":"Waiting on a person.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->",
            "state":"OPEN","reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
            "status":"Todo","origin":"","labels":[]}),
        ],
    );
    board_config["priority_mapping"] = json!({});
    let mut data = crate::fixtures::dataset();
    data["task_dependencies"] = json!([]);
    let (mut linear_config, workspace) = linear_workspace(&sandbox, data);
    linear_config["status_mapping"] = linear_default_mapping();
    let intents = sandbox.subdirectory("intents");
    let projection =
        |project: &str| json!({"host": HOST, "directory": intents, "project": project});
    let configure = |route: Value| {
        sandbox.project_document(&document(&json!({
            "human": {"plugin": "github-projects", "config": board_config, "routes": [route],
                      "cache": {"dir": sandbox.subdirectory("cache-human")},
                      "task_projection": projection("PROJECT-1")},
            "org": {"plugin": "linear", "config": linear_config,
                    "cache": {"dir": sandbox.subdirectory("cache-org")},
                    "task_projection": projection("P-1")}
        })));
    };
    configure(
        json!({"repositories": ["github.com/orgco/*"], "purpose": "human-task",
                     "to": "org"}),
    );
    let request = json!({"schema_version": 1, "item_id": "org-review",
        "owner": {"session": "session-1", "run": null, "plan": "plan-1"},
        "title": "Approve the organization's release", "body": "Sign off on the release.",
        "priority": "high", "repositories": ["github.com/orgco/service"],
        "project": "org:P-1", "source": "human", "state": "open", "reason": null});
    let path = sandbox.project().join("request.json");
    std::fs::write(&path, request.to_string()).unwrap();
    let board_before = board.served().len();
    let output = sandbox
        .command()
        .args(["task", "project-human", "--input"])
        .arg(&path)
        .arg("--json")
        .env("ONETASKGRAPH_HOST", HOST)
        .env("GITHUB_PROJECTS_FIXTURE_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(answer["source"], "org");
    assert_eq!(answer["project"], "P-1");
    assert!(
        answer["url"]
            .as_str()
            .unwrap()
            .starts_with("https://linear.app/"),
        "{answer}"
    );
    assert!(
        workspace
            .served()
            .iter()
            .any(|(query, _)| query.contains("issueCreate")),
        "created in Linear"
    );
    assert_eq!(
        board.served().len(),
        board_before,
        "the board was sent nothing"
    );

    // Without the purpose the same entry is a legacy route the human task does not take: it
    // stays with the board it was sent to, which files a sub-issue only in a repository of its
    // project issue's owner and so refuses it — and Linear is sent no create.
    configure(json!({"repositories": ["github.com/orgco/*"], "to": "org"}));
    let mut stays = request;
    stays["item_id"] = json!("org-review-unscoped");
    stays["project"] = Value::Null;
    std::fs::write(&path, stays.to_string()).unwrap();
    let creates = |workspace: &crate::fixtures::LinearWorkspace| {
        workspace
            .served()
            .iter()
            .filter(|(query, _)| query.contains("issueCreate"))
            .count()
    };
    let linear_creates = creates(&workspace);
    let output = sandbox
        .command()
        .args(["task", "project-human", "--input"])
        .arg(&path)
        .arg("--json")
        .env("ONETASKGRAPH_HOST", HOST)
        .env("GITHUB_PROJECTS_FIXTURE_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(failure["failure"]["source"], "human", "{failure}");
    assert_eq!(
        creates(&workspace),
        linear_creates,
        "Linear was sent no create"
    );
}
