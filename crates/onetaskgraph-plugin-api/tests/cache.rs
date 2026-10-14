//! What a write's response showed of its task, as a journal carries it: a whole task always at
//! a version, and the shape it is written in the one it is read back from.

use onetaskgraph_plugin_api::{Shown, Task, WholeTask};
use serde_json::json;

fn task(version: Option<&str>) -> Task {
    let mut task: Task = serde_json::from_value(json!({
        "id": "T-1", "title": "t", "content": "body", "status": {"category": "todo", "name": "todo"},
        "labels": [], "project": null, "url": null, "created_at": null, "updated_at": null
    }))
    .expect("a task");
    task.version = version.map(str::to_owned);
    task
}

#[test]
fn a_whole_task_is_built_only_at_a_version_and_reads_back_as_it_was_written() {
    assert!(
        WholeTask::new(task(None), Vec::new(), Vec::new(), None).is_none(),
        "a task without a version is no whole task a cache can key"
    );
    let whole = WholeTask::new(
        task(Some("gh2|v")),
        Vec::new(),
        Vec::new(),
        Some(Vec::new()),
    )
    .expect("a versioned task is whole");
    assert_eq!(whole.version(), "gh2|v");
    let shown = Shown::Whole {
        task: Box::new(whole),
    };
    let wire = serde_json::to_value(&shown).expect("it renders");
    let back: Shown = serde_json::from_value(wire.clone()).expect("it reads back");
    assert_eq!(back, shown, "what is written is what is read");
    assert_eq!(back.version(), Some("gh2|v"));

    let mut unversioned = wire;
    unversioned["task"]["task"]
        .as_object_mut()
        .expect("the task")
        .remove("version");
    let refused = serde_json::from_value::<Shown>(unversioned).expect_err("refused");
    assert!(
        refused.to_string().contains("carries the version"),
        "{refused}"
    );
}

#[test]
fn a_coverage_member_a_value_does_not_name_reads_as_uncovered_with_a_reason() {
    use onetaskgraph_plugin_api::{
        Coverage, DocumentVersionCoverage, ProjectVersionCoverage, VersionCoverage,
    };
    let uncovered = |coverage: &Coverage| {
        coverage
            .reason()
            .is_some_and(|reason| !reason.trim().is_empty())
    };
    // A task's coverage written before `forward_edges` existed keeps what it declared and
    // vouches for no edge.
    let task: VersionCoverage = serde_json::from_value(json!({
        "fields": {"coverage": "covered"},
        "comments": {"coverage": "covered"}
    }))
    .expect("a coverage");
    assert_eq!(task.fields, Coverage::Covered);
    assert_eq!(task.comments, Coverage::Covered);
    assert!(uncovered(&task.forward_edges), "{task:?}");

    let project: ProjectVersionCoverage =
        serde_json::from_value(json!({})).expect("a project coverage");
    assert!(uncovered(&project.fields) && uncovered(&project.forward_edges));
    let document: DocumentVersionCoverage =
        serde_json::from_value(json!({"fields": {"coverage": "covered"}}))
            .expect("a document coverage");
    assert_eq!(document.fields, Coverage::Covered);
    assert!(uncovered(&document.assets));
}

#[test]
fn whole_and_none_keep_their_meaning_across_every_member() {
    use onetaskgraph_plugin_api::{
        Coverage, DocumentVersionCoverage, ProjectVersionCoverage, VersionCoverage,
    };
    let task = VersionCoverage::whole();
    assert!(
        [&task.fields, &task.comments, &task.forward_edges]
            .iter()
            .all(|part| **part == Coverage::Covered)
    );
    let none = VersionCoverage::none("no token");
    assert!(
        [&none.fields, &none.comments, &none.forward_edges]
            .iter()
            .all(|part| part.reason() == Some("no token"))
    );
    let project = ProjectVersionCoverage::none("no token");
    assert_eq!(project.forward_edges.reason(), Some("no token"));
    assert_eq!(ProjectVersionCoverage::whole().fields, Coverage::Covered);
    let document = DocumentVersionCoverage::none("no token");
    assert_eq!(document.assets.reason(), Some("no token"));
    assert_eq!(DocumentVersionCoverage::whole().assets, Coverage::Covered);
}
