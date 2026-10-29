//! Human task projection through the binary: `task project-human --input FILE --json` against
//! loopback GitHub boards — the lifecycle of one item, its retries, its crashes, its routing and
//! its refusals, each observed as the tickets the boards hold and the requests they answered.

use std::path::PathBuf;

use onetaskgraph_e2e_support::common::{Sandbox, stderr, stdout};
use onetaskgraph_e2e_support::fixtures::{GitHubBoardFields, document, github_projects_with_items};
use onetaskgraph_github_projects::graphql;
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
            "cache": {"dir": self.sandbox.subdirectory("cache-human")},
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
    assert!(
        held.get("human.state").is_none(),
        "the state is the issue's own: {held}"
    );
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
    assert!(held.get("human.reason").is_none(), "{held}");
    assert_eq!(
        store.board.fields.status(&created).as_deref(),
        Some("Cancelled")
    );
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

impl Store {
    /// Write `request` where the binary reads it.
    fn request_file(&self, request: &Value) -> PathBuf {
        let path = self.sandbox.project().join(format!(
            "request-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, request.to_string()).unwrap();
        path
    }

    /// Project `request` and kill the binary as soon as the board has applied the first
    /// request `matches` holds of — before its answer reaches the binary: a crash at that
    /// point of the sequence.
    pub fn project_killed(
        &self,
        request: &Value,
        matches: impl Fn(&str, &Value) -> bool + Send + 'static,
    ) {
        let path = self.request_file(request);
        let mut command = self.sandbox.subprocess(onetaskgraph_e2e_support::binary());
        command
            .args(["task", "project-human", "--input"])
            .arg(&path)
            .arg("--json")
            .env("ONETASKGRAPH_HOST", HOST)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = std::sync::Arc::new(std::sync::Mutex::new(command.spawn().unwrap()));
        let killed = std::sync::Arc::clone(&child);
        self.board.fields.after_request(matches, 0, move |_| {
            let _ = killed.lock().unwrap().kill();
        });
        let status = loop {
            if let Some(status) = child.lock().unwrap().try_wait().unwrap() {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(
            !status.success(),
            "the binary finished before the crash point"
        );
    }
}

/// A request matching a document carrying `operation`.
fn carrying(operation: &'static str) -> impl Fn(&str, &Value) -> bool + Send + 'static {
    move |query: &str, _: &Value| query.contains(operation)
}

/// The operation names of the requests the board answered from `from` on.
fn creates_since(store: &Store, from: usize) -> usize {
    store
        .operations_since(from)
        .iter()
        .filter(|operation| **operation == "creating an issue")
        .count()
}

#[test]
fn a_dismissal_keeps_its_reason_and_a_lost_terminal_answer_is_retried_without_writing_again() {
    let store = Store::new(Vec::new());
    let open = request("item-reason", &[PUBLIC_REPOSITORY]);
    let created = native(&projected(&store.project(&open)));
    let mut dismissed = open.clone();
    dismissed["state"] = json!("dismissed");
    dismissed["reason"] = json!("Superseded by another example.");
    // The close lands and its answer never reaches the binary.
    store.project_killed(&dismissed, carrying("updateIssue(input"));
    assert_eq!(store.board.fields.state(&created).0, "CLOSED");

    // A fresh process reads the issue once, finds the terminal state there, and writes nothing.
    let before = store.requests();
    let answer = projected(&store.project(&dismissed));
    assert_eq!(native(&answer), created);
    assert_eq!(store.operations_since(before), ["reading one issue"]);
    let held = slot(&store.board.fields, &created);
    assert_eq!(held["human.reason"], "Superseded by another example.");
    assert_eq!(
        store.board.fields.state(&created),
        ("CLOSED".to_owned(), Some("NOT_PLANNED".to_owned()))
    );
    // Acknowledged now, so the next retry asks the board nothing.
    let before = store.requests();
    projected(&store.project(&dismissed));
    assert_eq!(store.requests(), before);

    // An omitted reason is the same request as a null one.
    let other = request("item-omitted", &[PUBLIC_REPOSITORY]);
    let mut omitted = other.clone();
    omitted.as_object_mut().unwrap().remove("reason");
    omitted["state"] = json!("completed");
    let completed = native(&projected(&store.project(&omitted)));
    let held = slot(&store.board.fields, &completed);
    assert!(held.get("human.reason").is_none(), "{held}");
    assert_eq!(
        store.board.fields.status(&completed).as_deref(),
        Some("Done")
    );
    assert_eq!(
        store.board.fields.state(&completed),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
    let mut nulled = other;
    nulled["state"] = json!("completed");
    let before = store.requests();
    assert_eq!(native(&projected(&store.project(&nulled))), completed);
    assert_eq!(store.requests(), before);
}

#[test]
fn a_create_whose_answer_was_lost_is_bound_by_its_one_verified_origin_match() {
    let store = Store::new(Vec::new());
    let open = request("item-lost", &[PUBLIC_REPOSITORY]);
    // Filed under its project — the last write of the create — and the answer lost.
    store.project_killed(&open, carrying("addSubIssue"));
    let tickets = store.tickets("item-lost");
    assert_eq!(tickets.len(), 1);

    let before = store.requests();
    let answer = projected(&store.project(&open));
    assert_eq!(native(&answer), tickets[0]);
    assert_eq!(
        answer["url"],
        format!(
            "https://github.com/{PUBLIC_REPOSITORY}/issues/{}",
            onetaskgraph_e2e_support::fixtures::github_number(&tickets[0])
        )
    );
    assert_eq!(creates_since(&store, before), 0);
    assert_eq!(
        store.operations_since(before),
        [super::plan_copy::operation_name(graphql::ORIGIN_LOOKUP)],
        "recovery is one origin read"
    );
    // Bound: the same answer again costs nothing, and resolving it writes the one ticket.
    let before = store.requests();
    assert_eq!(projected(&store.project(&open)), answer);
    assert_eq!(store.requests(), before);
    let mut completed = open;
    completed["state"] = json!("completed");
    assert_eq!(projected(&store.project(&completed)), answer);
    assert_eq!(store.board.fields.state(&tickets[0]).0, "CLOSED");
    assert_eq!(store.tickets("item-lost"), tickets);
}

#[test]
fn a_pending_create_with_no_verifiable_match_is_refused_and_never_sent_again() {
    // Crashed before the create reached the board: nothing carries the origin.
    let store = Store::new(Vec::new());
    let open = request("item-unsent", &[PUBLIC_REPOSITORY]);
    store.project_killed(&open, move |query: &str, _: &Value| {
        query == graphql::CREATION_CONTEXT
    });
    assert!(store.tickets("item-unsent").is_empty());
    let before = store.requests();
    let refused = store.project(&open);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("no second create was sent"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(creates_since(&store, before), 0);
    assert!(store.tickets("item-unsent").is_empty());

    // Created, and crashed before it was filed under its project: the one issue carrying the
    // origin cannot be verified, so it is refused rather than bound or created again.
    let open = request("item-unfiled", &[PUBLIC_REPOSITORY]);
    store.project_killed(&open, carrying("createIssue(input"));
    assert_eq!(store.tickets("item-unfiled").len(), 1);
    let before = store.requests();
    let refused = store.project(&open);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("reconcile"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(creates_since(&store, before), 0);
    assert_eq!(store.tickets("item-unfiled").len(), 1);
}

#[test]
fn two_issues_carrying_one_pending_origin_are_refused_without_a_third() {
    let store = Store::new(Vec::new());
    let open = request("item-twice", &[PUBLIC_REPOSITORY]);
    store.project_killed(&open, carrying("addSubIssue"));
    let first = store.tickets("item-twice");
    assert_eq!(first.len(), 1);
    // A person duplicates the issue, body, origin and all.
    let mut twin = store
        .board
        .fields
        .issues()
        .into_iter()
        .find(|issue| issue["id"] == json!(first[0]))
        .unwrap();
    twin["id"] = json!("TWIN-1");
    twin["item"] = json!("ITEM-TWIN-1");
    store.board.fields.add_items(vec![twin]);
    let before = store.requests();
    let refused = store.project(&open);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("zero or multiple origin matches"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(creates_since(&store, before), 0);
    assert_eq!(store.tickets("item-twice").len(), 2);
}

#[test]
fn simultaneous_managers_projecting_one_item_share_one_ticket() {
    let store = Store::new(Vec::new());
    let open = request("item-shared", &[PUBLIC_REPOSITORY]);
    let answers: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| projected(&store.project(&open))))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert!(
        answers.iter().all(|answer| *answer == answers[0]),
        "{answers:?}"
    );
    assert_eq!(store.tickets("item-shared"), [native(&answers[0])]);
}

#[test]
fn another_host_a_changed_payload_and_a_malformed_request_are_refused_writing_nothing() {
    let store = Store::new(Vec::new());
    let open = request("item-guarded", &[PUBLIC_REPOSITORY]);

    let before = store.requests();
    let refused = store.project_from(&open, "another-host");
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("another-host"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(store.requests(), before);
    assert!(store.tickets("item-guarded").is_empty());

    let created = native(&projected(&store.project(&open)));
    let mut changed = open.clone();
    changed["title"] = json!("A different decision");
    let before = store.requests();
    let refused = store.project(&changed);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("differs in owner, payload, route or privacy"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(store.requests(), before);
    assert_eq!(store.tickets("item-guarded"), [created]);

    let mut malformed = Vec::new();
    for field in [
        "item_id", "owner", "title", "priority", "source", "state", "project",
    ] {
        let mut missing = open.clone();
        missing.as_object_mut().unwrap().remove(field);
        malformed.push((format!("without {field}"), missing));
    }
    for (what, field, value) in [
        ("a question", "state", json!("question")),
        ("another version", "schema_version", json!(2)),
        ("an unknown priority", "priority", json!("critical")),
        ("a blank title", "title", json!("  ")),
        ("a repository list of numbers", "repositories", json!([1])),
        ("an unqualified owner", "owner", json!({"session": "s"})),
    ] {
        let mut wrong = open.clone();
        wrong[field] = value;
        malformed.push((what.to_owned(), wrong));
    }
    for extra in ["question", "kind", "classification"] {
        let mut wrong = open.clone();
        wrong[extra] = json!("question");
        malformed.push((format!("with {extra}"), wrong));
    }
    for (what, request) in malformed {
        let refused = store.project(&request);
        assert_ne!(refused.status.code(), Some(0), "{what} was accepted");
        let failure: Value = serde_json::from_str(&stdout(&refused)).unwrap();
        assert_eq!(
            failure["failure"]["kind"], "projection-input",
            "{what}: {failure}"
        );
    }
    assert_eq!(store.requests(), before);
}

#[test]
fn an_edit_a_person_makes_before_the_terminal_projection_survives_it() {
    let store = Store::new(Vec::new());
    for (item, reason) in [
        ("item-edited", None),
        ("item-edited-reason", Some("Superseded by another example.")),
    ] {
        let open = request(item, &[PUBLIC_REPOSITORY]);
        let created = native(&projected(&store.project(&open)));
        let body = store.board.fields.body(&created);
        let (_, slotted) = body
            .as_str()
            .unwrap()
            .split_once("<!-- onetaskgraph.metadata")
            .unwrap();
        let edited = format!(
            "A person rewrote this between the create and the decision.\n\n<!-- onetaskgraph.metadata{slotted}"
        );
        store.board.fields.edit_body(&created, &edited);

        let mut dismissed = open;
        dismissed["state"] = json!("dismissed");
        dismissed["reason"] = json!(reason);
        let before = store.requests();
        projected(&store.project(&dismissed));
        let body = store.board.fields.body(&created);
        let body = body.as_str().unwrap();
        assert!(
            body.starts_with("A person rewrote this between the create and the decision."),
            "{item}: {body}"
        );
        assert_eq!(
            store.board.fields.state(&created),
            ("CLOSED".to_owned(), Some("NOT_PLANNED".to_owned()))
        );
        match reason {
            // Nothing of the body is written: the state is the issue's own.
            None => {
                assert_eq!(body, edited, "{item}");
                assert!(
                    !store
                        .operations_since(before)
                        .iter()
                        .any(|operation| operation.starts_with("reading")),
                    "{item}"
                );
            }
            // The reason is written against the body as it reads now: the `Status` write answers
            // with the issue whole, and the body is composed against that answer — still two
            // requests, neither of them a read of its own.
            Some(reason) => {
                assert_eq!(slot(&store.board.fields, &created)["human.reason"], reason);
                assert_eq!(
                    store.operations_since(before),
                    [
                        super::plan_copy::operation_name(graphql::UPDATE_FIELD_WHOLE),
                        "updating an issue"
                    ],
                    "{item}"
                );
            }
        }
    }
}

#[test]
fn a_later_cold_process_takes_the_kept_placement_and_a_changed_option_is_read_afresh() {
    let store = Store::new(Vec::new());
    let setup = super::plan_copy::operation_name(graphql::CREATION_CONTEXT);
    let created_with = |item: &str| {
        let before = store.requests();
        let answer = projected(&store.project(&request(item, &[PUBLIC_REPOSITORY])));
        assert_eq!(
            store.board.fields.status(&native(&answer)).as_deref(),
            Some("Todo"),
            "{item}"
        );
        store.operations_since(before)
    };
    assert!(created_with("item-first").contains(&setup));
    // Kept in the source's own cache directory, beside nothing of anybody's work: ids under a
    // token, no title and no body.
    let kept = std::fs::read_to_string(store.sandbox.project().join("cache-human/placement.json"))
        .expect("the placement is kept in the source's cache directory");
    assert!(kept.contains("\"token\""), "{kept}");
    assert!(
        !kept.contains("item-first") && !kept.contains("Choose the next"),
        "{kept}"
    );
    // The binding keeps the written issue's ids and nothing of its work or its state.
    let journals: Vec<String> = std::fs::read_dir(&store.intents)
        .unwrap()
        .flat_map(|identity| std::fs::read_dir(identity.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    let journal: Value = serde_json::from_str(
        journals
            .iter()
            .find(|text| text.contains("\"item-first\""))
            .unwrap(),
    )
    .unwrap();
    let written = journal["phase"]["written"].to_string();
    let kept_ids = journal["phase"]["written"].as_object().unwrap();
    for id in ["issue", "item", "board"] {
        assert!(kept_ids[id].is_string(), "{written}");
    }
    for work in ["Decide on item-first", "Choose the next", "human."] {
        assert!(!written.contains(work), "{work} in {written}");
    }
    for state in [
        "title", "body", "slot", "status", "option", "closed", "reason",
    ] {
        assert!(!kept_ids.contains_key(state), "{state} in {written}");
    }
    // A second process reads the project issue it is filed under, as every create does, finds
    // the kept placement's token unchanged there, and reads no board fields or repository.
    let second = created_with("item-second");
    assert!(!second.contains(&setup), "{second:?}");
    assert_eq!(second.first().copied(), Some("reading one issue"));

    // An option given a new id moves the token: the next process reads its context afresh,
    // writes the option's new id, and keeps that context for the one after it.
    store.board.fields.remint_status_option("Todo");
    assert!(created_with("item-third").contains(&setup));
    assert!(!created_with("item-fourth").contains(&setup));
}

#[test]
fn a_request_on_standard_input_is_projected_and_a_half_landed_terminal_is_finished() {
    use std::io::Write as _;
    let store = Store::new(Vec::new());
    let open = request("item-stdin", &[PUBLIC_REPOSITORY]);
    let mut child = store
        .sandbox
        .subprocess(onetaskgraph_e2e_support::binary())
        .args(["task", "project-human", "--input", "-", "--json"])
        .env("ONETASKGRAPH_HOST", HOST)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(open.to_string().as_bytes())
        .unwrap();
    let created = native(&projected(&child.wait_with_output().unwrap()));
    assert_eq!(store.tickets("item-stdin"), std::slice::from_ref(&created));

    // A reason is recorded only for a dismissal.
    let mut completed = open.clone();
    completed["state"] = json!("completed");
    completed["reason"] = json!("Done early.");
    let before = store.requests();
    assert_ne!(store.project(&completed).status.code(), Some(0));
    assert_eq!(store.requests(), before);

    // The card moves and the close is refused: the retry reads the issue and sends only the
    // close the tracker does not hold yet.
    completed["reason"] = Value::Null;
    store.board.fields.refuse_once("updateIssue(input");
    assert_ne!(store.project(&completed).status.code(), Some(0));
    assert_eq!(store.board.fields.state(&created).0, "OPEN");
    let before = store.requests();
    projected(&store.project(&completed));
    assert_eq!(
        store.operations_since(before),
        ["reading one issue", "updating an issue"]
    );
    assert_eq!(
        store.board.fields.state(&created),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
    assert_eq!(store.board.fields.status(&created).as_deref(), Some("Done"));
}

#[test]
fn kept_contexts_that_cannot_be_read_are_declined_and_read_afresh() {
    let store = Store::new(Vec::new());
    let setup = super::plan_copy::operation_name(graphql::CREATION_CONTEXT);
    projected(&store.project(&request("item-kept", &[PUBLIC_REPOSITORY])));

    // A kept placement that is not one — an empty repository id — is declined: the next create
    // reads its context and keeps a sound one.
    let kept = store.sandbox.project().join("cache-human/placement.json");
    let mut placement: Value =
        serde_json::from_str(&std::fs::read_to_string(&kept).unwrap()).unwrap();
    placement["repository_id"] = json!("");
    std::fs::write(&kept, placement.to_string()).unwrap();
    let before = store.requests();
    let open = request("item-after-corruption", &[PUBLIC_REPOSITORY]);
    let created = native(&projected(&store.project(&open)));
    assert!(store.operations_since(before).contains(&setup));

    // A written context naming another board is declined: the terminal write reads the issue
    // first, then settles it.
    let journal = std::fs::read_dir(&store.intents)
        .unwrap()
        .flat_map(|identity| std::fs::read_dir(identity.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
                && std::fs::read_to_string(path)
                    .unwrap()
                    .contains("\"item-after-corruption\"")
        })
        .expect("the item's journal");
    let mut held: Value =
        serde_json::from_str(&std::fs::read_to_string(&journal).unwrap()).unwrap();
    held["phase"]["written"]["number"] = json!(999);
    std::fs::write(&journal, held.to_string()).unwrap();
    let mut completed = open;
    completed["state"] = json!("completed");
    let before = store.requests();
    projected(&store.project(&completed));
    assert_eq!(
        store.operations_since(before).first().copied(),
        Some("reading one issue")
    );
    assert_eq!(
        store.board.fields.state(&created),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
}

#[test]
fn a_create_answered_without_its_url_reads_it_and_one_with_no_canonical_url_is_refused() {
    // The create's answer names no URL: the issue is read once for it, and the card's URL is
    // still the issue's canonical address.
    let store = Store::new(Vec::new());
    store.board.fields.omit_created_issue_urls();
    let before = store.requests();
    let answer = projected(&store.project(&request("item-unaddressed", &[PUBLIC_REPOSITORY])));
    let created = native(&answer);
    assert_eq!(
        answer["url"],
        format!(
            "https://github.com/{PUBLIC_REPOSITORY}/issues/{}",
            onetaskgraph_e2e_support::fixtures::github_number(&created)
        )
    );
    assert_eq!(
        store.operations_since(before).last().copied(),
        Some("reading one issue"),
        "the URL was read after the create"
    );

    // No address the tracker gives is canonical HTTPS: the projection is refused rather than
    // answered with a link a card cannot use, and a retry sends no second create.
    let store = Store::new(Vec::new());
    store.board.fields.serve_insecure_issue_urls();
    let open = request("item-insecure", &[PUBLIC_REPOSITORY]);
    let refused = store.project(&open);
    assert_ne!(refused.status.code(), Some(0));
    assert!(
        stderr(&refused).contains("canonical"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(store.tickets("item-insecure").len(), 1);
    let before = store.requests();
    let retried = store.project(&open);
    assert_ne!(retried.status.code(), Some(0));
    assert_eq!(creates_since(&store, before), 0, "no second create");
    assert_eq!(store.tickets("item-insecure").len(), 1);
}
