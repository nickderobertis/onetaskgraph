//! What projecting human tasks costs a shared board: 100 items projected through the binary by
//! 10 simultaneous managers onto one screened loopback board of 2,000 items, each resolved once —
//! 34 completed, 33 dismissed with no reason and 33 with one — with ten creates whose answer is
//! lost and recovered, ten terminal writes whose answer is lost and retried cold, and ten warm
//! retries after them.
//!
//! Every request is priced by the plugin's own point-cost table; every aliased mutation field is
//! counted beside it and charged five secondary points, a read one. The recording is the
//! `human-task-projection-points` budget's telemetry, which `human-items/report.mjs` hands to
//! onebudgetspec. What is held here is what the tickets are, never what they cost.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use onetaskgraph_e2e_support::common::{Sandbox, stderr, stdout};
use onetaskgraph_e2e_support::fixtures::{GitHubBoardFields, document};
use onetaskgraph_e2e_support::onevcs::{BoundaryRecord, OnevcsHome};
use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

use super::human_items::{HOST, PROJECT, PUBLIC_REPOSITORY, board, native, slot};

pub(super) const BUDGET: &str = "human-task-projection-points";

/// Named in the recording, so the reporter refuses telemetry another journey wrote.
pub(super) const WORKLOAD: &str = "human task projection lifecycle";

const TASKS: usize = 100;
const MANAGERS: usize = 10;
const BOARD_ITEMS: usize = 2_000;

/// The secondary points a mutation's every aliased field is charged, as the plugin's pacing
/// model charges it; a read is charged one.
const SECONDARY_PER_FIELD: u64 = 5;

/// One host: the screened board every manager projects onto, and the intents they share.
struct Host {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    home: OnevcsHome,
    _record: BoundaryRecord,
}

impl Host {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let items = (0..BOARD_ITEMS - 1)
            .map(super::plan_copy::background)
            .collect();
        let board = board(&sandbox, items);
        let (owner, name) = PUBLIC_REPOSITORY.split_once('/').unwrap();
        let mut home = OnevcsHome::new(&sandbox);
        home.public(owner, name);
        let record = home.recorded(&sandbox);
        let intents: PathBuf = sandbox.subdirectory("intents");
        let sources = json!({"human": {
            "plugin": "github-projects", "config": board.config, "visibility": "public",
            "cache": {"dir": sandbox.subdirectory("cache-human")},
            "task_projection": {"host": HOST, "directory": intents, "project": PROJECT}}});
        let mut configured: Value = serde_json::from_str(&document(&sources)).unwrap();
        configured["write_policy"] = record.write_policy().clone();
        sandbox.project_document(&configured.to_string());
        Self {
            sandbox,
            board: board.fields,
            home,
            _record: record,
        }
    }

    /// The binary projecting `request` with `token` as its board credential.
    fn command(&self, request: &Value, token: &str) -> Command {
        let path = self.sandbox.project().join(format!(
            "request-{}-{}.json",
            request["item_id"].as_str().unwrap(),
            request["state"].as_str().unwrap()
        ));
        std::fs::write(&path, request.to_string()).unwrap();
        let mut command = self.sandbox.subprocess(onetaskgraph_e2e_support::binary());
        command
            .args(["task", "project-human", "--input"])
            .arg(&path)
            .arg("--json")
            .env("ONETASKGRAPH_HOST", HOST)
            .env("ONEVCS_HOME", self.home.path())
            .env("GITHUB_PROJECTS_FIXTURE_TOKEN", token);
        command
    }

    fn project(&self, request: &Value, token: &str) -> Value {
        let output: Output = self.command(request, token).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).unwrap()
    }

    /// Project `request`, the binary killed once the board applied the first request carrying
    /// `operation` — its answer never reaching it.
    fn crashed(&self, request: &Value, token: &str, operation: &'static str) {
        let mut command = self.command(request, token);
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let child = Arc::new(Mutex::new(command.spawn().unwrap()));
        let killed = Arc::clone(&child);
        self.board.after_request(
            move |query: &str, _: &Value| query.contains(operation),
            0,
            move |_| {
                let _ = killed.lock().unwrap().kill();
            },
        );
        let status = loop {
            if let Some(status) = child.lock().unwrap().try_wait().unwrap() {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(!status.success(), "the binary finished before {operation}");
    }

    /// What `token` has been sent from request `from` on, as one phase's priced row.
    fn row(&self, token: &str, from: usize, phase: &str) -> Value {
        let requests = self.board.served_to(token)[from..].to_vec();
        let priced: Vec<Value> = requests
            .iter()
            .map(|(query, variables)| {
                let mutation = query.trim_start().starts_with("mutation");
                let fields = if mutation {
                    super::asset_board::top_level_fields(query, variables) as u64
                } else {
                    0
                };
                json!({"operation": super::plan_copy::operation_name(query),
                       "points": worst_case_point_cost(query).expect("a priced document"),
                       "mutation_fields": fields,
                       "secondary_points": if mutation { SECONDARY_PER_FIELD * fields } else { 1 }})
            })
            .collect();
        json!({"phase": phase, "requests": priced.len(), "priced": priced})
    }

    fn sent(&self, token: &str) -> usize {
        self.board.served_to(token).len()
    }
}

/// Item `n`'s open request, owned by its manager's session.
fn open(n: usize) -> Value {
    json!({"schema_version": 1, "item_id": format!("human-item-{n}"),
        "owner": {"session": format!("manager-{}", n / 10), "run": null, "plan": null},
        "title": format!("Review synthetic decision {n}"),
        "body": "Choose the next public example.", "priority": "high",
        "repositories": [format!("github.com/{PUBLIC_REPOSITORY}")],
        "project": null, "source": "human", "state": "open", "reason": null})
}

/// Item `n`'s terminal request: completed for every third, dismissed otherwise — with a reason
/// for every third of those.
fn terminal(n: usize) -> Value {
    let mut request = open(n);
    match n % 3 {
        0 => request["state"] = json!("completed"),
        1 => request["state"] = json!("dismissed"),
        _ => {
            request["state"] = json!("dismissed");
            request["reason"] = json!("Superseded by another example.");
        }
    }
    request
}

/// Item `n`'s one ticket, held to what its terminal request says.
fn settled(board: &GitHubBoardFields, n: usize, answer: &Value) {
    let id = native(answer);
    let needle = format!("\"human.item_id\":\"human-item-{n}\"");
    let tickets = board
        .issues()
        .into_iter()
        .filter(|issue| {
            issue["body"]
                .as_str()
                .is_some_and(|body| body.contains(&needle))
        })
        .count();
    assert_eq!(tickets, 1, "item {n} has one ticket");
    let reason = if n.is_multiple_of(3) {
        "COMPLETED"
    } else {
        "NOT_PLANNED"
    };
    assert_eq!(
        board.state(&id),
        ("CLOSED".to_owned(), Some(reason.to_owned())),
        "item {n}"
    );
    let held = slot(board, &id);
    assert_eq!(
        held.get("human.reason").and_then(Value::as_str),
        (n % 3 == 2).then_some("Superseded by another example."),
        "item {n}"
    );
    assert!(
        answer["url"]
            .as_str()
            .unwrap()
            .starts_with(&format!("https://github.com/{PUBLIC_REPOSITORY}/issues/")),
        "{answer}"
    );
}

/// Every priced request of `rows`, summed by operation and in total.
fn summed(rows: &[Value], key: &str) -> Vec<Value> {
    let mut by: std::collections::BTreeMap<String, [u64; 4]> = std::collections::BTreeMap::new();
    for row in rows {
        if key == "phase" {
            by.entry(row["phase"].as_str().unwrap().to_owned())
                .or_default();
        }
        for request in row["priced"].as_array().unwrap() {
            let name = if key == "operation" {
                request["operation"].as_str().unwrap().to_owned()
            } else {
                row["phase"].as_str().unwrap().to_owned()
            };
            let entry = by.entry(name).or_default();
            entry[0] += 1;
            entry[1] += request["points"].as_u64().unwrap();
            entry[2] += request["mutation_fields"].as_u64().unwrap();
            entry[3] += request["secondary_points"].as_u64().unwrap();
        }
    }
    by.into_iter()
        .map(|(name, [requests, points, fields, secondary])| {
            json!({key: name, "requests": requests, "points": points,
                   "mutation_fields": fields, "secondary_points": secondary})
        })
        .collect()
}

#[test]
fn a_hundred_tasks_from_ten_managers_are_projected_recovered_and_resolved_once_each() {
    let host = Host::new();
    // Nine of each manager's ten items, every manager at once.
    let rows: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..MANAGERS)
            .map(|manager| {
                let host = &host;
                scope.spawn(move || {
                    let token = format!("test-token-projection-{manager}");
                    let mut rows = Vec::new();
                    for offset in 1..10 {
                        let n = manager * 10 + offset;
                        let from = host.sent(&token);
                        let created = host.project(&open(n), &token);
                        rows.push(host.row(&token, from, "create"));
                        let from = host.sent(&token);
                        let resolved = host.project(&terminal(n), &token);
                        rows.push(host.row(&token, from, "terminal"));
                        assert_eq!(resolved, created, "item {n}");
                        settled(&host.board, n, &resolved);
                    }
                    rows
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    let mut rows = rows;
    // Each manager's first item, one manager after another so each crash lands where it is
    // meant to: its create's answer lost and recovered, its terminal's lost and retried cold,
    // and a warm retry after.
    for manager in 0..MANAGERS {
        let n = manager * 10;
        let token = format!("test-token-projection-{manager}");
        let from = host.sent(&token);
        host.crashed(&open(n), &token, "addSubIssue");
        rows.push(host.row(&token, from, "create"));
        let from = host.sent(&token);
        let recovered = host.project(&open(n), &token);
        rows.push(host.row(&token, from, "create_recovery"));
        let from = host.sent(&token);
        host.crashed(&terminal(n), &token, "updateIssue(input");
        rows.push(host.row(&token, from, "terminal"));
        let from = host.sent(&token);
        let retried = host.project(&terminal(n), &token);
        rows.push(host.row(&token, from, "cold_terminal_retry"));
        let from = host.sent(&token);
        let warm = host.project(&terminal(n), &token);
        rows.push(host.row(&token, from, "warm_noop"));
        assert_eq!(retried, recovered, "item {n}");
        assert_eq!(warm, recovered, "item {n}");
        settled(&host.board, n, &warm);
    }

    let phases = summed(&rows, "phase");
    let operations = summed(&rows, "operation");
    let total = |field: &str| -> u64 {
        phases
            .iter()
            .map(|phase| phase[field].as_u64().unwrap())
            .sum()
    };
    let totals = json!({"requests": total("requests"), "points": total("points"),
        "mutation_fields": total("mutation_fields"),
        "secondary_points": total("secondary_points")});
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec_pretty(&json!({
            "budget": BUDGET, "workload": WORKLOAD, "tasks": TASKS, "managers": MANAGERS,
            "board_items": BOARD_ITEMS, "screened": true,
            "terminals": {"completed": 34, "dismissed_without_reason": 33,
                          "dismissed_with_reason": 33},
            "recoveries": {"create": MANAGERS, "cold_terminal": MANAGERS, "warm_noop": MANAGERS},
            "secondary_model": {"per_mutation_field": SECONDARY_PER_FIELD, "per_read": 1},
            "totals": totals, "phases": phases, "operations": operations}))
        .unwrap(),
    )
    .unwrap();
}
