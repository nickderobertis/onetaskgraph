//! One Linear team's vocabulary — its workflow states, its workspace's project statuses and the
//! per-kind `status_mapping` configured over them — shared by every suite whose journeys drive
//! a Linear source configured that way.
//!
//! It is here rather than in the Linear suite because the engine's own journeys (a command
//! boundary, a routed copy) configure the same team, and a second spelling of it there would
//! be a second vocabulary nobody reconciles. The project statuses are as `projectStatuses`
//! answered on 2026-10-05, and the team states are the ones the mapping names, with `Triage`
//! and `In Review` beside them so an unmapped name has something to read.

use serde_json::{Value, json};

/// The example team's `status_mapping`, as a host that configures one per kind writes it.
pub fn example_mapping() -> Value {
    json!({
        "backlog":     {"task": "Proposed",        "project": "Proposal"},
        "draft":       {"task": "Backlog",         "project": "Idea"},
        "todo":        {"task": "Todo",            "project": "Planned"},
        "queued":      {"task": "Queued",          "project": "Accepted"},
        "in-progress": "In Progress",
        "unknown":     {"task": "Needs Attention", "project": "Blocked"},
        "done":        {"task": "Done",            "project": "Completed"},
        "cancelled":   "Canceled",
    })
}

/// The team's workflow states: the eight the mapping names, and `Triage` and `In Review`.
pub const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-proposed", "Proposed", "backlog"),
    ("S-backlog", "Backlog", "backlog"),
    ("S-todo", "Todo", "unstarted"),
    ("S-queued", "Queued", "unstarted"),
    ("S-in-progress", "In Progress", "started"),
    ("S-needs-attention", "Needs Attention", "started"),
    ("S-done", "Done", "completed"),
    ("S-canceled", "Canceled", "canceled"),
    ("S-triage", "Triage", "triage"),
    ("S-in-review", "In Review", "started"),
];

/// The example team's project statuses, in Linear's order.
pub const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Idea", "backlog"),
    ("Proposal", "backlog"),
    ("Backlog", "backlog"),
    ("Discovery", "planned"),
    ("Planned", "planned"),
    ("Accepted", "planned"),
    ("Requirements Gathering", "started"),
    ("In Design", "started"),
    ("PRD Review", "started"),
    ("In Progress", "started"),
    ("Blocked", "started"),
    ("Maintenance", "completed"),
    ("Completed", "completed"),
    ("Canceled", "canceled"),
];
