//! Where a plan's counterparts are, recorded beside their links, through the binary.
//!
//! A copy records on each item it copied, beside the `onetaskgraph.copies` link it always
//! wrote, the location the board gave the counterpart, at `onetaskgraph.copy_locations`. A
//! document copy then points each of its references at that location without reading the
//! task it names. It does so only while the entry names exactly what the link names: a link
//! with no location beside it, which an older release wrote, is followed by reading its
//! counterpart, as is an entry a link no longer stands beside. And a document already sitting
//! under the project its project's link names is taken as filed there without the project
//! being read, while one a person moved under another project is put back.

use super::asset_board::Call;
use super::plan_copy::{
    Counterparts, Plan, SMALL, TASKS, counterparts, holds_the_plan, item_reads, success,
};
use serde_json::{Value, json};

impl Plan {
    /// The metadata the folder reports for its task `n`, read through the binary.
    fn task_metadata(&self, n: usize) -> Value {
        success(&self.run(&["task", "show", &format!("notes:T-{n}"), "--json"]))["items"][0]["item"]
            ["metadata"]
            .clone()
    }

    /// Replace one line of task `n`'s file, keeping every other byte.
    fn rewrite_task_line(&self, n: usize, from: &str, to: &str) {
        let path = self.root.join(format!("tasks/T-{n}.md"));
        let held = std::fs::read_to_string(&path).unwrap();
        assert!(held.contains(from), "T-{n} holds {from:?}: {held}");
        std::fs::write(&path, held.replacen(from, to, 1)).unwrap();
    }
}

/// The ids of the plan's tasks on the board that the item reads among `calls` name.
fn tasks_read(calls: &[Call], at: &Counterparts) -> std::collections::BTreeSet<String> {
    let tasks: std::collections::BTreeSet<String> =
        (1..=TASKS).map(|n| at[&format!("T-{n}")].clone()).collect();
    item_reads(calls)
        .into_iter()
        .flat_map(|(_, ids)| ids)
        .filter(|id| tasks.contains(id))
        .collect()
}

/// The `onetaskgraph.copy_locations` line the folder writes for task `n`'s counterpart `id`.
fn located_line(id: &str) -> String {
    format!(
        "{{\"board\":{{\"id\":\"board:{id}\",\"location\":{{\"url\":\"https://example.invalid/{id}\"}}}}}}"
    )
}

#[test]
fn a_first_copy_records_where_each_counterpart_is_and_its_document_copy_reads_no_task() {
    let plan = Plan::new(SMALL, Vec::new());
    let (project, _) = plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    let (document, calls) =
        plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let at = counterparts(&project, &document);
    holds_the_plan(&plan, &at, None);
    assert_eq!(
        tasks_read(&calls, &at),
        std::collections::BTreeSet::new(),
        "the document copy read a task its references name"
    );
    for n in 1..=TASKS {
        let id = &at[&format!("T-{n}")];
        let metadata = plan.task_metadata(n);
        // The link is exactly what it always was, so a release that knows only it reads it as
        // it always did.
        assert_eq!(
            metadata["onetaskgraph.copies"],
            json!({"board": format!("board:{id}")}),
            "T-{n}'s link"
        );
        assert_eq!(
            metadata["onetaskgraph.copy_locations"],
            json!({"board": {"id": format!("board:{id}"),
                             "location": {"url": format!("https://example.invalid/{id}")}}}),
            "T-{n}'s location, as its create answered it"
        );
    }
}

#[test]
fn a_link_with_no_location_beside_it_is_followed_by_reading_its_counterpart() {
    // What an older release left: links and no locations. The references still land on each
    // task's counterpart, read by the document copy, and that copy records nothing on the
    // tasks — only a copy of a task records where it is.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    for n in 1..=TASKS {
        let id = &at[&format!("T-{n}")];
        let path = plan.root.join(format!("tasks/T-{n}.md"));
        let held = std::fs::read_to_string(&path).unwrap();
        let without: String = held
            .lines()
            .filter(|line| !line.contains("onetaskgraph.copy_locations"))
            .map(|line| format!("{line}\n"))
            .collect();
        assert_ne!(without, held, "T-{n} recorded where {id} is: {held}");
        std::fs::write(&path, without).unwrap();
        let metadata = plan.task_metadata(n);
        assert_eq!(
            metadata["onetaskgraph.copies"],
            json!({"board": format!("board:{id}")})
        );
        assert_eq!(metadata.get("onetaskgraph.copy_locations"), None);
    }
    let (_, calls) = plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let expected: std::collections::BTreeSet<String> =
        (1..=TASKS).map(|n| at[&format!("T-{n}")].clone()).collect();
    assert_eq!(
        tasks_read(&calls, &at),
        expected,
        "each linked task is read for where it is"
    );
    holds_the_plan(&plan, &at, None);
}

#[test]
fn a_location_its_link_does_not_name_is_ignored() {
    // T-1's entry names another item than its link does, and points somewhere else: the link
    // wins, T-1's counterpart is read, and the reference points at it.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let id = &at["T-1"];
    plan.rewrite_task_line(
        1,
        &located_line(id),
        "{\"board\":{\"id\":\"board:SOMEWHERE-ELSE\",\"location\":{\"url\":\"https://elsewhere.invalid/nothing\"}}}",
    );
    let (report, calls) =
        plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    assert_eq!(
        tasks_read(&calls, &at),
        std::collections::BTreeSet::from([id.clone()]),
        "only the task whose entry its link does not stand beside is read"
    );
    assert!(
        !report.to_string().contains("elsewhere.invalid"),
        "{report}"
    );
    holds_the_plan(&plan, &at, None);
}

#[test]
fn a_document_already_under_its_linked_project_is_filed_without_reading_the_project() {
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let (_, calls) = plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let read: std::collections::BTreeSet<String> = item_reads(&calls)
        .into_iter()
        .flat_map(|(_, ids)| ids)
        .collect();
    assert_eq!(
        read,
        std::collections::BTreeSet::from([at["D"].clone()]),
        "the document copy reads its own counterpart and nothing else"
    );
    assert!(calls.iter().all(|call| !call.bytes.is_empty()));
    holds_the_plan(&plan, &at, None);
}

#[test]
fn a_document_a_person_moved_under_another_project_is_put_back() {
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.github.file_under(&at["D"], Some("BG-10"));
    assert_eq!(
        plan.github.item(&at["D"]).unwrap()["parent"],
        json!("BG-10")
    );
    let (_, calls) = plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let read: std::collections::BTreeSet<String> = item_reads(&calls)
        .into_iter()
        .flat_map(|(_, ids)| ids)
        .collect();
    assert!(
        read.contains(&at["P"]),
        "the project its link names is read to confirm it: {read:?}"
    );
    holds_the_plan(&plan, &at, None);
}
