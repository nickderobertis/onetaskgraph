//! What a copy of the generated plan writes onto the loopback board, through the binary.
//!
//! **A re-copy writes only the members that changed**, each in the one mutation that carries
//! it, and nothing at all for an item that did not change — so a person's edit of a member it
//! does not change, made between its read and its write, is kept. The base this change starts
//! from sent one `updateIssue` per project and task on every re-copy — the timing spike's no-op
//! writes — carrying a title, a body and a state the board already held. The board records
//! every request it is sent, so a re-copy writing anything it did not have to shows up here as
//! a mutation naming an item nothing changed.
//!
//! **A first copy files each item by its own create** and sends the rest of what every item
//! needs together once they all exist, as the timing spike found GitHub accepts; one of those
//! aliased writes refused part way takes back every item the copy created.

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

#[test]
fn a_persons_edit_made_between_the_copys_read_and_its_write_is_kept() {
    // A re-copy after T-4's body changed reads T-4 before it writes it. In between, a person
    // retitles T-4 on the board. The copy changes T-4's body and nothing else, so it writes the
    // body alone, and the person's title is what the board holds once it finishes.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let edited = at["T-4"].clone();
    plan.edit_task(4, "\n---\n", "\n---\nA line added since the last copy.\n");
    let target = edited.clone();
    plan.github.after_request(
        |query, _| query == onetaskgraph_github_projects::graphql::ISSUE_COPY_DETAILS,
        0,
        move |board| board.edit_title(&target, "Retitled by a person"),
    );
    let (_, _, calls) = plan.copy();
    let written = mutations(&calls);
    assert_eq!(written.len(), 1, "{written:#?}");
    assert_eq!(members(&written[0].1), ["body"]);
    let held = held(&plan, &edited);
    assert_eq!(
        held["title"], "Retitled by a person",
        "the person's edit is kept"
    );
    assert!(
        held["body"]
            .as_str()
            .unwrap()
            .contains("A line added since the last copy."),
        "the copy's own change landed"
    );
}

/// What a request of a copy's aliased writes sent, each slot's input in the order it sent them.
fn sent(variables: &Value, prefix: &str) -> Vec<Value> {
    (0..onetaskgraph_github_projects::MUTATION_BATCH)
        .filter(|slot| variables[format!("send{slot}")] == true)
        .map(|slot| variables[format!("{prefix}{slot}")].clone())
        .collect()
}

#[test]
fn a_first_copy_files_each_item_by_its_create_and_sends_its_other_writes_together() {
    // Every item is created filed on the board and under its project by its own `createIssue`
    // — the project's alone, the eight tasks' in one aliased request — and the rest of what
    // each needs lands once all of them exist: their board items read in one request, their
    // `Status` and origin values in aliased field writes of at most eleven, and the ten edges
    // in one more. The document is the
    // same in its own process. Nothing is added to the board or filed under a project after its
    // create, and the board holds the plan exactly as a copy always left it.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, calls) = plan.copy();
    let at = counterparts(&project, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
    let written = mutations(&calls);
    let names: Vec<&str> = written.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "creating an issue",
            "creating several issues together",
            "writing board fields of the items a copy created",
            "writing board fields of the items a copy created",
            "recording several dependencies together",
            "creating an issue",
            "writing board fields of the items a copy created",
        ],
        "{written:#?}"
    );
    let board = json!(["PVT-board"]);
    let project_create = &written[0].1["input"];
    assert_eq!(project_create["projectV2Ids"], board);
    assert_eq!(project_create.get("parentIssueId"), None);
    let tasks = sent(&written[1].1, "create");
    assert_eq!(tasks.len(), 8);
    for input in &tasks {
        assert_eq!(input["projectV2Ids"], board);
        assert_eq!(input["parentIssueId"], json!(at["P"]));
    }
    // The project's and every task's `Status` and origin, each value its own aliased mutation:
    // a board left with GitHub's default workflows files a new item with no `Status`.
    let mut values = sent(&written[2].1, "field");
    assert_eq!(values.len(), 11);
    values.extend(sent(&written[3].1, "field"));
    let count = |field: &str| {
        values
            .iter()
            .filter(|value| value["fieldId"] == field)
            .count()
    };
    assert_eq!(
        (count("FIELD-status"), count("FIELD-origin"), values.len()),
        (9, 9, 18),
        "{values:#?}"
    );
    assert_eq!(sent(&written[4].1, "block").len(), 10);
    let document_create = &written[5].1["input"];
    assert_eq!(document_create["projectV2Ids"], board);
    assert_eq!(document_create["parentIssueId"], json!(at["P"]));
    let document_fields = sent(&written[6].1, "field");
    assert_eq!(document_fields.len(), 1);
    assert_eq!(document_fields[0]["value"], json!({"text": "notes:D"}));
    // The board items GitHub adds after each create, read once per process, after every create
    // and before any field.
    let reads: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| {
            operation(call) == "reading the board items of the issues a copy created"
        })
        .map(|(at, _)| at)
        .collect();
    assert_eq!(reads.len(), 2, "{reads:?}");
    let creates = |call: &Call| operation(call).starts_with("creating");
    let fields = |call: &Call| operation(call).starts_with("writing board fields");
    let last_create = calls
        .iter()
        .rposition(|call| creates(call) && call.at < calls[reads[1]].at);
    assert!(last_create.is_some_and(|create| create < reads[1]));
    assert!(
        calls[..reads[0]]
            .iter()
            .filter(|call| creates(call))
            .count()
            == 2
    );
    assert!(calls[..reads[0]].iter().all(|call| !fields(call)));
}

#[test]
fn a_status_a_board_already_files_a_new_item_at_is_not_written_again() {
    // A board whose own workflow files every new item at `Todo`: the copy reads the board item
    // each create made, finds the plan's `Todo` already there, and writes the origin alone.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.file_new_items_without_status(false);
    let (project, document, calls) = plan.copy();
    let at = counterparts(&project, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
    let values: Vec<Value> = mutations(&calls)
        .iter()
        .filter(|(name, _)| name == "writing board fields of the items a copy created")
        .flat_map(|(_, variables)| sent(variables, "field"))
        .collect();
    assert_eq!(values.len(), 10, "{values:#?}");
    assert!(
        values
            .iter()
            .all(|value| value["fieldId"] == "FIELD-origin"),
        "{values:#?}"
    );
}

/// Hold the board to holding nothing of a plan copy that failed: no issue it created, and no
/// link to one at the folder.
fn left_nothing(plan: &Plan, created: usize) {
    for n in 1..=created {
        let issue = format!("ISSUE-{n}");
        assert!(
            plan.github.item(&issue).is_none(),
            "the board holds {issue}"
        );
        assert!(
            !plan.github.holds_issue(&issue),
            "the repository holds {issue}"
        );
    }
    for file in ["projects/P.md", "tasks/T-1.md", "tasks/T-8.md"] {
        assert!(
            !std::fs::read_to_string(plan.root.join(file))
                .unwrap()
                .contains("onetaskgraph.copies"),
            "{file} links to an item that is gone"
        );
    }
}

/// A project copy of the plan with `alias` of a copy's batched writes refused: refused itself,
/// naming GitHub's own failure, and leaving nothing behind.
fn refused_part_way(alias: &'static str) {
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.fail_field_alias(alias);
    let lease = plan.clock.lease(0);
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    lease.wait_for_detach();
    drop(lease);
    assert_ne!(
        output.status.code(),
        Some(0),
        "{alias}: the copy was refused"
    );
    let said = onetaskgraph_e2e_support::common::stderr(&output);
    assert!(
        said.contains("Something went wrong while executing your query") && said.contains(alias),
        "{alias}: the failure GitHub reported is what the caller is told: {said}"
    );
    left_nothing(&plan, TASKS + 1);
}

#[test]
fn a_first_copy_whose_batched_edge_is_refused_takes_back_every_item_it_created() {
    // The fourth edge of the one request carrying all ten fails after the three before it ran.
    refused_part_way("block3");
}

#[test]
fn a_first_copy_whose_batched_create_is_refused_part_way_takes_back_every_item_it_created() {
    // The fourth task's create fails after the three before it — and the project's, sent on
    // its own first — created their issues: every one of them is taken back.
    refused_part_way("create3");
}

#[test]
fn with_its_cache_on_an_unchanged_recopy_writes_nothing_and_its_document_copy_reads_no_plan_item() {
    // A plan copy whose source has not changed since the last one: the project copy finds
    // every item as the source says it should be and writes nothing, so every token its reads
    // left in the cache stays the board's token, and the document copy that follows it is
    // answered by those entries — it reads no item of the plan by id.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    // The first copy wrote each record's link into its source file, which moved every source's
    // token: the second copy is the one that settles, and the third copies nothing changed.
    plan.copy();
    let (_, project_calls) =
        plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    assert_eq!(
        mutations(&project_calls),
        Vec::new(),
        "the project copy writes nothing"
    );
    let (_, document_calls) =
        plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    assert_eq!(
        mutations(&document_calls),
        Vec::new(),
        "the document copy writes nothing"
    );
    let plan_ids = super::plan_copy::plan_ids(&at);
    let read: Vec<(String, std::collections::BTreeSet<String>)> =
        super::plan_copy::item_reads(&document_calls)
            .into_iter()
            .filter(|(_, ids)| ids.iter().any(|id| plan_ids.contains(id)))
            .collect();
    assert_eq!(
        read,
        Vec::new(),
        "the document copy read an item of the plan by id"
    );
}
