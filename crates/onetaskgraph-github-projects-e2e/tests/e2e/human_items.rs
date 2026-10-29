//! Human task projection through the binary: `task project-human --input FILE --json` against
//! loopback GitHub boards — the lifecycle of one item, its retries, its crashes, its routing and
//! its refusals, each observed as the tickets the boards hold and the requests they answered.

use std::path::PathBuf;

use onetaskgraph_e2e_support::common::{Sandbox, stderr, stdout};
use onetaskgraph_e2e_support::fixtures::{GitHubBoardFields, document, github_projects_with_items};
use serde_json::{Value, json};

/// The host every journey's configuration names as the authority over its projections.
pub(super) const HOST: &str = "projection-host";

/// The repository the boards create their issues in.
pub(super) const PUBLIC_REPOSITORY: &str = "synthco/public-widgets";

/// The project issue every planless item is filed under.
pub(super) const PROJECT: &str = "PROJECT-1";

/// The project issue a board holds for planless human tasks.
pub(super) fn project_item(id: &str, repository: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":"Human tasks",
        "body":"Work waiting on a person.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->",
        "state":"OPEN","reason":null,"parent":null,"repo":repository,"status":"Todo",
        "origin":"","labels":[]})
}

/// One loopback board configured as a source, with the request that reaches it.
pub(super) struct Board {
    pub fields: GitHubBoardFields,
    pub config: Value,
}

/// A board holding `items` beside the human-task project, creating its issues in
/// [`PUBLIC_REPOSITORY`] and answering canonical issue URLs.
pub(super) fn board(sandbox: &Sandbox, mut items: Vec<Value>) -> Board {
    items.push(project_item(PROJECT, PUBLIC_REPOSITORY));
    let (mut config, fields) = github_projects_with_items(sandbox, items);
    config["repository"] = json!(PUBLIC_REPOSITORY);
    config["priority_mapping"] = json!({"urgent": "Urgent"});
    fields.use_repository(PUBLIC_REPOSITORY);
    fields.use_canonical_issue_urls();
    fields.file_new_items_without_status(true);
    Board { fields, config }
}

/// A store holding one public board, `human`, whose projections this host is authoritative
/// over.
pub(super) struct Store {
    pub sandbox: Sandbox,
    pub board: Board,
    pub intents: PathBuf,
}

impl Store {
    pub fn new(items: Vec<Value>) -> Self {
        let sandbox = Sandbox::new();
        let board = board(&sandbox, items);
        let intents = sandbox.subdirectory("intents");
        let store = Self {
            sandbox,
            board,
            intents,
        };
        store.configure(json!({}));
        store
    }

    /// Write the configuration, `extra` sources beside `human`.
    pub fn configure(&self, extra: Value) {
        let mut sources = json!({"human": {
            "plugin": "github-projects",
            "config": self.board.config,
            "task_projection": {"host": HOST, "directory": self.intents, "project": PROJECT}
        }});
        for (name, source) in extra.as_object().unwrap() {
            sources[name] = source.clone();
        }
        self.sandbox.project_document(&document(&sources));
    }

    /// Write `request` to a file and project it from this host.
    pub fn project(&self, request: &Value) -> std::process::Output {
        self.project_from(request, HOST)
    }

    /// Write `request` to a file and project it from the host `host`.
    pub fn project_from(&self, request: &Value, host: &str) -> std::process::Output {
        let path = self.sandbox.project().join(format!(
            "request-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, request.to_string()).unwrap();
        self.sandbox
            .command()
            .args(["task", "project-human", "--input"])
            .arg(&path)
            .arg("--json")
            .env("ONETASKGRAPH_HOST", host)
            .output()
            .unwrap()
    }

    /// The requests the board has answered so far.
    pub fn requests(&self) -> usize {
        self.board.fields.served().len()
    }

    /// The operations the board answered from request `from` on.
    pub fn operations_since(&self, from: usize) -> Vec<&'static str> {
        self.board.fields.served()[from..]
            .iter()
            .map(|(query, _)| super::plan_copy::operation_name(query))
            .collect()
    }

    /// Every issue on the board whose body carries `item_id` as its human item.
    pub fn tickets(&self, item_id: &str) -> Vec<String> {
        let needle = format!("\"human.item_id\":\"{item_id}\"");
        self.board
            .fields
            .issues()
            .into_iter()
            .filter(|issue| {
                issue["body"]
                    .as_str()
                    .is_some_and(|body| body.contains(&needle))
            })
            .map(|issue| issue["id"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// A v1 request for `item_id`, open, with `repositories`.
pub(super) fn request(item_id: &str, repositories: &[&str]) -> Value {
    json!({"schema_version": 1, "item_id": item_id,
        "owner": {"session": "session-1", "run": null, "plan": null},
        "title": format!("Decide on {item_id}"), "body": "Choose the next public example.",
        "priority": "high",
        "repositories": repositories.iter().map(|r| format!("github.com/{r}")).collect::<Vec<_>>(),
        "project": null, "source": "human", "state": "open", "reason": null})
}

/// The answer a successful projection printed.
pub(super) fn projected(output: &std::process::Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).expect("a HumanTaskProjected document")
}

/// The native id of the answer's task.
pub(super) fn native(answer: &Value) -> String {
    answer["task"]
        .as_str()
        .unwrap()
        .split_once(':')
        .unwrap()
        .1
        .to_owned()
}

/// The metadata slot of one issue's body.
pub(super) fn slot(fields: &GitHubBoardFields, id: &str) -> Value {
    let body = fields.body(id);
    let body = body.as_str().expect("the board holds the issue");
    let (_, inner) = body
        .split_once("<!-- onetaskgraph.metadata")
        .expect("the body carries its metadata slot");
    let (inner, _) = inner.split_once("-->").expect("the slot is closed");
    serde_json::from_str(inner.trim()).expect("the slot is JSON")
}

#[test]
fn an_item_is_created_once_bound_and_settled_once_and_a_conflicting_terminal_is_refused() {
    let store = Store::new(Vec::new());
    let open = request("item-1", &[PUBLIC_REPOSITORY]);

    let before = store.requests();
    let answer = projected(&store.project(&open));
    let created = native(&answer);
    assert_eq!(
        answer.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["project", "source", "task", "url"]
    );
    assert_eq!(answer["source"], "human");
    assert_eq!(answer["project"], PROJECT);
    assert_eq!(
        answer["url"],
        format!(
            "https://github.com/{PUBLIC_REPOSITORY}/issues/{}",
            onetaskgraph_e2e_support::fixtures::github_number(&created)
        )
    );
    assert_eq!(store.tickets("item-1"), std::slice::from_ref(&created));
    let held = slot(&store.board.fields, &created);
    assert_eq!(held["human.item_id"], "item-1");
    assert_eq!(held["human.state"], "open");
    assert!(
        held["onetaskgraph.origin"]
            .as_str()
            .unwrap()
            .starts_with(&format!("human-task:{HOST}:"))
    );
    assert_eq!(
        store.board.fields.priority(&created).as_deref(),
        Some("High")
    );
    assert_eq!(store.board.fields.status(&created).as_deref(), Some("Todo"));
    let created_with = store.operations_since(before);
    assert!(
        !created_with
            .iter()
            .any(|operation| operation.contains("comment")),
        "{created_with:?}"
    );

    // A retry of a bound item answers the same ticket and asks the board nothing.
    let before = store.requests();
    assert_eq!(projected(&store.project(&open)), answer);
    assert_eq!(store.requests(), before, "a bound retry sent requests");

    // Dismissed with no reason: closed as not planned, the reason left absent.
    let mut dismissed = open.clone();
    dismissed["state"] = json!("dismissed");
    let before = store.requests();
    assert_eq!(projected(&store.project(&dismissed)), answer);
    let settled_with = store.operations_since(before);
    assert_eq!(
        store.board.fields.state(&created),
        ("CLOSED".to_owned(), Some("NOT_PLANNED".to_owned()))
    );
    let held = slot(&store.board.fields, &created);
    assert_eq!(held["human.state"], "dismissed");
    assert!(held.get("human.reason").is_none(), "{held}");
    assert!(
        !settled_with
            .iter()
            .any(|operation| operation.starts_with("reading")),
        "a terminal write after the create read the issue again: {settled_with:?}"
    );

    // The same terminal again is a no-op; a different one is refused, writing nothing.
    let before = store.requests();
    assert_eq!(projected(&store.project(&dismissed)), answer);
    assert_eq!(store.requests(), before);
    let mut completed = open.clone();
    completed["state"] = json!("completed");
    let refused = store.project(&completed);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("conflicting terminal"),
        "{}",
        stderr(&refused)
    );
    let failure: Value = serde_json::from_str(&stdout(&refused)).unwrap();
    assert_eq!(failure["failure"]["kind"], "projection-refused");
    assert_eq!(store.requests(), before);
    let mut reopened = open.clone();
    reopened["state"] = json!("open");
    assert_ne!(store.project(&reopened).status.code(), Some(0));
    assert_eq!(store.board.fields.state(&created).0, "CLOSED");
    assert_eq!(store.tickets("item-1"), [created]);
}
