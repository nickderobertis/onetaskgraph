//! What a copy of the generated plan writes onto the loopback board, through the binary.
//!
//! **A re-copy writes only the members that changed**, each in the one mutation that carries
//! it, and nothing at all for an item that did not change — so a person's edit of a member it
//! does not change, made between its read and its write, is kept. The board records every
//! request it is sent, so a re-copy writing anything it did not have to shows up here as a
//! mutation naming an item nothing changed.
//!
//! **A first copy files each item by its own create** and sends the rest of what every item
//! needs together once they all exist; one of those aliased writes refused part way takes back
//! every item the copy created.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Counterparts, Plan, SMALL, TASKS, counterparts, operation};
use onetaskgraph_e2e_support::fixtures::AliasAnswer;
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
    // A request carrying both field values and edges says which of each it sends apart.
    let send = match prefix {
        "field" if variables.get("sendField0").is_some() => "sendField",
        "block" if variables.get("sendBlock0").is_some() => "sendBlock",
        _ => "send",
    };
    (0..onetaskgraph_github_projects::WRITE_BATCH)
        .filter(|slot| variables[format!("{send}{slot}")] == true)
        .map(|slot| variables[format!("{prefix}{slot}")].clone())
        .collect()
}

#[test]
fn a_first_copy_files_each_item_by_its_create_and_sends_its_other_writes_together() {
    // Every item is created filed on the board and under its project by its own `createIssue`
    // — the project's alone, the eight tasks' in one aliased request — and the rest of what
    // each needs lands once all of them exist: their board items read in one request, their
    // `Status` and origin values and their ten edges in aliased writes of at most nineteen —
    // the eighteen values and the first edge in one, the nine edges left in the next. The
    // document is the
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
            "writing board fields and dependencies of the items a copy created",
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
    let values = sent(&written[2].1, "field");
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
    assert_eq!(
        (
            sent(&written[2].1, "block").len(),
            sent(&written[3].1, "block").len()
        ),
        (1, 9)
    );
    let document_create = &written[4].1["input"];
    assert_eq!(document_create["projectV2Ids"], board);
    assert_eq!(document_create["parentIssueId"], json!(at["P"]));
    let document_fields = sent(&written[5].1, "field");
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
        .filter(|(name, _)| name.starts_with("writing board fields"))
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
    refused_after_landing(&Plan::new(SMALL, Vec::new()), alias);
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

/// The waits a copy's process made for GitHub to file the items it created on the board.
fn filing_waits(plan: &Plan) -> Vec<std::time::Duration> {
    plan.clock
        .slept()
        .into_iter()
        .filter(|slept| slept.cause.as_deref() == Some("board-filing"))
        .map(|slept| slept.length)
        .collect()
}

#[test]
fn a_first_copy_waits_for_github_to_file_the_items_it_created_and_then_lands_the_plan() {
    // GitHub adds a created issue to the board its create names after answering, and twice
    // more the read of where it is answers that it is nowhere yet: the copy waits a second,
    // then two, reads again, and lands the plan whole.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.file_created_items_after(2);
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
    assert_eq!(
        filing_waits(&plan),
        [1, 2].map(std::time::Duration::from_secs),
        "the waits for GitHub to file what the copy created"
    );
}

#[test]
fn a_first_copy_whose_items_github_files_by_the_last_read_lands_the_plan() {
    // GitHub files them in time for the sixth and last read, after every wait: that read is
    // the one that finds them, and the copy lands the plan rather than refusing.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.file_created_items_after(5);
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
    assert_eq!(
        filing_waits(&plan),
        [1, 2, 4, 8, 16].map(std::time::Duration::from_secs),
        "the waits for GitHub to file what the copy created"
    );
}

#[test]
fn a_first_copy_whose_items_github_never_files_takes_back_every_item_it_created() {
    // Never filed: the copy waits one, two, four, eight and sixteen seconds, reading again after
    // each, then files each still absent itself — GitHub answering that it is there already —
    // reads once more, and still finding none refuses naming what GitHub never added, and takes
    // back every issue it created.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.file_created_items_after(usize::MAX);
    let lease = plan.clock.lease(0);
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    lease.wait_for_detach();
    drop(lease);
    assert_ne!(output.status.code(), Some(0), "the copy was refused");
    let said = onetaskgraph_e2e_support::common::stderr(&output);
    assert!(
        said.contains("after 31 seconds and adding them to it, still answers them on no item")
            && said.contains("run the copy again once GitHub has caught up"),
        "{said}"
    );
    assert_eq!(
        filing_waits(&plan),
        [1, 2, 4, 8, 16].map(std::time::Duration::from_secs),
        "the waits for GitHub to file what the copy created"
    );
    left_nothing(&plan, TASKS + 1);
}

#[test]
fn a_batched_create_refused_part_way_whose_cleanup_github_refuses_names_what_is_still_there() {
    // The fourth task's create fails after the three before it created their issues, and
    // GitHub then refuses to delete those three: the copy is refused naming both failures and
    // each issue still in the repository, which nothing else could name, and the project the
    // copy created first, whose delete GitHub accepts, is taken back.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.fail_field_alias("create3");
    // The three are taken back together, in one request of aliased deletes.
    plan.github.refuse_once("deleteIssue(input:$delete");
    let lease = plan.clock.lease(0);
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    lease.wait_for_detach();
    drop(lease);
    assert_ne!(output.status.code(), Some(0), "the copy was refused");
    let said = onetaskgraph_e2e_support::common::stderr(&output);
    let left = ["ISSUE-2", "ISSUE-3", "ISSUE-4"];
    assert!(
        said.contains("create3")
            && said.contains("GitHub then refused to take back ISSUE-2, ISSUE-3, ISSUE-4")
            && said.contains("delete them before writing again"),
        "{said}"
    );
    for issue in left {
        assert!(
            plan.github.holds_issue(issue),
            "{issue} is still there, as said"
        );
    }
    assert!(
        !plan.github.holds_issue("ISSUE-1"),
        "the project was taken back"
    );
}

#[test]
fn a_first_copy_closes_a_done_task_it_created_once_its_option_is_written() {
    // A task the plan holds `done` is created open, filed like the rest, and closed completed
    // in the held-back writes, after its `Status` option — in one request of closes.
    let plan = Plan::new(SMALL, Vec::new());
    plan.edit_task(2, "status: todo", "status: done");
    let (project, document, calls) = plan.copy();
    let at = counterparts(&project, &document);
    let held = plan.github.item(&at["T-2"]).expect("T-2's issue");
    assert_eq!(
        (
            held["state"].clone(),
            held["reason"].clone(),
            held["status"].clone()
        ),
        (json!("CLOSED"), json!("COMPLETED"), json!("Done")),
        "{held:#}"
    );
    let written = mutations(&calls);
    let closes: Vec<Value> = written
        .iter()
        .filter(|(name, _)| name == "closing the issues a copy created")
        .flat_map(|(_, variables)| sent(variables, "close"))
        .collect();
    assert_eq!(closes.len(), 1, "{written:#?}");
    assert_eq!(closes[0]["id"], json!(at["T-2"]));
    for n in [1, 3, 8] {
        let open = plan.github.item(&at[&format!("T-{n}")]).expect("an issue");
        assert_eq!(open["state"], json!("OPEN"), "T-{n}");
    }
}

#[test]
fn a_first_copy_whose_close_is_refused_takes_back_every_item_it_created() {
    let plan = Plan::new(SMALL, Vec::new());
    plan.edit_task(2, "status: todo", "status: done");
    refused_after_landing(&plan, "close0");
}

#[test]
fn a_held_back_write_answered_malformed_takes_back_every_item_it_created() {
    // Each of these landed, and GitHub's answer does not say so of the item it was asked to
    // write — an alias missing, or answered naming another item — so the copy cannot vouch
    // for it, refuses, and takes back every issue it created.
    for (alias, answer) in [
        ("field4", AliasAnswer::Missing),
        ("field4", AliasAnswer::NamingAnother),
        ("block2", AliasAnswer::NamingAnother),
        ("block2", AliasAnswer::NamingAnotherBlocker),
    ] {
        let plan = Plan::new(SMALL, Vec::new());
        plan.github.bend_batch_alias(alias, answer);
        let lease = plan.clock.lease(0);
        let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
        lease.wait_for_detach();
        drop(lease);
        assert_ne!(output.status.code(), Some(0), "{alias} {answer:?}");
        let said = onetaskgraph_e2e_support::common::stderr(&output);
        let expected = match answer {
            AliasAnswer::Missing => format!("with nothing at {alias}"),
            AliasAnswer::NamingAnother | AliasAnswer::NamingAnotherBlocker => {
                format!("at {alias} with ANOTHER-ITEM")
            }
        };
        assert!(said.contains(&expected), "{alias} {answer:?}: {said}");
        left_nothing(&plan, TASKS + 1);
    }
}

#[test]
fn a_batched_create_answered_without_one_of_its_issues_leaves_the_board_uncertain_and_says_so() {
    // The fourth task's create lands and its answer leaves it out, with no error beside it:
    // the issue exists under an id this copy was never told. Every issue it was told of is
    // taken back, and the copy is refused naming the board and the plugin method whose answer
    // it could not read, saying to inspect the board for the one it could not name.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github
        .bend_batch_alias("create3", AliasAnswer::Missing);
    let lease = plan.clock.lease(0);
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    lease.wait_for_detach();
    drop(lease);
    assert_ne!(output.status.code(), Some(0), "the copy was refused");
    let said = onetaskgraph_e2e_support::common::stderr(&output);
    assert!(
        said.contains("GitHub issue creation returned no issue")
            && said.contains("write_tasks")
            && said.contains("next: inspect board"),
        "{said}"
    );
    for issue in ["ISSUE-1", "ISSUE-2", "ISSUE-3", "ISSUE-4"] {
        assert!(!plan.github.holds_issue(issue), "{issue} was taken back");
    }
    assert!(
        plan.github.holds_issue("ISSUE-5"),
        "the issue the answer left out is still there, as the copy says it may be"
    );
}

#[test]
fn an_origin_lookup_answering_a_blank_repository_id_is_not_what_a_create_is_sent_under() {
    // The batched read ahead of a plan copy carries the repository each issue is created in.
    // Answered with that id blank, it is refused as malformed rather than kept, so the copy
    // reads the repository on its own and every create names the real one — the board refuses
    // any other — and the whole plan lands.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.blank_repository_id();
    let lease = plan.clock.lease(0);
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    lease.wait_for_detach();
    drop(lease);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        onetaskgraph_e2e_support::common::stderr(&output)
    );
    for n in 1..=TASKS + 1 {
        let issue = format!("ISSUE-{n}");
        assert!(
            plan.github.item(&issue).is_some(),
            "the board holds {issue}"
        );
    }
}

/// A project copy of `plan` with `alias` of its held-back writes refused: refused itself, naming
/// GitHub's own failure, and leaving nothing behind.
fn refused_after_landing(plan: &Plan, alias: &'static str) {
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
        "{alias}: {said}"
    );
    left_nothing(plan, TASKS + 1);
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

/// The plan's project copied onto the board, attached to the clock: what it exited with and
/// said, and every request the board answered for it.
fn project_copied(plan: &Plan) -> (std::process::Output, Vec<Call>) {
    plan.measured_output(&["project", "copy", "notes:P", "--to", "board", "--json"])
}

#[test]
fn a_first_copy_whose_issues_are_never_placed_by_their_create_files_each_itself_and_lands() {
    // GitHub never places the issues the creates filed on the board: every read of where they
    // are answers them on none. Once its reads are spent, the copy files each with
    // `addProjectV2ItemById` itself, one write each, and lands the plan whole.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github
        .place_filed_issues_after(vec![std::time::Duration::from_secs(3_600)], || {
            std::time::Duration::ZERO
        });
    let (output, calls) = project_copied(&plan);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        onetaskgraph_e2e_support::common::stderr(&output)
    );
    let added = calls
        .iter()
        .filter(|call| operation(call) == "adding an issue to the board")
        .count();
    assert_eq!(added, TASKS + 1, "one add for each issue never placed");
    assert_eq!(
        filing_waits(&plan),
        [1, 2, 4, 8, 16].map(std::time::Duration::from_secs)
    );
    let (_, document, _) = plan.copy();
    let report: Value =
        serde_json::from_str(&onetaskgraph_e2e_support::common::stdout(&output)).unwrap();
    let at = counterparts(&report, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
}

#[test]
fn a_first_copy_told_its_issue_is_filed_already_reads_it_again_and_lands() {
    // GitHub places the issues only after every read the copy makes of them: the add the copy
    // then sends is refused "Content already exists in this project", which says they are
    // placed, and one more read finds each one's item. Nothing is filed twice.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.file_created_items_after(6);
    let (output, calls) = project_copied(&plan);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        onetaskgraph_e2e_support::common::stderr(&output)
    );
    let reads = calls
        .iter()
        .filter(|call| operation(call) == "reading the board items of the issues a copy created")
        .count();
    assert_eq!(reads, 7, "six reads, the refused adds, and one more read");
    let (_, document, _) = plan.copy();
    let report: Value =
        serde_json::from_str(&onetaskgraph_e2e_support::common::stdout(&output)).unwrap();
    let at = counterparts(&report, &document);
    super::plan_copy::holds_the_plan(&plan, &at, None);
}

#[test]
fn a_first_copy_whose_own_add_to_the_board_is_refused_takes_back_every_item_it_created() {
    let plan = Plan::new(SMALL, Vec::new());
    plan.github
        .place_filed_issues_after(vec![std::time::Duration::from_secs(3_600)], || {
            std::time::Duration::ZERO
        });
    plan.github
        .refuse_once("addProjectV2ItemById(input:$input)");
    let (output, _) = project_copied(&plan);
    assert_ne!(output.status.code(), Some(0), "the copy was refused");
    left_nothing(&plan, TASKS + 1);
}

#[test]
fn a_batched_cleanup_github_does_not_confirm_whole_names_the_issue_it_did_not_confirm() {
    // The fourth task's create fails after the three before it, and of the three deletes that
    // take those back, GitHub's answer leaves the second out: the copy names that issue alone as
    // what may still be there, and the two it confirmed are gone.
    let plan = Plan::new(SMALL, Vec::new());
    plan.github.fail_field_alias("create3");
    plan.github
        .bend_batch_alias("delete1", AliasAnswer::Missing);
    let (output, _) = project_copied(&plan);
    assert_ne!(output.status.code(), Some(0), "the copy was refused");
    let said = onetaskgraph_e2e_support::common::stderr(&output);
    assert!(
        said.contains("GitHub then refused to take back ISSUE-3")
            && !said.contains("ISSUE-2,")
            && !said.contains("ISSUE-4"),
        "{said}"
    );
    for gone in ["ISSUE-2", "ISSUE-4"] {
        assert!(!plan.github.holds_issue(gone), "{gone} was taken back");
    }
}
