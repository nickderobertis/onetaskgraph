//! How a copy puts its writes to the caller's check, read off the record of every boundary
//! command the store ran: the released onevcs commands behind a recorder, against an onevcs
//! home of synthetic identities, with every hosted backend a loopback stand-in.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use onetaskgraph_e2e_support::onevcs::{BoundaryCall, BoundaryCommand, BoundaryRecord};
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
        .filter(|call| call.command == BoundaryCommand::Check)
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
        .filter(|call| call.command == BoundaryCommand::Inspect)
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
            .filter(|call| call.command == BoundaryCommand::Inspect)
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

/// A linking caller's policy whose answer about every repository it is asked is `public` until
/// it is told otherwise, and which counts what it was asked: each repository's visibility, and
/// each check, which it passes.
#[derive(Default)]
struct Counted {
    private: std::sync::atomic::AtomicBool,
    inspected: std::sync::atomic::AtomicUsize,
    checked: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Counted {
    async fn repository_visibility(
        &self,
        _: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        use std::sync::atomic::Ordering;
        self.inspected.fetch_add(1, Ordering::SeqCst);
        Ok(if self.private.load(Ordering::SeqCst) {
            onetaskgraph_core::boundary::RepositoryVisibility::Private
        } else {
            onetaskgraph_core::boundary::RepositoryVisibility::Public
        })
    }

    async fn check_public_write(
        &self,
        _: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        self.checked
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(onetaskgraph_core::boundary::WriteVerdict::Pass)
    }
}

#[tokio::test]
async fn what_a_policy_answered_is_held_for_one_command_and_asked_again_after_end_command() {
    use std::sync::atomic::Ordering;
    let store = Store::new(None, json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(Counted::default());
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    // Two writes of one command naming one repository, with one text: the repository is asked
    // about once and the text checked once.
    for _ in 0..2 {
        engine
            .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
            .await
            .expect("a public repository's task");
    }
    assert_eq!(policy.inspected.load(Ordering::SeqCst), 1);
    assert_eq!(policy.checked.load(Ordering::SeqCst), 1);
    // A changed text in the same command is checked again.
    engine
        .create_task(&super::task_on_site("Another plain step.", &[super::OPEN]))
        .await
        .expect("a public repository's task");
    assert_eq!(policy.checked.load(Ordering::SeqCst), 2);
    // The repository turns private. The next command asks again and is refused, writing
    // nothing.
    policy.private.store(true, Ordering::SeqCst);
    engine.end_command().await.expect("the command ends");
    let before = tree(&store.folder("site"));
    let refused = engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect_err("a task of a repository now private");
    assert_eq!(super::kind_of(&refused), "not-private-destination");
    assert_eq!(policy.inspected.load(Ordering::SeqCst), 2);
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
}

#[tokio::test]
async fn a_replaced_policy_answers_for_itself_though_the_command_has_not_ended() {
    use std::sync::atomic::Ordering;
    let store = Store::new(None, json!({}));
    store.public_project_p();
    let first = std::sync::Arc::new(Counted::default());
    let engine = super::linked_engine(&store).with_write_policy(first.clone());
    engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect("a public repository's task");
    // The same engine, asking a policy for which that repository is private: nothing the first
    // answered is taken as this one's, so the same write is asked about again and refused.
    let second = std::sync::Arc::new(Counted::default());
    second.private.store(true, Ordering::SeqCst);
    let engine = engine.with_write_policy(second.clone());
    let before = tree(&store.folder("site"));
    let refused = engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect_err("a task of a repository the new policy holds private");
    assert_eq!(super::kind_of(&refused), "not-private-destination");
    assert_eq!(second.inspected.load(Ordering::SeqCst), 1);
    assert_eq!(first.inspected.load(Ordering::SeqCst), 1);
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
}

#[tokio::test]
async fn a_check_passed_under_one_term_scope_is_asked_again_under_another() {
    use std::sync::atomic::Ordering;
    let store = Store::new(None, json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(Counted::default());
    let engine = super::linked_engine(&store)
        .with_write_policy(policy.clone())
        .with_term_scope(Some(Vec::new()));
    engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect("a public repository's task");
    assert_eq!(policy.checked.load(Ordering::SeqCst), 1);
    // Every registered identity's terms now apply: the pass under no terms says nothing about
    // them, so the same text is checked again.
    let engine = engine.with_term_scope(None);
    engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect("a public repository's task");
    assert_eq!(policy.checked.load(Ordering::SeqCst), 2);
}

/// A linking caller's policy whose every repository is public and whose check answers what
/// `verdict` makes of each write, recording how many titles each check it was asked carried.
struct Judging {
    verdict: fn(
        &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> onetaskgraph_core::boundary::WriteVerdict,
    checked: std::sync::Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Judging {
    async fn repository_visibility(
        &self,
        _: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Ok(onetaskgraph_core::boundary::RepositoryVisibility::Public)
    }

    async fn check_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        let titles = ["Goal", "Library", "Consumer", "Rollout"]
            .iter()
            .filter(|title| input.metadata.iter().any(|value| value == *title))
            .count();
        self.checked.lock().unwrap().push(titles);
        Ok((self.verdict)(input))
    }
}

/// A store holding the plan in a public `draft`, and a linking caller's engine over it asking
/// a [`Judging`] policy answering `verdict`.
fn judged_engine(
    verdict: fn(
        &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> onetaskgraph_core::boundary::WriteVerdict,
) -> (Store, std::sync::Arc<Judging>, onetaskgraph_core::Engine) {
    let store = Store::laid(
        None,
        |sandbox| json!({"draft": folder(sandbox, "draft", "public")}),
    );
    plan(&store);
    let policy = std::sync::Arc::new(Judging {
        verdict,
        checked: std::sync::Mutex::default(),
    });
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    (store, policy, engine)
}

/// The project copy of the plan to `site`.
fn plan_copy_request() -> onetaskgraph_core::CopyRequest {
    onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(vec!["draft:goal".parse().unwrap()])
            .expect("one item"),
        scope: onetaskgraph_core::CopyScope::Projects { tasks: true },
        destination: onetaskgraph_plugin_api::SourceName::new("site").unwrap(),
        target_task: None,
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: false,
        no_link: false,
        expect: None,
        status_policy: onetaskgraph_core::StatusPolicy::Source,
        statuses: Vec::new(),
        conditions: Default::default(),
    }
}

/// A check refusing only what holds both the plan's project and its last task.
fn refusing_together(
    input: &onetaskgraph_core::boundary::PublicWriteInput,
) -> onetaskgraph_core::boundary::WriteVerdict {
    if input.metadata.iter().any(|value| value == "Goal")
        && input.metadata.iter().any(|value| value == "Rollout")
    {
        onetaskgraph_core::boundary::WriteVerdict::Refuse {
            reason: "only together".to_owned(),
        }
    } else {
        onetaskgraph_core::boundary::WriteVerdict::Pass
    }
}

#[tokio::test]
async fn a_union_refused_is_refused_again_in_the_same_command_though_each_write_passed_alone() {
    // The refusal's record-by-record checks pass each write alone; none of those is taken as
    // the union having passed, so the same copy asked again before the command ends is checked
    // whole again and refused again, writing nothing.
    let (store, policy, engine) = judged_engine(refusing_together);
    for attempt in 0..2 {
        let refused = engine
            .copy(&plan_copy_request())
            .await
            .expect_err("the union is refused");
        assert_eq!(
            super::kind_of(&refused),
            "boundary-refused",
            "attempt {attempt}"
        );
        assert!(tree(&store.folder("site")).is_empty(), "attempt {attempt}");
    }
    let checked = policy.checked.lock().unwrap().clone();
    assert_eq!(
        checked.iter().filter(|titles| **titles == 4).count(),
        2,
        "{checked:?}"
    );
}

/// The plan's project copied to `site` through a linking caller's engine asking `policy`.
async fn copied_through(
    verdict: fn(
        &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> onetaskgraph_core::boundary::WriteVerdict,
) -> (
    Store,
    std::sync::Arc<Judging>,
    Result<onetaskgraph_core::CopyReport, onetaskgraph_core::EngineError>,
) {
    let store = Store::laid(
        None,
        |sandbox| json!({"draft": folder(sandbox, "draft", "public")}),
    );
    plan(&store);
    let policy = std::sync::Arc::new(Judging {
        verdict,
        checked: std::sync::Mutex::default(),
    });
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    let request = onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(vec!["draft:goal".parse().unwrap()])
            .expect("one item"),
        scope: onetaskgraph_core::CopyScope::Projects { tasks: true },
        destination: onetaskgraph_plugin_api::SourceName::new("site").unwrap(),
        target_task: None,
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: false,
        no_link: false,
        expect: None,
        status_policy: onetaskgraph_core::StatusPolicy::Source,
        statuses: Vec::new(),
        conditions: Default::default(),
    };
    let copied = engine.copy(&request).await;
    (store, policy, copied)
}

#[tokio::test]
async fn a_union_refused_whose_writes_each_pass_alone_is_still_refused_and_nothing_is_written() {
    // A check refusing only what the whole plan says together — every record of it at once —
    // passes each record alone: the copy is refused all the same, named after the first record,
    // and nothing is written.
    let (store, policy, copied) = copied_through(|input| {
        if input.metadata.iter().any(|value| value == "Goal")
            && input.metadata.iter().any(|value| value == "Rollout")
        {
            onetaskgraph_core::boundary::WriteVerdict::Refuse {
                reason: "only together".to_owned(),
            }
        } else {
            onetaskgraph_core::boundary::WriteVerdict::Pass
        }
    })
    .await;
    let refused = copied.expect_err("the union is refused");
    assert_eq!(super::kind_of(&refused), "boundary-refused");
    assert!(refused.to_string().contains("draft:goal"), "{refused}");
    let checked = policy.checked.lock().unwrap().clone();
    assert_eq!(checked[0], 4, "the union first: {checked:?}");
    assert_eq!(checked.len(), 5, "then each record alone: {checked:?}");
    assert!(
        tree(&store.folder("site")).is_empty(),
        "nothing was written"
    );
}

#[tokio::test]
async fn a_union_the_check_cannot_decide_is_the_first_writes_unavailability_and_nothing_is_written()
{
    let (store, policy, copied) =
        copied_through(|_| onetaskgraph_core::boundary::WriteVerdict::Unavailable {
            reason: "the check could not be run".to_owned(),
        })
        .await;
    let refused = copied.expect_err("an unavailable check is never a pass");
    assert_eq!(super::kind_of(&refused), "boundary-unavailable");
    assert!(refused.to_string().contains("draft:goal"), "{refused}");
    assert_eq!(
        policy.checked.lock().unwrap().clone(),
        [4],
        "the union alone, with no record checked again"
    );
    assert!(
        tree(&store.folder("site")).is_empty(),
        "nothing was written"
    );
}

/// A linking caller's policy that cannot answer the first repository it is asked about, and
/// answers `public` for every later one, passing every check.
#[derive(Default)]
struct FailingOnce {
    inspected: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for FailingOnce {
    async fn repository_visibility(
        &self,
        _: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        if self
            .inspected
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            return Err(onetaskgraph_core::boundary::PolicyError {
                message: "the visibility command could not be run".to_owned(),
            });
        }
        Ok(onetaskgraph_core::boundary::RepositoryVisibility::Public)
    }

    async fn check_public_write(
        &self,
        _: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        Ok(onetaskgraph_core::boundary::WriteVerdict::Pass)
    }
}

#[tokio::test]
async fn a_repository_the_policy_could_not_answer_is_asked_again_in_the_same_command() {
    // The first answer fails, so that write is refused as naming a repository nobody vouched
    // for; the failure is not held, so the next write in the same command asks again, is told
    // the repository is public, and is written.
    let store = Store::new(None, json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(FailingOnce::default());
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    let refused = engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect_err("a repository nobody vouched for");
    assert_eq!(super::kind_of(&refused), "private-member");
    engine
        .create_task(&super::task_on_site(PLAIN, &[super::OPEN]))
        .await
        .expect("asked again, and public");
    assert_eq!(
        policy.inspected.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[test]
fn a_dependency_on_an_item_classified_private_is_withheld_and_said_on_standard_error() {
    // The consumer depends on a task of `ledgerless`, a public folder whose task is classified
    // private by its own record: the copy lands, the public destination carries no edge or id of
    // it, and standard error names the edge it left out and why.
    let (store, _, _) = screened();
    let path = store.folder("ledgerless");
    std::fs::create_dir_all(path.join("tasks")).unwrap();
    std::fs::write(
        path.join("tasks/secret.md"),
        "---\ntitle: Secret\nstatus: todo\nclassification: private\n---\nHeld close.\n",
    )
    .unwrap();
    let document = store.sandbox.project().join("onetaskgraph.yaml");
    let mut configured: Value =
        serde_json::from_str(&std::fs::read_to_string(&document).unwrap()).unwrap();
    configured["sources"]["ledgerless"] = json!({"plugin": "local-md", "config": {"root": path}});
    std::fs::write(&document, configured.to_string()).unwrap();
    plan(&store);
    store.record(
        "draft",
        "tasks",
        "consumer",
        "title: Consumer\nstatus: todo\nproject: goal\ndepends_on: [library, {id: \"ledgerless:secret\", item: task}]",
        PLAIN,
    );
    let output = store.run(&["project", "copy", "draft:goal", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(
            "onetaskgraph: withheld draft:consumer's dependency on ledgerless:secret from site: it \
             is classified private, and its source named no tracker an opaque reference to it \
             could point into\n"
        ),
        "{}",
        stderr(&output)
    );
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("ledgerless") && !text.contains("secret"),
            "{path} names the private item: {text}"
        );
    }
}

#[tokio::test]
async fn a_refused_union_whose_write_the_check_cannot_decide_alone_is_that_writes_unavailability() {
    // The union is refused, and checked record by record the check cannot decide the consumer
    // alone: the copy is refused as unavailable naming the consumer, never as having passed,
    // and nothing is written.
    let (store, policy, copied) = copied_through(|input| {
        let names = |title: &str| input.metadata.iter().any(|value| value == title);
        if names("Goal") && names("Rollout") {
            onetaskgraph_core::boundary::WriteVerdict::Refuse {
                reason: "only together".to_owned(),
            }
        } else if names("Consumer") {
            onetaskgraph_core::boundary::WriteVerdict::Unavailable {
                reason: "the check could not be run".to_owned(),
            }
        } else {
            onetaskgraph_core::boundary::WriteVerdict::Pass
        }
    })
    .await;
    let refused = copied.expect_err("an undecided write is never a pass");
    assert_eq!(super::kind_of(&refused), "boundary-unavailable");
    assert!(refused.to_string().contains("draft:consumer"), "{refused}");
    let checked = policy.checked.lock().unwrap().clone();
    assert_eq!(checked[0], 4, "the union first: {checked:?}");
    assert!(
        tree(&store.folder("site")).is_empty(),
        "nothing was written"
    );
}

/// A task copy of `ids` of the plan to `site`, recording no link back at `draft`, so a later
/// copy of the same tasks exposes exactly what this one did.
fn tasks_copied(ids: &[&str]) -> onetaskgraph_core::CopyRequest {
    onetaskgraph_core::CopyRequest {
        items: onetaskgraph_core::CopyItems::new(
            ids.iter().map(|id| id.parse().unwrap()).collect(),
        )
        .expect("an item"),
        scope: onetaskgraph_core::CopyScope::Tasks,
        no_link: true,
        ..plan_copy_request()
    }
}

/// A check refusing only what holds both of two plain tasks.
fn refusing_the_pair(
    input: &onetaskgraph_core::boundary::PublicWriteInput,
) -> onetaskgraph_core::boundary::WriteVerdict {
    if input.metadata.iter().any(|value| value == "Alpha")
        && input.metadata.iter().any(|value| value == "Beta")
    {
        onetaskgraph_core::boundary::WriteVerdict::Refuse {
            reason: "only together".to_owned(),
        }
    } else {
        onetaskgraph_core::boundary::WriteVerdict::Pass
    }
}

#[tokio::test]
async fn writes_that_passed_in_separate_checks_are_checked_together_before_they_are_written_together()
 {
    // Each task copied alone passes. Copied together in the same command, every value either
    // exposes has passed a check — but never in one input — so the two are put to the check
    // together, which refuses them, and nothing more is written.
    let (store, policy, engine) = judged_engine(refusing_the_pair);
    // Two tasks of no project and no edge, so each exposes the same alone as together.
    for (id, title) in [("alpha", "Alpha"), ("beta", "Beta")] {
        store.record(
            "draft",
            "tasks",
            id,
            &format!("title: {title}\nstatus: todo"),
            PLAIN,
        );
    }
    // Copied alone twice each: from the second copy on, each exposes the record of the copy
    // before it too, and exposes exactly that again copied with the other.
    for alone in ["draft:alpha", "draft:beta", "draft:alpha", "draft:beta"] {
        engine
            .copy(&tasks_copied(&[alone]))
            .await
            .expect("one task alone passes");
    }
    let before = tree(&store.folder("site"));
    let refused = engine
        .copy(&tasks_copied(&["draft:alpha", "draft:beta"]))
        .await
        .expect_err("the two together are refused");
    assert_eq!(super::kind_of(&refused), "boundary-refused");
    assert_eq!(
        policy.checked.lock().unwrap().len(),
        4 + 3,
        "four alone, then the pair and each of it"
    );
    assert_eq!(
        tree(&store.folder("site")),
        before,
        "nothing more was written"
    );
}

#[tokio::test]
async fn a_value_that_passed_on_one_surface_is_checked_again_on_another() {
    // The same string passed as a task's prose, then named as a task's title: a check reads
    // each surface on its own terms, so the title is put to it rather than taken as passed.
    let store = Store::new(None, json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(Counted::default());
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    let mut prose = super::task_on_site("zeta", &[]);
    prose.title = "Linked".to_owned();
    engine.create_task(&prose).await.expect("prose passes");
    let mut titled = super::task_on_site("Linked", &[]);
    titled.title = "zeta".to_owned();
    engine.create_task(&titled).await.expect("a title passes");
    assert_eq!(
        policy.checked.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the value moved to another surface was checked"
    );
}
