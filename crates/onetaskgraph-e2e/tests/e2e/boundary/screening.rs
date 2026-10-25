//! How a copy puts its writes to the caller's check, read off the record of every boundary
//! command the store ran: the released onevcs commands behind a recorder, against an onevcs
//! home of synthetic identities, with every hosted backend a loopback stand-in.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use onetaskgraph_e2e_support::onevcs::{BoundaryCall, BoundaryRecord};
use serde_json::{Value, json};

use super::{OnevcsHome, PLAIN, Sandbox, Store, folder, landed_ids, registered, tree};
use crate::common::stderr;
use crate::fixtures::linear_rate_limited;

/// The public plan: a project of three tasks in `draft`, one naming a public repository, two
/// depending on the one before them, copied whole to the public `site`.
fn plan(store: &Store) {
    store.record(
        "draft",
        "projects",
        "goal",
        "title: Goal\nstatus: todo",
        PLAIN,
    );
    store.record(
        "draft",
        "tasks",
        "library",
        &format!(
            "title: Library\nstatus: todo\nproject: goal\nrepositories: [{}]",
            super::OPEN
        ),
        PLAIN,
    );
    store.record(
        "draft",
        "tasks",
        "consumer",
        "title: Consumer\nstatus: todo\nproject: goal\ndepends_on: [library]",
        PLAIN,
    );
    store.record(
        "draft",
        "tasks",
        "rollout",
        &format!("title: Rollout\nstatus: todo\nproject: goal\nrepositories: [{}]\ndepends_on: [consumer]", super::OPEN),
        PLAIN,
    );
}

/// A store screening its writes through a recorder of the boundary commands, holding the
/// public folders `draft` and `site` and, unrelated to the plan, the private Linear source
/// `ledger`, whose every request is refused as rate limited and counted.
fn screened() -> (
    Store,
    BoundaryRecord,
    std::sync::Arc<std::sync::Mutex<usize>>,
) {
    let sandbox = Sandbox::new();
    let mut home = OnevcsHome::new(&sandbox);
    registered(&mut home);
    let record = home.recorded(&sandbox);
    let (ledger, asked) = linear_rate_limited(&sandbox);
    let extra = json!({
        "draft": folder(&sandbox, "draft", "public"),
        "ledger": {"plugin": "linear", "config": ledger, "visibility": "private"},
    });
    let policy = record.write_policy().clone();
    let store = Store::configured(sandbox, Some(home), Some(policy), extra);
    (store, record, asked)
}

/// When `site`'s first file was last written, since the Unix epoch.
fn first_written(store: &Store) -> Option<Duration> {
    tree(&store.folder("site"))
        .into_keys()
        .filter_map(|path| std::fs::metadata(store.folder("site").join(path)).ok())
        .filter_map(|meta| meta.modified().ok())
        .map(|at| at.duration_since(UNIX_EPOCH).unwrap_or_default())
        .min()
}

fn checks(calls: &[BoundaryCall]) -> Vec<&BoundaryCall> {
    calls
        .iter()
        .filter(|call| call.command == "check")
        .collect()
}

#[test]
fn a_public_copy_screens_its_writes_together_before_the_first_and_never_asks_an_unrelated_private_source()
 {
    let (store, record, ledger) = screened();
    plan(&store);
    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        *ledger.lock().unwrap(),
        0,
        "the private Linear source nothing of the plan names was never asked"
    );
    assert!(
        !stderr(&output).contains("withheld"),
        "a copy that withholds nothing says nothing of it: {}",
        stderr(&output)
    );
    let first = record.calls();
    // One inspect of the one repository the plan names, however many records name it.
    let inspected: Vec<&Value> = first
        .iter()
        .filter(|call| call.command == "inspect")
        .map(|call| &call.input["repository"])
        .collect();
    assert_eq!(inspected, [&json!(super::OPEN)], "{first:?}");
    // One check, of every record of the plan together, finished before the first write.
    let union = checks(&first);
    assert_eq!(union.len(), 1, "{first:?}");
    let carried = union[0].input["metadata"].as_array().unwrap();
    for title in ["Goal", "Library", "Consumer", "Rollout"] {
        assert!(carried.contains(&json!(title)), "{title} in {carried:?}");
    }
    let written = first_written(&store).expect("the copy wrote the plan");
    assert!(
        started < union[0].finished && union[0].finished <= written,
        "the check finished before the first write"
    );

    // The next command is screened again: its own inspect and its own check.
    store.record(
        "draft",
        "tasks",
        "rollout",
        &format!("title: Rollout, revised\nstatus: todo\nproject: goal\nrepositories: [{}]\ndepends_on: [consumer]", super::OPEN),
        PLAIN,
    );
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let second = record.calls()[first.len()..].to_vec();
    assert_eq!(
        second
            .iter()
            .filter(|call| call.command == "inspect")
            .count(),
        1,
        "{second:?}"
    );
    let rechecked = checks(&second);
    assert_eq!(rechecked.len(), 1, "{second:?}");
    assert!(
        rechecked[0].input["metadata"]
            .as_array()
            .unwrap()
            .contains(&json!("Rollout, revised")),
        "the changed exposure is screened again"
    );
    assert_eq!(*ledger.lock().unwrap(), 0);
}

#[test]
fn a_refused_union_is_checked_record_by_record_to_name_the_one_refused_and_nothing_is_written() {
    let (store, record, _) = screened();
    plan(&store);
    store.record(
        "draft",
        "tasks",
        "consumer",
        "title: Consumer\nstatus: todo\nproject: goal\ndepends_on: [library]",
        super::A_TEXT,
    );
    let (kind, said) = store.refused(&["project", "copy", "draft:goal", "--to", "site"]);
    assert_eq!(kind, "boundary-refused", "{said}");
    assert!(
        said.contains("draft:consumer"),
        "it names the record refused: {said}"
    );
    assert!(!said.contains("quietharbor"), "neutrally: {said}");
    assert!(
        tree(&store.folder("site")).is_empty(),
        "nothing was written"
    );
    let calls = record.calls();
    let checked = checks(&calls);
    // The union first, refused; then record by record, until the one refused.
    assert_eq!(checked[0].exit, Some(1), "{calls:?}");
    assert!(checked.len() > 1, "{calls:?}");
    assert_eq!(checked.last().unwrap().exit, Some(1));
    assert!(
        checked[1..checked.len() - 1]
            .iter()
            .all(|call| call.exit == Some(0)),
        "{calls:?}"
    );
}

#[test]
fn a_dependency_on_an_item_of_an_unavailable_private_source_is_withheld_and_said_on_standard_error()
{
    // The consumer depends on an item of `ledger`, a source declared private which answers
    // every request rate limited: the copy lands, the public destination carries no edge or id
    // of it, and standard error names the edge it left out and why.
    let (store, _, _) = screened();
    plan(&store);
    store.record(
        "draft",
        "tasks",
        "consumer",
        "title: Consumer\nstatus: todo\nproject: goal\ndepends_on: [library, {id: \"ledger:LED-1\", item: task}]",
        PLAIN,
    );
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let landed = landed_ids(&String::from_utf8_lossy(&output.stdout));
    let consumer = &landed["draft:consumer"];
    assert!(
        stderr(&output).contains(
            "onetaskgraph: withheld draft:consumer's dependency on ledger:LED-1 from site: its \
             source ledger is declared private, and its source named no tracker an opaque \
             reference to it could point into\n",
        ),
        "{}",
        stderr(&output)
    );
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("ledger") && !text.contains("LED-1"),
            "{path} names the private item: {text}"
        );
    }
    assert!(consumer.starts_with("site:"), "{consumer}");
}
