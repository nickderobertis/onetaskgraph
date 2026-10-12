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
