//! What one orchestrator follow-up cycle costs a GitHub board in GraphQL rate-limit points,
//! driven through the compiled binary against the loopback board and recorded as the telemetry
//! of the `github-points-per-follow-up-cycle` budget, which `budget_runner` hands to
//! onebudgetspec, the one judge of it.
//!
//! The workload is the batched calls an orchestrator makes of a board holding 400 other open
//! items, each command a fresh process with no cache:
//!
//! - **batch** — one `task find` of 20 narrowed queries, eight of which find a ticket; one
//!   `task show-many` reading those eight with their comments; one `task copy` of 16 tickets out
//!   of a folder of Markdown, 12 new and 4 already on the board, held at the versions the find
//!   reported; one `task comment add-many` of four comments and one `task update-many` of four
//!   multi-field updates, each held at the version the find reported;
//! - **retry** — the same five calls again: the find and the read as they were, the copy held at
//!   the versions the retry's own find reports, so every one of its tickets is unchanged, and the
//!   two batches with the holds they were first sent with, so each item answers `version-moved`
//!   and nothing is written twice;
//! - **pass** — `task comments --since` over the 20 proposals, which carry 40 new comments; one
//!   `task show-many` of the 20 reading back the 40 replies the journey wrote — those writes are
//!   the orchestrator's own and are set up outside the measurement — and one `task update-many`
//!   of ten held ticket changes;
//! - **repeated pass** — the same three calls from the watermark the pass answered, which finds
//!   nothing new.
//!
//! Every request the board serves is priced as GitHub prices it: by the plugin's own table
//! (`onetaskgraph_github_projects::point_cost`) at the page sizes that request bound.

use std::collections::BTreeMap;
use std::path::PathBuf;

use onetaskgraph_github_projects::{Variables, largest_page_sizes, point_cost};
use serde_json::{Value, json};

use crate::budget_runner::directory;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, github_projects_with_items};

/// The budget this journey records.
pub(crate) const BUDGET: &str = "github-points-per-follow-up-cycle";
/// The workload the budget states, which its report refuses any other recording of.
pub(crate) const WORKLOAD: &str = "a board of 400 other open items, 20 proposals and 8 tickets; \
    batch: find of 20 queries, show-many of 8, copy of 16 (12 new, 4 held), add-many of 4, \
    update-many of 4 held; the batch again as a retry; a comment pass over 20 proposals with 40 \
    new comments: comments since, show-many of 20 verifying 40 replies, update-many of 10 held; \
    the pass again from its watermark";

/// The cycle's phases, in the order it runs them, as the recording names them.
pub(crate) const PHASES: [&str; 4] = ["batch", "retry", "pass", "repeated pass"];

/// The other open items on the board.
const OTHERS: usize = 400;
/// The proposals the comment pass reads.
const PROPOSALS: usize = 20;
/// The tickets the find finds: the first four copied from the folder, the other four the
/// board's own.
const FOUND: usize = 8;
/// The tickets the copy creates.
const NEW: usize = 12;

/// The page sizes `variables` binds, each page-size variable at what the request sent and at
/// its largest where it sent none.
fn bound(variables: &Value) -> Variables {
    let mut bound = largest_page_sizes();
    for (name, size) in &mut bound {
        if let Some(sent) = variables.get(name).and_then(Value::as_u64) {
            *size = u32::try_from(sent).expect("a page size fits u32");
        }
    }
    bound
}

/// The points one request spent, as GitHub prices it.
pub(crate) fn points_of(document: &str, variables: &Value) -> u64 {
    point_cost(document, &bound(variables)).expect("a document the plugin prices")
}

/// What one call sent the board.
struct Call {
    name: &'static str,
    served: Vec<(String, Value)>,
}

impl Call {
    fn points(&self) -> u64 {
        self.served
            .iter()
            .map(|(document, variables)| points_of(document, variables))
            .sum()
    }
    /// What it would spend were every page it asked for as large as the plugin can ask.
    fn worst_case(&self) -> u64 {
        self.served
            .iter()
            .map(|(document, _)| {
                onetaskgraph_github_projects::worst_case_point_cost(document)
                    .expect("a document the plugin prices")
            })
            .sum()
    }
    fn record(&self) -> Value {
        json!({"call": self.name, "requests": self.served.len(), "points": self.points()})
    }
}

/// The ticket of proposal `n`: its metadata, and what the find's query of it names.
fn follow_up(n: usize) -> Value {
    json!({"proposal": format!("p{n}")})
}

fn board_item(id: &str, title: &str, body: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,"body":body,
        "state":"OPEN","reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
        "status":"Todo","origin":"","labels":[]})
}

struct Cycle {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    notes: PathBuf,
    /// The board ids of the tickets the find finds, `K-0` … `K-7` by proposal.
    found: Vec<String>,
}

impl Cycle {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let mut items: Vec<Value> = (0..OTHERS)
            .map(|n| {
                board_item(
                    &format!("X-{n}"),
                    &format!("Other work {n}"),
                    &format!("other {n}"),
                )
            })
            .collect();
        items.extend((0..PROPOSALS).map(|n| {
            board_item(
                &format!("PROP-{n}"),
                &format!("Proposal {n}"),
                &format!("proposal {n}"),
            )
        }));
        items.extend((4..FOUND).map(|n| {
            board_item(
                &format!("K-{n}"),
                &format!("Follow-up of proposal {n}"),
                &format!("follow-up {n}"),
            )
        }));
        let (config, board) = github_projects_with_items(&sandbox, items);
        for n in 0..PROPOSALS {
            board.set_metadata(
                &format!("PROP-{n}"),
                "orchestrator.proposal",
                json!({"state": "open"}),
            );
        }
        for n in 4..FOUND {
            board.set_metadata(&format!("K-{n}"), "orchestrator.follow-up", follow_up(n));
        }
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
        sandbox.project_document(
            &serde_json::to_string_pretty(&json!({
                "sources": {
                    "board": {"plugin": "github-projects", "config": config},
                    "notes": {"plugin": "local-md", "config": {"root": notes,
                        "status_mapping": {"Todo": "todo", "Doing": "in-progress"}}},
                },
            }))
            .expect("a document"),
        );
        let cycle = Self {
            sandbox,
            board,
            notes,
            found: Vec::new(),
        };
        for n in (0..4).chain(FOUND..FOUND + NEW) {
            cycle.author(n);
        }
        // The four tickets the folder already holds on the board: copied once, so each records
        // its link and the board holds it.
        let copied = cycle.run(
            &[
                "task", "copy", "notes:k0", "notes:k1", "notes:k2", "notes:k3", "--to", "board",
                "--json",
            ],
            "",
            0,
        );
        let mut found: Vec<String> = copied["items"]
            .as_array()
            .expect("the items")
            .iter()
            .map(|item| {
                item["destination"]
                    .as_str()
                    .expect("a destination")
                    .trim_start_matches("board:")
                    .to_owned()
            })
            .collect();
        found.extend((4..FOUND).map(|n| format!("K-{n}")));
        // Every change before a read has settled, so each read reports a version.
        cycle.board.pass_time(120);
        Self { found, ..cycle }
    }

    /// Write the folder's ticket `k<n>`, the follow-up of proposal `n`.
    fn author(&self, n: usize) {
        std::fs::write(
            self.notes.join(format!("tasks/k{n}.md")),
            format!(
                "---\ntitle: Follow-up of proposal {n}\nstatus: Todo\nmetadata:\n  orchestrator.follow-up:\n    proposal: p{n}\n---\nfollow-up {n}\n"
            ),
        )
        .expect("a ticket");
    }

    /// `onetaskgraph <arguments>` with `stdin`, exiting `code`, and its JSON.
    fn run(&self, arguments: &[&str], stdin: &str, code: i32) -> Value {
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .write_stdin(stdin)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`:\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
    }

    /// [`Self::run`], with every request the board served for it.
    fn measured(
        &self,
        name: &'static str,
        arguments: &[&str],
        stdin: &str,
        code: i32,
    ) -> (Value, Call) {
        let mark = self.board.served_count();
        let answer = self.run(arguments, stdin, code);
        let served = self.board.served_since(mark);
        (answer, Call { name, served })
    }

    /// The find of the 20 proposals' follow-ups: its answer, and each found ticket's version by
    /// board id.
    fn find(&self) -> (Call, BTreeMap<String, String>) {
        let queries: Vec<Value> = (0..PROPOSALS)
            .map(|n| {
                json!({"key": format!("p{n}"),
                       "metadata": {"orchestrator.follow-up/proposal": format!("p{n}")}})
            })
            .collect();
        let request = json!({"schema_version": 1, "queries": queries}).to_string();
        let (answer, call) = self.measured(
            "find",
            &["task", "find", "board", "--queries", "-", "--json"],
            &request,
            0,
        );
        let mut versions = BTreeMap::new();
        for found in answer["answers"].as_array().expect("the answers") {
            assert_eq!(found["complete"], true, "{found}");
            for item in found["items"].as_array().expect("the items") {
                let id = item["id"].as_str().expect("an id");
                let version = item["item"]["version"].as_str().expect("a version");
                versions.insert(
                    id.trim_start_matches("board:").to_owned(),
                    version.to_owned(),
                );
            }
        }
        (call, versions)
    }

    fn show_many(&self, ids: &[String]) -> Call {
        let mut arguments = vec!["task", "show-many", "--json"];
        let qualified: Vec<String> = ids.iter().map(|id| format!("board:{id}")).collect();
        arguments.extend(qualified.iter().map(String::as_str));
        self.measured("show-many", &arguments, "", 0).1
    }

    /// The copy of the 16 tickets, the four already on the board held at `versions`.
    fn copy(&self, versions: &BTreeMap<String, String>) -> (Value, Call) {
        let mut arguments: Vec<String> = ["task", "copy"].map(str::to_owned).to_vec();
        arguments.extend((0..4).map(|n| format!("notes:k{n}")));
        arguments.extend((FOUND..FOUND + NEW).map(|n| format!("notes:k{n}")));
        arguments.extend(["--to", "board", "--json"].map(str::to_owned));
        for n in 0..4 {
            arguments.push("--if-version".to_owned());
            arguments.push(format!("notes:k{n}={}", versions[&self.found[n]]));
        }
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        self.measured("copy", &arguments, "", 0)
    }

    fn add_many(&self, batch: &Value, code: i32) -> (Value, Call) {
        self.measured(
            "add-many",
            &[
                "task", "comment", "add-many", "board", "--input", "-", "--json",
            ],
            &batch.to_string(),
            code,
        )
    }

    fn update_many(&self, batch: &Value, code: i32) -> (Value, Call) {
        self.measured(
            "update-many",
            &["task", "update-many", "board", "--input", "-", "--json"],
            &batch.to_string(),
            code,
        )
    }

    fn comments_since(&self, since: &str) -> (Value, Call) {
        self.measured(
            "comments-since",
            &[
                "task",
                "comments",
                "board",
                "--since",
                since,
                "--metadata",
                "orchestrator.proposal/state=open",
                "--json",
            ],
            "",
            0,
        )
    }

    fn proposals(&self) -> Vec<String> {
        (0..PROPOSALS).map(|n| format!("PROP-{n}")).collect()
    }

    /// Run the whole cycle, every call's measurement in order, by phase.
    fn measure(&self) -> Vec<(&'static str, Vec<Call>)> {
        // batch
        let (find, versions) = self.find();
        assert_eq!(versions.len(), FOUND, "{versions:?}");
        let read = self.show_many(&self.found);
        let (copied, copy) = self.copy(&versions);
        let actions: Vec<&str> = copied["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["action"].as_str().expect("an action"))
            .collect();
        assert_eq!(
            actions
                .iter()
                .filter(|action| **action == "created")
                .count(),
            NEW,
            "{copied:#}"
        );
        let comments = json!({"schema_version": 1, "comments": (0..4).map(|n| json!({
            "item": format!("board:{}", self.found[n]),
            "body": format!("Filed as the follow-up of proposal {n}."),
            "if_version": versions[&self.found[n]],
        })).collect::<Vec<_>>()});
        let (added, add) = self.add_many(&comments, 0);
        assert!(
            added["results"]
                .as_array()
                .expect("results")
                .iter()
                .all(|result| result["outcome"] == "written"),
            "{added:#}"
        );
        let updates = json!({"schema_version": 1, "updates": (4..FOUND).map(|n| json!({
            "item": format!("board:{}", self.found[n]),
            "if_version": versions[&self.found[n]],
            "body": format!("follow-up {n}, triaged"),
            "status": "in-progress",
            "priority": "high",
            "when_priority": ["none"],
            "metadata": {"orchestrator.triage": {"by": "orchestrator"}},
        })).collect::<Vec<_>>()});
        let (updated, update) = self.update_many(&updates, 0);
        assert!(
            updated["results"]
                .as_array()
                .expect("results")
                .iter()
                .all(|result| result["outcome"] == "written"),
            "{updated:#}"
        );
        let batch = vec![find, read, copy, add, update];
        self.board.pass_time(120);

        // retry
        let (find, retried) = self.find();
        assert_eq!(retried.len(), FOUND + NEW, "{retried:?}");
        let read = self.show_many(&self.found);
        let (copied, copy) = self.copy(&retried);
        assert!(
            copied["items"]
                .as_array()
                .expect("items")
                .iter()
                .all(|item| item["action"] == "unchanged"),
            "{copied:#}"
        );
        let (added, add) = self.add_many(&comments, 4);
        assert!(
            added["results"]
                .as_array()
                .expect("results")
                .iter()
                .all(|result| result["outcome"] == "version-moved"),
            "{added:#}"
        );
        let (updated, update) = self.update_many(&updates, 4);
        assert!(
            updated["results"]
                .as_array()
                .expect("results")
                .iter()
                .all(|result| result["outcome"] == "version-moved"),
            "{updated:#}"
        );
        let retry = vec![find, read, copy, add, update];

        // pass
        let since = self.board.now();
        for proposal in self.proposals() {
            for n in 0..2 {
                self.board
                    .add_comment_by(&proposal, &format!("question {n}"), "ada");
            }
        }
        self.board.pass_time(120);
        let (seen, comments_read) = self.comments_since(&since);
        assert_eq!(
            seen["comments"].as_array().expect("comments").len(),
            2 * PROPOSALS,
            "{seen:#}"
        );
        let watermark = seen["watermark"].as_str().expect("a watermark").to_owned();
        // The orchestrator's 40 replies: its own writes, outside what this measures.
        for proposal in self.proposals() {
            for n in 0..2 {
                self.board.add_comment(&proposal, &format!("answer {n}"));
            }
        }
        self.board.pass_time(120);
        let verify = self.show_many(&self.proposals());
        let new: Vec<String> = retried
            .keys()
            .filter(|id| !self.found.contains(id))
            .take(10)
            .cloned()
            .collect();
        let changes = json!({"schema_version": 1, "updates": new.iter().map(|id| json!({
            "item": format!("board:{id}"),
            "if_version": retried[id],
            "status": "in-progress",
            "metadata": {"orchestrator.answered": true},
        })).collect::<Vec<_>>()});
        let (changed, change) = self.update_many(&changes, 0);
        assert!(
            changed["results"]
                .as_array()
                .expect("results")
                .iter()
                .all(|result| result["outcome"] == "written"),
            "{changed:#}"
        );
        let pass = vec![comments_read, verify, change];
        self.board.pass_time(120);

        // repeated pass
        let (seen, comments_read) = self.comments_since(&watermark);
        assert_eq!(
            seen["comments"].as_array().expect("comments").len(),
            0,
            "{seen:#}"
        );
        let verify = self.show_many(&self.proposals());
        let (_, change) = self.update_many(&changes, 4);
        let repeated = vec![comments_read, verify, change];

        let [batched, retried, passed, repeated_pass] = PHASES;
        vec![
            (batched, batch),
            (retried, retry),
            (passed, pass),
            (repeated_pass, repeated),
        ]
    }
}

/// Write the recording: the total, and each phase's calls with their requests and points.
fn record(phases: &[(&'static str, Vec<Call>)]) -> (u64, String) {
    let total: u64 = phases
        .iter()
        .flat_map(|(_, calls)| calls)
        .map(Call::points)
        .sum();
    let worst: u64 = phases
        .iter()
        .flat_map(|(_, calls)| calls)
        .map(Call::worst_case)
        .sum();
    let detail = phases
        .iter()
        .map(|(phase, calls)| {
            let each = calls
                .iter()
                .map(|call| {
                    format!(
                        "{} {} requests {} points",
                        call.name,
                        call.served.len(),
                        call.points()
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let points: u64 = calls.iter().map(Call::points).sum();
            format!("{phase} {points} points ({each})")
        })
        .collect::<Vec<_>>()
        .join("; ");
    let detail = format!("{detail}; {worst} points at the largest page sizes");
    let by_phase: Vec<Value> = phases
        .iter()
        .map(|(phase, calls)| {
            json!({"phase": phase, "calls": calls.iter().map(Call::record).collect::<Vec<_>>()})
        })
        .collect();
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        json!({"budget": BUDGET, "value": total, "workload": WORKLOAD, "phases": by_phase,
               "detail": detail})
        .to_string(),
    )
    .expect("the telemetry file");
    (total, detail)
}

#[test]
fn measure_github_points_per_follow_up_cycle() {
    let cycle = Cycle::new();
    let phases = cycle.measure();
    let (total, detail) = record(&phases);
    eprintln!("{BUDGET}: {total} points; {detail}");
}
