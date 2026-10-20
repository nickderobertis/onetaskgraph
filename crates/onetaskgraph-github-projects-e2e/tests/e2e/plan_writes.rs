//! What a re-copy of the generated plan writes onto the loopback board: only the members that
//! changed, each in the one mutation that carries it, and nothing at all for an item that did
//! not change.
//!
//! The base this change starts from sent one `updateIssue` per project and task on every
//! re-copy — the timing spike's no-op writes — carrying a title, a body and a state the board
//! already held. The board records every request it is sent, so a re-copy writing anything it
//! did not have to shows up here as a mutation naming an item nothing changed.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Counterparts, Plan, SMALL, TASKS, counterparts, operation};
use serde_json::{Value, json};

/// Every mutation among `calls`: the plugin's name for it and its variables.
fn mutations(calls: &[Call]) -> Vec<(String, Value)> {
    calls
        .iter()
        .filter(|call| matches!(call.kind, Kind::Mutation | Kind::AliasedMutations))
        .map(|call| {
            let request: Value = serde_json::from_slice(&call.bytes).unwrap();
            (operation(call), request["variables"].clone())
        })
        .collect()
}

/// The mutations among `calls` naming the item `content` (its issue) or `card` (its board
/// item) in any input they carry.
fn naming<'a>(
    written: &'a [(String, Value)],
    content: &str,
    card: &str,
) -> Vec<&'a (String, Value)> {
    written
        .iter()
        .filter(|(_, variables)| {
            let text = variables.to_string();
            text.contains(&format!("\"{content}\"")) || text.contains(&format!("\"{card}\""))
        })
        .collect()
}

/// The keys of an `updateIssue`'s input besides the id it names: the members it writes.
fn members(variables: &Value) -> Vec<String> {
    let mut members: Vec<String> = variables["input"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|key| *key != "id")
        .cloned()
        .collect();
    members.sort();
    members
}

impl Plan {
    /// Rewrite one line of task `n`'s file — its front matter or its content — keeping every
    /// other byte, the link an earlier copy recorded in it included.
    fn edit_task(&self, n: usize, from: &str, to: &str) {
        let path = self.root.join(format!("tasks/T-{n}.md"));
        let held = std::fs::read_to_string(&path).unwrap();
        assert!(held.contains(from), "T-{n} holds {from:?}: {held}");
        std::fs::write(&path, held.replacen(from, to, 1)).unwrap();
    }
}

/// The board item `id` as the board holds it, every member a copy writes.
fn held(plan: &Plan, id: &str) -> Value {
    let item = plan
        .github
        .item(id)
        .unwrap_or_else(|| panic!("no item {id}"));
    json!({"title": item["title"], "body": item["body"], "state": item["state"],
           "status": item["status"], "priority": item["priority"], "parent": item["parent"],
           "blockers": plan.github.blockers(id)})
}

/// Every plan item as the board holds it, by the plan's own id.
fn everything(plan: &Plan, at: &Counterparts) -> std::collections::BTreeMap<String, Value> {
    at.iter()
        .map(|(source, id)| (source.clone(), held(plan, id)))
        .collect()
}

#[test]
fn a_recopy_writes_each_changed_member_alone_and_an_unchanged_item_not_at_all() {
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let task = |n: usize| at[&format!("T-{n}")].clone();
    let card = |id: &str| format!("ITEM-{id}");

    // One task's body changes: exactly one `updateIssue`, naming that task, carrying its body
    // and nothing else — no title, no state, no field and no other item.
    let before = everything(&plan, &at);
    plan.edit_task(4, "\n---\n", "\n---\nA line added since the last copy.\n");
    let (_, _, calls) = plan.copy();
    let written = mutations(&calls);
    assert_eq!(written.len(), 1, "{written:#?}");
    let (name, variables) = &written[0];
    assert_eq!(name, "updating an issue");
    assert_eq!(variables["input"]["id"], json!(task(4)));
    assert_eq!(members(variables), ["body"], "{variables:#}");
    let after = everything(&plan, &at);
    for (source, item) in &after {
        if source == "T-4" {
            assert_ne!(item["body"], before[source]["body"]);
            let mut unchanged = item.clone();
            unchanged["body"] = before[source]["body"].clone();
            assert_eq!(unchanged, before[source], "T-4 changed its body alone");
        } else {
            assert_eq!(item, &before[source], "{source} is as it was");
        }
    }

    // Nothing changes: no mutation at all.
    let before = everything(&plan, &at);
    let (_, _, calls) = plan.copy();
    assert_eq!(
        mutations(&calls),
        Vec::new(),
        "an unchanged re-copy writes nothing"
    );
    assert_eq!(everything(&plan, &at), before);

    // Four tasks change one member each — T-1 its body, T-2 its title, T-3 its state, from
    // open to closed, and T-5 its priority — and each is written as that member alone.
    plan.edit_task(1, "\n---\n", "\n---\nA new opening line.\n");
    plan.edit_task(
        2,
        "title: Step 2 of the plan",
        "title: The second step, renamed",
    );
    plan.edit_task(3, "status: todo", "status: done");
    plan.edit_task(5, "status: todo\n", "status: todo\npriority: high\n");
    let before = everything(&plan, &at);
    let (_, _, calls) = plan.copy();
    let written = mutations(&calls);

    let of = |n: usize| naming(&written, &task(n), &card(&task(n)));
    let body = of(1);
    assert_eq!(body.len(), 1, "{body:#?}");
    assert_eq!(body[0].0, "updating an issue");
    assert_eq!(members(&body[0].1), ["body"]);

    let title = of(2);
    assert_eq!(title.len(), 1, "{title:#?}");
    assert_eq!(title[0].0, "updating an issue");
    assert_eq!(members(&title[0].1), ["title"]);
    assert_eq!(title[0].1["input"]["title"], "The second step, renamed");

    // Closing it selects the `Done` option and then closes the issue: the `updateIssue`
    // carries the state and nothing else.
    let state = of(3);
    assert_eq!(state.len(), 2, "{state:#?}");
    assert_eq!(state[0].0, "writing a board field");
    assert_eq!(
        state[0].1["input"]["value"],
        json!({"singleSelectOptionId": "OPT-done"})
    );
    assert_eq!(state[1].0, "updating an issue");
    assert_eq!(members(&state[1].1), ["stateInput"]);

    let priority = of(5);
    assert_eq!(priority.len(), 1, "{priority:#?}");
    assert_eq!(priority[0].0, "writing a board field");
    assert_eq!(priority[0].1["input"]["fieldId"], "FIELD-priority");

    assert_eq!(
        written.len(),
        5,
        "nothing else was written — no other task, the project or the document: {written:#?}"
    );

    let after = everything(&plan, &at);
    let changed = |source: &str, member: &str| {
        let mut unchanged = after[source].clone();
        unchanged[member] = before[source][member].clone();
        assert_ne!(
            after[source][member], before[source][member],
            "{source}'s {member}"
        );
        unchanged
    };
    assert_eq!(changed("T-1", "body"), before["T-1"]);
    assert_eq!(after["T-2"]["title"], "The second step, renamed");
    assert_eq!(changed("T-2", "title"), before["T-2"]);
    assert_eq!(after["T-3"]["state"], "CLOSED");
    assert_eq!(after["T-3"]["status"], "Done");
    let mut closed = changed("T-3", "state");
    closed["status"] = before["T-3"]["status"].clone();
    assert_eq!(closed, before["T-3"]);
    assert_eq!(after["T-5"]["priority"], "High");
    assert_eq!(changed("T-5", "priority"), before["T-5"]);
    for source in at.keys() {
        if !["T-1", "T-2", "T-3", "T-5"].contains(&source.as_str()) {
            assert_eq!(after[source], before[source], "{source} is as it was");
        }
    }
    assert_eq!(at.len(), TASKS + 2);
}
