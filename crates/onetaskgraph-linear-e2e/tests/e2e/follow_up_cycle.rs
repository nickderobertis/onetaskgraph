//! One follow-up cycle of batched store calls against a Linear team, through the binary, counted
//! at the loopback Linear's endpoint.
//!
//! A team source holding 400 other open items, every call a fresh process as a caller's SDK
//! makes it: the batch — one `task find` of 20 narrowed queries, one comments read of the 8 items
//! found, one copy of 16 tickets (12 new, 4 bound at held versions), one `comment add-many` of 4
//! and one `update-many` of 4 held multi-field updates — then the same five calls again as a
//! retry, which writes nothing; then a comment pass — the comments since an instant over 20
//! proposals carrying 40 new comments, one read verifying the 40 replies the answering agents
//! wrote, and one `update-many` of 10 held ticket changes — and the same pass again from its
//! watermark, which finds nothing new. The 40 replies themselves are written outside what is
//! counted: one per comment a person wrote, they are no store call this cycle makes.
//!
//! `measure_linear_requests_per_follow_up_cycle` records the figure for the
//! `linear-requests-per-follow-up-cycle` budget, with each call's lookup, read and mutation
//! requests apart.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};
use crate::plan_road::Phase;
use crate::telemetry::record;

/// The instant the comment pass reads from: every person's comment is after it.
const SINCE: &str = "2026-10-01T00:00:00Z";

struct Cycle {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    batches: PathBuf,
}

/// One issue at `Todo`, carrying `metadata`.
fn issue(id: &str, title: &str, metadata: Value) -> Value {
    json!({"id": id, "title": title, "content": format!("The body of {id}."),
           "status": {"category": "todo", "name": "Todo"}, "labels": [], "metadata": metadata})
}

impl Cycle {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let mut tasks = Vec::new();
        for at in 0..400 {
            tasks.push(issue(
                &format!("F-{at}"),
                &format!("Other work {at}"),
                json!({}),
            ));
        }
        // The eight tickets a search finds, under eight of the twenty queries.
        for at in 1..=8 {
            tasks.push(issue(
                &format!("K-{at}"),
                &format!("Known ticket {at}"),
                json!({"cycle.ticket": format!("t-{at}")}),
            ));
        }
        // The twenty proposals, each carrying two comments people wrote after the instant.
        let mut comments = Vec::new();
        for at in 1..=20 {
            let id = format!("P-{at}");
            tasks.push(issue(
                &id,
                &format!("Proposal {at}"),
                json!({"cycle.kind": "proposal"}),
            ));
            for n in 0..2 {
                comments.push(json!({"task": id, "comment": {
                    "id": format!("C-{at}-{n}"), "body": format!("A question {n} on {id}."),
                    "author": "a person",
                    "created_at": format!("2026-10-02T00:{at:02}:{n:02}Z"),
                    "updated_at": format!("2026-10-02T00:{at:02}:{n:02}Z")}}));
            }
        }
        let held = json!({"tasks": tasks, "projects": [], "documents": [], "labels": [],
                          "task_dependencies": [], "project_dependencies": [],
                          "comments": comments});
        let (config, workspace) = linear_workspace(&sandbox, held);
        let drafts = sandbox.subdirectory("drafts");
        std::fs::create_dir_all(drafts.join("tasks")).expect("the tasks folder");
        for at in 1..=16 {
            std::fs::write(
                drafts.join("tasks").join(format!("T-{at}.md")),
                format!("---\ntitle: Ticket {at}\nstatus: todo\n---\nWhat ticket {at} does.\n"),
            )
            .expect("a ticket");
        }
        sandbox.project_document(&document(&json!({
            "drafts": {"plugin": "local-md", "config": {"root": drafts}},
            "team": {"plugin": "linear", "config": config},
        })));
        let batches = sandbox.subdirectory("batches");
        Self {
            sandbox,
            workspace,
            batches,
        }
    }

    /// `onetaskgraph <arguments> --json` on a fresh process, exiting `code`, and its JSON.
    fn json(&self, arguments: &[&str], code: i32) -> Value {
        let mut command = self.sandbox.command();
        command.args(arguments).arg("--json").write_stdin("");
        let output = command.assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON on stdout")
    }

    /// `document` written to a batch file named `name`.
    fn batch(&self, name: &str, document: &Value) -> String {
        let path = self.batches.join(name);
        std::fs::write(&path, document.to_string()).expect("a batch file");
        path_of(&path)
    }

    /// Run `call` and answer what it sent the team, as one measured call named `name`.
    fn measured<T>(
        &self,
        name: &'static str,
        calls: &mut Vec<(&'static str, Phase)>,
        call: impl FnOnce() -> T,
    ) -> T {
        let mark = self.workspace.served_count();
        let answered = call();
        calls.push((name, Phase::of(&self.workspace.served_since(mark))));
        answered
    }
}

fn path_of(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

/// The five calls of one batch, as a retry makes them again: the same documents, held to the
/// versions the first one read. Its comments and its updates are of the same four tickets, so
/// the batch's own updates move every one, and a retry of either writes nothing — on Linear a
/// comment alone does not move its issue's version, which would let a retried comment land
/// twice.
struct Batch {
    find: String,
    found: Vec<String>,
    copy: Vec<String>,
    comments: String,
    updates: String,
}

/// The cycle, walked once and asserted on, and each measured call.
pub(crate) fn walk() -> Vec<(&'static str, Phase)> {
    let cycle = Cycle::new();
    // Four tickets were copied before, and are bound at the versions that copy answered.
    let earlier = cycle.json(
        &[
            "task",
            "copy",
            "drafts:T-13",
            "drafts:T-14",
            "drafts:T-15",
            "drafts:T-16",
            "--to",
            "team",
        ],
        0,
    );
    let mut copy: Vec<String> = ["task", "copy"].map(str::to_owned).to_vec();
    copy.extend((1..=16).map(|at| format!("drafts:T-{at}")));
    copy.extend(["--to".to_owned(), "team".to_owned()]);
    for item in earlier["items"].as_array().expect("items") {
        let version = item["result"]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("the bound copy answers its version: {item}"));
        copy.push("--if-version".to_owned());
        copy.push(format!(
            "{}={version}",
            item["source"].as_str().expect("a source")
        ));
    }

    let mut calls = Vec::new();
    let find = cycle.batch(
        "find.json",
        &json!({"schema_version": 1, "queries": (1..=20).map(|at| json!({
            "key": format!("q-{at}"), "metadata": {"cycle.ticket": format!("t-{at}")}
        })).collect::<Vec<_>>()}),
    );
    let found = cycle.measured("batch: find", &mut calls, || {
        cycle.json(&["task", "find", "team", "--queries", &find], 0)
    });
    let answers = found["answers"].as_array().expect("answers");
    assert_eq!(answers.len(), 20);
    let mut items: Vec<(String, String)> = Vec::new();
    for answer in answers {
        assert_eq!(answer["complete"], true, "{answer}");
        for item in answer["items"].as_array().expect("items") {
            items.push((
                item["id"].as_str().expect("an id").to_owned(),
                item["item"]["version"]
                    .as_str()
                    .expect("a version")
                    .to_owned(),
            ));
        }
    }
    assert_eq!(
        items.len(),
        8,
        "eight of the twenty queries find their ticket"
    );
    let ids: Vec<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
    let batch = Batch {
        find,
        found: ids.iter().map(|id| (*id).to_owned()).collect(),
        copy,
        comments: cycle.batch(
            "comments.json",
            &json!({"schema_version": 1, "comments": items[..4].iter().map(|(id, version)| json!({
                "item": id, "body": format!("Picked up {id}."), "if_version": version
            })).collect::<Vec<_>>()}),
        ),
        updates: cycle.batch(
            "updates.json",
            &json!({"schema_version": 1, "updates": items[..4].iter().map(|(id, version)| json!({
                "item": id, "if_version": version, "status": "in-progress",
                "priority": "high", "metadata": {"cycle.state": "working"},
                "body": format!("Working on {id}.\n")
            })).collect::<Vec<_>>()}),
        ),
    };
    run_batch(&cycle, &batch, &mut calls, false);
    run_batch(&cycle, &batch, &mut calls, true);

    // The comment pass.
    let listed = cycle.measured("pass: comments since", &mut calls, || {
        cycle.json(
            &[
                "task",
                "comments",
                "team",
                "--since",
                SINCE,
                "--metadata",
                "cycle.kind=proposal",
            ],
            0,
        )
    });
    let comments = listed["comments"].as_array().expect("comments");
    assert_eq!(
        comments.len(),
        40,
        "every person's comment since the instant"
    );
    let watermark = listed["watermark"]
        .as_str()
        .expect("a watermark")
        .to_owned();
    // The answering agents' replies, one per comment: not a call of the cycle.
    let replies = cycle.batch(
        "replies.json",
        &json!({"schema_version": 1, "comments": comments.iter().map(|comment| json!({
            "item": comment["item"], "body": format!("An answer to {}.", comment["id"].as_str().unwrap())
        })).collect::<Vec<_>>()}),
    );
    let replied = cycle.json(
        &["task", "comment", "add-many", "team", "--input", &replies],
        0,
    );
    assert!(
        replied["results"]
            .as_array()
            .expect("results")
            .iter()
            .all(|result| result["outcome"] == "written")
    );
    let mut proposals: Vec<String> = (1..=20).map(|at| format!("team:P-{at}")).collect();
    proposals.sort();
    let mut verify: Vec<&str> = vec!["task", "show-many"];
    verify.extend(proposals.iter().map(String::as_str));
    let verified = cycle.measured("pass: verify replies", &mut calls, || {
        cycle.json(&verify, 0)
    });
    let answered: usize = verified["details"]
        .as_array()
        .or_else(|| verified.as_array())
        .expect("details")
        .iter()
        .map(|detail| {
            detail["comments"].as_array().map_or(0, |comments| {
                comments
                    .iter()
                    .filter(|comment| {
                        comment["body"]
                            .as_str()
                            .is_some_and(|body| body.starts_with("An answer"))
                    })
                    .count()
            })
        })
        .sum();
    assert_eq!(answered, 40, "every reply reads back: {verified}");
    let tickets = cycle.json(&["task", "list", "--source", "team", "--all"], 0);
    let copied: Vec<(String, String)> = tickets["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| {
            item["item"]["title"]
                .as_str()
                .is_some_and(|title| title.starts_with("Ticket "))
        })
        .take(10)
        .map(|item| {
            (
                item["id"].as_str().expect("an id").to_owned(),
                item["item"]["version"]
                    .as_str()
                    .expect("a version")
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(copied.len(), 10);
    let changes = cycle.batch(
        "changes.json",
        &json!({"schema_version": 1, "updates": copied.iter().map(|(id, version)| json!({
            "item": id, "if_version": version, "status": "done",
            "metadata": {"cycle.answered": true}
        })).collect::<Vec<_>>()}),
    );
    let changed = cycle.measured("pass: update-many", &mut calls, || {
        cycle.json(&["task", "update-many", "team", "--input", &changes], 0)
    });
    assert!(
        changed["results"]
            .as_array()
            .expect("results")
            .iter()
            .all(|result| result["outcome"] == "written"),
        "{changed}"
    );

    // The same pass from its watermark: nothing new.
    let again = cycle.measured("repeated pass: comments since", &mut calls, || {
        cycle.json(
            &[
                "task",
                "comments",
                "team",
                "--since",
                &watermark,
                "--metadata",
                "cycle.kind=proposal",
            ],
            0,
        )
    });
    assert_eq!(
        again["comments"].as_array().map(Vec::len),
        Some(0),
        "{again}"
    );
    calls
}

/// The batch's five calls — or, as a `retry`, the same five again, which write nothing.
fn run_batch(cycle: &Cycle, batch: &Batch, calls: &mut Vec<(&'static str, Phase)>, retry: bool) {
    let label = |name: &'static str, retried: &'static str| if retry { retried } else { name };
    if retry {
        cycle.measured("retry: find", calls, || {
            cycle.json(&["task", "find", "team", "--queries", &batch.find], 0)
        });
    }
    let mut show: Vec<&str> = vec!["task", "show-many"];
    show.extend(batch.found.iter().map(String::as_str));
    cycle.measured(
        label("batch: comments read", "retry: comments read"),
        calls,
        || cycle.json(&show, 0),
    );
    let copy: Vec<&str> = batch.copy.iter().map(String::as_str).collect();
    let copied = cycle.measured(label("batch: copy", "retry: copy"), calls, || {
        cycle.json(&copy, 0)
    });
    for item in copied["items"].as_array().expect("items") {
        let source = item["source"].as_str().expect("a source");
        let bound = ["drafts:T-13", "drafts:T-14", "drafts:T-15", "drafts:T-16"].contains(&source);
        let expected = if retry || bound {
            "unchanged"
        } else {
            "created"
        };
        assert_eq!(item["action"], expected, "{item}");
    }
    let code = if retry { 4 } else { 0 };
    let commented = cycle.measured(
        label("batch: comment add-many", "retry: comment add-many"),
        calls,
        || {
            cycle.json(
                &[
                    "task",
                    "comment",
                    "add-many",
                    "team",
                    "--input",
                    &batch.comments,
                ],
                code,
            )
        },
    );
    let updated = cycle.measured(
        label("batch: update-many", "retry: update-many"),
        calls,
        || {
            cycle.json(
                &["task", "update-many", "team", "--input", &batch.updates],
                code,
            )
        },
    );
    let outcome = if retry { "version-moved" } else { "written" };
    for result in commented["results"]
        .as_array()
        .into_iter()
        .chain(updated["results"].as_array())
        .flatten()
    {
        assert_eq!(result["outcome"], outcome, "{result}");
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] The budget's own journey, in this crate's `test` target whose telemetry the budget reads, as every `measure_*` journey here is.
#[test]
fn measure_linear_requests_per_follow_up_cycle() {
    let calls = walk();
    let total: usize = calls.iter().map(|(_, phase)| phase.total()).sum();
    let part = |prefix: &str| -> usize {
        calls
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, phase)| phase.total())
            .sum()
    };
    let detail = format!(
        "{total} requests over the cycle — the batch {}, its retry {}, the pass {}, the repeated \
         pass {}: {}",
        part("batch:"),
        part("retry:"),
        part("pass:"),
        part("repeated pass:"),
        calls
            .iter()
            .map(|(name, phase)| phase.render(name))
            .collect::<Vec<_>>()
            .join("; ")
    );
    record("linear-requests-per-follow-up-cycle", total, &detail);
}
