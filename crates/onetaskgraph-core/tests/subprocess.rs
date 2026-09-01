//! The two halves of `docs/plugin-protocol.md`, driven against each other.
//!
//! Nothing here stands in for the layer under test. The engine's half is the real
//! [`SubprocessSource`]; the plugin's half is the real [`serve`]; between them is a real
//! operating-system pipe with real JSON lines on it. What the pipe buys over a spawned
//! program — which the end-to-end journeys use, and which is the shape a user gets — is
//! the ability to hold the *plugin's* end and answer badly on purpose, which is the only
//! way to prove the refusals §6 owes.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write, pipe};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use onetaskgraph_core::{
    MAX_LINE, RequestDeadline, SubprocessConfig, SubprocessSource, plugin_for, serve,
};
use onetaskgraph_plugin_api::{
    Cursor, Direction, Document, DocumentQuery, ItemWrite, LabelFilter, Location, NativeId,
    PageRequest, ProjectFilter, ProjectQuery, SecretResolver, SourceError, SourceName, Support,
    Task, TaskQuery, TaskSource, WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

/// The work every test here serves, small enough to page through in one assertion.
fn dataset() -> Value {
    json!({
        "tasks": [
            {"id": "T-1", "title": "Alpha", "content": "the engine core",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}], "project": "P-1"},
            {"id": "T-2", "title": "Beta", "content": "alpha in the body",
             "status": {"category": "done", "name": "Shipped"}, "labels": []},
            {"id": "T-3", "title": "Gamma", "content": "unrelated",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}]}
        ],
        "projects": [
            {"id": "P-1", "title": "Engine", "content": "the engine",
             "status": {"category": "in-progress", "name": "Doing"}, "labels": []},
            {"id": "P-2", "title": "Docs", "content": "alpha docs",
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ],
        "labels": [{"id": "L-1", "name": "bug"}, {"id": "L-2", "name": "chore"}],
        "task_dependencies": [
            {"from": "T-1", "to": "T-2", "kind": "blocks"},
            {"from": "T-3", "to": "T-2", "kind": "related"}
        ],
        "project_dependencies": [{"from": "P-1", "to": "P-2", "kind": "blocks"}]
    })
}

/// The settings this build's reference host takes: which plugin, and its own block.
fn hosted_settings() -> Value {
    let mut config = dataset();
    config["capabilities"] = json!({"max_page_size": 2});
    json!({"kind": "in-memory", "config": config})
}

/// A name every test configures its source under.
fn name() -> SourceName {
    SourceName::new("work").expect("a usable name")
}

/// A resolver with nothing in it.
struct NoSecrets;

impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// The same source this build would have used in process, for answers to be compared to.
fn in_process() -> Box<dyn TaskSource> {
    let settings = hosted_settings();
    plugin_for("in-memory")
        .expect("the in-memory plugin is registered")
        .build(&name(), &settings["config"], &NoSecrets)
        .expect("the dataset is a valid block")
}

/// A source at the other end of a pipe, served by [`serve`] over the same dataset.
fn a_process_away(settings: Value) -> Result<SubprocessSource, SourceError> {
    let (to_engine, from_plugin) = pipe().expect("a pipe");
    let (to_plugin, from_engine) = pipe().expect("a pipe");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let _ = runtime.block_on(serve(BufReader::new(to_plugin), from_plugin));
    });
    SubprocessSource::over(from_engine, to_engine, &name(), &settings, BTreeMap::new())
}

/// A source at the other end of a peer that answers exactly these lines, in order.
///
/// The point of it is the answers a correct plugin never gives: a version it was not
/// asked for, an envelope with both members, an id nobody sent.
fn scripted(answers: Vec<String>) -> Result<SubprocessSource, SourceError> {
    let (to_engine, mut from_liar) = pipe().expect("a pipe");
    let (to_liar, from_engine) = pipe().expect("a pipe");
    std::thread::spawn(move || {
        let mut asked = BufReader::new(to_liar);
        for answer in answers {
            let mut request = String::new();
            match asked.read_line(&mut request) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if writeln!(from_liar, "{answer}").is_err() || from_liar.flush().is_err() {
                return;
            }
        }
    });
    SubprocessSource::over(from_engine, to_engine, &name(), &json!({}), BTreeMap::new())
}

/// One page request, asking for `limit` rows from the start.
fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

/// A task query with no predicates at all.
fn everything() -> TaskQuery {
    TaskQuery {
        text: None,
        labels: LabelFilter::default(),
        statuses: Vec::new(),
        project: ProjectFilter::Any,
    }
}

/// The line `serve` writes in answer to `requests`, one per request.
fn served(requests: &[Value]) -> Vec<Value> {
    let mut input = String::new();
    for request in requests {
        input.push_str(&serde_json::to_string(request).expect("a request"));
        input.push('\n');
    }
    let mut output: Vec<u8> = Vec::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime
        .block_on(serve(input.as_bytes(), &mut output))
        .expect("the streams are in memory and cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("every answer is one JSON object"))
        .collect()
}

/// An `initialize` request over this build's reference host settings.
fn handshake(version: u32, settings: Value) -> Value {
    json!({
        "id": "0",
        "method": "initialize",
        "params": {
            "protocol_version": version,
            "engine": {"name": "onetaskgraph", "version": "0.1.0"},
            "source_name": "work",
            "config": settings,
            "secrets": {}
        }
    })
}

/// The `kind` of the error an answer carries.
fn refusal(answer: &Value) -> &str {
    answer["error"]["kind"].as_str().expect("an error envelope")
}

/// The message of the error an answer carries.
fn because(answer: &Value) -> &str {
    answer["error"]["message"]
        .as_str()
        .expect("a message to read")
}

#[tokio::test]
async fn a_source_a_process_away_answers_what_the_same_source_answers_in_process() {
    let here = in_process();
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");

    assert_eq!(
        there.query_tasks(&everything(), &page(50)).await,
        here.query_tasks(&everything(), &page(50)).await
    );
    let projects = ProjectQuery {
        text: None,
        labels: LabelFilter::default(),
        statuses: Vec::new(),
    };
    assert_eq!(
        there.query_projects(&projects, &page(50)).await,
        here.query_projects(&projects, &page(50)).await
    );
    assert_eq!(there.labels(&page(50)).await, here.labels(&page(50)).await);
    assert_eq!(
        there.get_task(&NativeId("T-1".to_owned())).await,
        here.get_task(&NativeId("T-1".to_owned())).await
    );
    assert_eq!(
        there.get_project(&NativeId("P-1".to_owned())).await,
        here.get_project(&NativeId("P-1".to_owned())).await
    );
    assert_eq!(there.health().await, here.health().await);
}

#[tokio::test]
async fn a_write_crosses_the_wire_and_lands_in_the_source_on_the_other_side() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");
    // Read at the handshake, exactly as capabilities are, so the engine refuses a copy
    // into an unwritable plugin before it reads anything.
    assert_eq!(there.writes(), WriteSupport::Supported);

    let item: Task = serde_json::from_value(json!({
        "id": "T-9", "title": "Written across a pipe", "content": "body",
        "status": {"category": "todo", "name": "Todo"}, "labels": [],
        "metadata": {"caller.count": 3, "caller.shape": {"nested": [true, null]}},
        "repositories": ["github.com/nickderobertis/onetaskgraph"]
    }))
    .expect("a task");
    let written = there
        .write_task(&ItemWrite {
            target: None,
            item: item.clone(),
            depends_on: Vec::new(),
        })
        .await
        .expect("the plugin takes the write");
    assert_eq!(written, NativeId("T-9".to_owned()));

    // What a source can do must not change because it is a process away: the value and
    // the JSON type of every caller-defined key survive the boundary in both directions.
    let read = there
        .get_task(&written)
        .await
        .expect("the plugin answers")
        .expect("the task is there");
    assert_eq!(read.title, "Written across a pipe");
    assert_eq!(read.metadata["caller.count"], json!(3));
    assert_eq!(
        read.metadata["caller.shape"],
        json!({"nested": [true, null]})
    );

    let updated = there
        .write_project(&ItemWrite {
            target: Some(NativeId("P-1".to_owned())),
            item: serde_json::from_value(json!({
                "id": "P-1", "title": "Renamed across a pipe",
                "status": {"category": "todo", "name": "Todo"}, "labels": []
            }))
            .expect("a project"),
            depends_on: Vec::new(),
        })
        .await
        .expect("the plugin takes the write");
    assert_eq!(updated, NativeId("P-1".to_owned()));
    assert_eq!(
        there.get_project(&updated).await.unwrap().unwrap().title,
        "Renamed across a pipe"
    );

    // And a refusal is the plugin's own, carried whole rather than flattened.
    let Err(SourceError::Refused { message }) = there
        .write_task(&ItemWrite {
            target: Some(NativeId("absent".to_owned())),
            item,
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a target the hosted source does not hold must be refused");
    };
    assert!(
        message.contains("names no task this source holds"),
        "{message}"
    );
}

#[tokio::test]
async fn a_plugin_that_says_nothing_about_writing_is_read_as_one_that_cannot() {
    // §3.3: the member is optional, and absent means unsupported — which is what a
    // version-1 plugin written before there was a write side sends.
    let silent = scripted(vec![
        json!({"id":"0","result":{"protocol_version":2,"kind":"ancient","capabilities":{
            "projects":"native","orphan_tasks":"native","filter_by_label":"native",
            "filter_by_status":"native","search_title":"native","search_content":"native",
            "task_dependencies":"both-directions","project_dependencies":"both-directions",
            "max_page_size":10}}})
        .to_string(),
    ])
    .expect("the handshake succeeds");
    assert_eq!(silent.writes(), WriteSupport::Unsupported);
}

#[tokio::test]
async fn a_hosted_source_reports_its_own_kind_and_its_own_capabilities() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");

    // The transport is not the kind: what a caller sees is the source that answered, so a
    // plan naming `in-memory` is telling the truth about where the rows came from.
    assert_eq!(there.kind(), "in-memory");
    assert_eq!(there.capabilities(), in_process().capabilities());
    assert_eq!(there.capabilities().max_page_size, 2);
}

#[tokio::test]
async fn an_existing_stream_applies_its_deadline_after_initialization() {
    let (to_engine, mut from_plugin) = pipe().expect("a pipe");
    let (to_plugin, from_engine) = pipe().expect("a pipe");
    std::thread::spawn(move || {
        let mut requests = BufReader::new(to_plugin);
        let mut request = String::new();
        requests.read_line(&mut request).expect("the handshake");
        writeln!(
            from_plugin,
            "{}",
            json!({"id": "0", "result": {"protocol_version": 2,
                "kind": "silent", "capabilities": capabilities()}})
        )
        .expect("the handshake answer");
        from_plugin.flush().expect("the answer is visible");
        request.clear();
        requests
            .read_line(&mut request)
            .expect("the health request");
        std::thread::park();
    });
    let source = SubprocessSource::over_with_request_deadline(
        from_engine,
        to_engine,
        &name(),
        &json!({}),
        BTreeMap::new(),
        RequestDeadline::from_millis(NonZeroU64::new(20).expect("positive")),
    )
    .expect("the handshake succeeds");

    let SourceError::Unavailable { message } = source.health().await.expect_err("it times out")
    else {
        panic!("a request deadline is a reachability failure");
    };
    assert!(
        message.contains("health") && message.contains("20 milliseconds"),
        "{message}"
    );
    let again = Instant::now();
    assert!(
        matches!(
            source.labels(&page(1)).await,
            Err(SourceError::Unavailable { .. })
        ),
        "a later request is bounded too"
    );
    assert!(
        again.elapsed() < Duration::from_secs(1),
        "the later request hung"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_deadline_turns_a_silent_child_into_a_named_source_error() {
    // POSIX supplies one ubiquitous executable that can answer the real handshake and
    // then stay silent. Windows lacks an equivalent system program; the transport code
    // is platform-neutral and its functional journeys still run there.
    let answer = json!({"id": "0", "result": {"protocol_version": 2,
        "kind": "silent", "capabilities": capabilities()}})
    .to_string();
    let source = SubprocessSource::connect_with_deadline(
        "/bin/sh",
        &[
            "-c".to_owned(),
            "read -r _; printf '%s\\n' \"$1\"; read -r _; while :; do :; done".to_owned(),
            "_".to_owned(),
            answer,
        ],
        &name(),
        &json!({}),
        BTreeMap::new(),
        RequestDeadline::from_millis(NonZeroU64::new(20).expect("positive")),
    )
    .expect("the handshake succeeds");

    let started = Instant::now();
    let SourceError::Unavailable { message } = source.health().await.expect_err("it times out")
    else {
        panic!("a deadline is a reachability failure");
    };
    assert!(
        message.contains("health") && message.contains("20 milliseconds"),
        "{message}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the deadline did not hang"
    );
    let again = Instant::now();
    assert!(
        matches!(source.health().await, Err(SourceError::Unavailable { .. })),
        "the timed-out connection stays closed"
    );
    assert!(
        again.elapsed() < Duration::from_secs(1),
        "a later call did not inherit the hang"
    );
}

#[tokio::test]
async fn both_dependency_directions_cross_the_wire_unchanged() {
    let here = in_process();
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");

    for direction in [Direction::DependsOn, Direction::DependedOnBy] {
        assert_eq!(
            there
                .task_dependencies(&NativeId("T-2".to_owned()), direction, &page(50))
                .await,
            here.task_dependencies(&NativeId("T-2".to_owned()), direction, &page(50))
                .await,
            "task dependencies differ for {direction:?}"
        );
        assert_eq!(
            there
                .project_dependencies(&NativeId("P-2".to_owned()), direction, &page(50))
                .await,
            here.project_dependencies(&NativeId("P-2".to_owned()), direction, &page(50))
                .await,
            "project dependencies differ for {direction:?}"
        );
    }
}

#[tokio::test]
async fn a_walk_across_the_wire_pages_to_exhaustion_and_stops() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = there
            .query_tasks(
                &everything(),
                &PageRequest {
                    cursor: cursor.clone(),
                    limit: 2,
                },
            )
            .await
            .expect("a page");
        seen.extend(page.items.into_iter().map(|task| task.id.0));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, ["T-1", "T-2", "T-3"]);
}

#[tokio::test]
async fn a_predicate_crosses_the_wire_and_the_hosted_source_applies_it() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");
    let query = TaskQuery {
        text: None,
        labels: LabelFilter {
            any_of: vec!["bug".to_owned()],
            ..LabelFilter::default()
        },
        statuses: Vec::new(),
        project: ProjectFilter::Any,
    };

    let kept: Vec<String> = there
        .query_tasks(&query, &page(50))
        .await
        .expect("a page")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect();
    assert_eq!(kept, ["T-1", "T-3"]);
}

#[tokio::test]
async fn an_id_that_names_nothing_is_null_rather_than_a_failure() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");

    assert_eq!(there.get_task(&NativeId("nope".to_owned())).await, Ok(None));
    assert_eq!(
        there.get_project(&NativeId("nope".to_owned())).await,
        Ok(None)
    );
}

#[test]
fn a_protocol_version_this_plugin_does_not_know_is_refused_by_name() {
    let answers = served(&[handshake(3, hosted_settings())]);

    assert_eq!(
        answers.len(),
        1,
        "the plugin exits after refusing: {answers:?}"
    );
    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("version 3") && because(&answers[0]).contains("version 2"),
        "the refusal names both versions: {}",
        because(&answers[0])
    );
}

#[test]
fn a_request_before_the_handshake_is_refused_rather_than_answered() {
    let answers =
        served(&[json!({"id": "1", "method": "labels", "params": {"page": {"limit": 5}}})]);

    assert_eq!(refusal(&answers[0]), "malformed");
    assert!(
        because(&answers[0]).contains("before the handshake"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_second_handshake_on_one_connection_is_refused() {
    let answers = served(&[
        handshake(2, hosted_settings()),
        handshake(2, hosted_settings()),
    ]);

    assert!(answers[0]["result"].is_object(), "{answers:?}");
    assert_eq!(refusal(&answers[1]), "malformed");
    assert!(
        because(&answers[1]).contains("already initialized"),
        "{}",
        because(&answers[1])
    );
}

#[test]
fn a_line_with_no_request_id_is_skipped_and_the_next_request_still_answered() {
    let mut input = String::from("this is not JSON at all\n");
    input.push_str("{\"method\":\"labels\"}\n");
    input.push('\n');
    input.push_str(&serde_json::to_string(&handshake(2, hosted_settings())).expect("a request"));
    input.push('\n');
    let mut output: Vec<u8> = Vec::new();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(serve(input.as_bytes(), &mut output))
        .expect("in-memory streams cannot fail");

    let answered = String::from_utf8(output).expect("UTF-8");
    assert_eq!(
        answered.lines().count(),
        1,
        "only the handshake is answerable: {answered}"
    );
    assert!(answered.contains("\"protocol_version\":2"), "{answered}");
}

#[test]
fn a_line_that_is_addressed_but_is_not_a_request_is_answered_rather_than_dropped() {
    let answers = served(&[json!({"id": "9", "method": 7})]);

    assert_eq!(answers[0]["id"], "9");
    assert_eq!(refusal(&answers[0]), "malformed");
    assert!(
        because(&answers[0]).contains("not a request envelope"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_method_this_version_does_not_have_is_refused_by_name() {
    let answers = served(&[
        handshake(2, hosted_settings()),
        json!({"id": "1", "method": "delete_everything", "params": {}}),
    ]);

    assert_eq!(refusal(&answers[1]), "malformed");
    assert!(
        because(&answers[1]).contains("delete_everything"),
        "{}",
        because(&answers[1])
    );
}

#[test]
fn parameters_of_the_wrong_shape_are_refused_naming_the_method() {
    let answers = served(&[
        handshake(2, hosted_settings()),
        json!({"id": "1", "method": "get_task", "params": {}}),
    ]);

    assert_eq!(refusal(&answers[1]), "malformed");
    assert!(
        because(&answers[1]).contains("get_task"),
        "{}",
        because(&answers[1])
    );
}

#[test]
fn settings_naming_no_plugin_of_this_build_are_refused_with_the_kinds_it_knows() {
    let answers = served(&[handshake(2, json!({"kind": "jira", "config": {}}))]);

    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("jira") && because(&answers[0]).contains("in-memory"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn settings_this_host_cannot_read_are_refused_for_what_they_are_missing() {
    let answers = served(&[handshake(2, json!({"config": {}}))]);

    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("kind"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_hosted_plugin_that_refuses_to_build_answers_with_its_own_error() {
    let answers = served(&[handshake(2, json!({"kind": "linear", "config": {}}))]);

    // Linear has no forwarded credential, proving a build failure crosses the wire as
    // the plugin's own authentication error rather than as a transport failure.
    assert!(answers[0]["error"].is_object(), "{answers:?}");
    assert!(
        because(&answers[0]).contains("LINEAR_API_KEY"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_plugin_answering_in_a_version_the_engine_did_not_ask_for_is_refused() {
    let error = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 3, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
    ])
    .expect_err("a version the engine does not speak");

    let SourceError::Config { message } = error else {
        panic!("a version disagreement is a configuration refusal: {error:?}");
    };
    assert!(
        message.contains("version 2") && message.contains("version 3"),
        "{message}"
    );
}

#[test]
fn a_plugin_that_omits_the_protocol_version_is_refused_rather_than_assumed() {
    let error = scripted(vec![
        json!({"id": "0", "result": {"kind": "made-up", "capabilities": capabilities()}})
            .to_string(),
    ])
    .expect_err("an unstated version");

    let SourceError::Config { message } = error else {
        panic!("an unstated version is a configuration refusal: {error:?}");
    };
    assert!(message.contains("did not say"), "{message}");
}

#[test]
fn a_handshake_answer_that_is_not_an_initialize_result_is_malformed() {
    let error = scripted(vec![json!({"id": "0", "result": {"kind": 7}}).to_string()])
        .expect_err("a result of the wrong shape");

    assert!(matches!(error, SourceError::Malformed { .. }), "{error:?}");
}

#[test]
fn a_plugin_kind_with_no_name_is_rejected_at_the_handshake_boundary() {
    for kind in ["", " \t"] {
        let error = scripted(vec![
            json!({"id": "0", "result": {"protocol_version": 2, "kind": kind,
                   "capabilities": capabilities()}})
            .to_string(),
        ])
        .expect_err("a plugin kind must name something");

        let SourceError::Malformed { message } = error else {
            panic!("an invalid handshake field is malformed: {error:?}");
        };
        assert!(message.contains("plugin kind"), "{message}");
    }
}

#[test]
fn a_handshake_answer_carrying_both_members_is_a_protocol_violation() {
    let error = scripted(vec![
        json!({"id": "0", "result": {}, "error": {"kind": "auth", "message": "no"}}).to_string(),
    ])
    .expect_err("both members at once");

    let SourceError::Malformed { message } = error else {
        panic!("both members at once is a violation: {error:?}");
    };
    assert!(message.contains("both a result and an error"), "{message}");
}

#[test]
fn a_handshake_the_plugin_refuses_is_reported_as_the_plugin_worded_it() {
    let error = scripted(vec![
        json!({"id": "0", "error": {"kind": "auth", "message": "the token expired"}}).to_string(),
    ])
    .expect_err("a refused handshake");

    assert_eq!(
        error,
        SourceError::Auth {
            message: "the token expired".to_owned()
        }
    );
}

#[test]
fn a_plugin_that_says_nothing_at_all_is_reported_as_unreachable() {
    let error = scripted(Vec::new()).expect_err("nothing was answered");

    assert!(
        matches!(error, SourceError::Unavailable { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_plugin_answering_an_id_nobody_asked_is_a_protocol_violation() {
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
        json!({"id": "999", "result": {"reachable": true}}).to_string(),
    ])
    .expect("the handshake succeeds");

    let SourceError::Malformed { message } = source.health().await.expect_err("a wrong id") else {
        panic!("an id nobody sent is a violation");
    };
    assert!(message.contains("999"), "{message}");
}

#[tokio::test]
async fn a_plugin_line_that_is_not_json_is_quoted_back_at_a_readable_length() {
    let long = "x".repeat(500);
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
        long,
    ])
    .expect("the handshake succeeds");

    let SourceError::Malformed { message } = source.health().await.expect_err("not JSON") else {
        panic!("a line that is not JSON is a violation");
    };
    assert!(message.contains("(truncated)"), "{message}");
    assert!(message.len() < 500, "the quote is cut short: {message}");
}

#[tokio::test]
async fn a_delete_result_is_read_as_an_object_rather_than_as_exactly_the_empty_one() {
    // §4.10 answers `{}`, and §2.1 is what says how strictly to read that: a reader ignores
    // members it does not know, at every level, so a later version can add an optional one
    // without a version bump. A decoder refusing the unknown member here would refuse that
    // plugin outright — and would be the only one of this boundary that did. What is
    // refused is an answer that is not an object at all, which no version of §4.10 permits.
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
        json!({"id": "1", "result": {"removed_at": "2026-08-30T00:00:00Z"}}).to_string(),
        json!({"id": "2", "result": "done"}).to_string(),
    ])
    .expect("the handshake succeeds");

    source
        .delete_task(&NativeId("T-1".to_owned()))
        .await
        .expect("a member this version does not know is ignored, not refused");

    let SourceError::Malformed { message } = source
        .delete_project(&NativeId("P-1".to_owned()))
        .await
        .expect_err("a result that is not an object")
    else {
        panic!("a delete result that is not an object is a protocol violation");
    };
    assert!(message.contains("delete_project"), "{message}");
}

#[tokio::test]
async fn a_document_refusal_crosses_the_wire_as_the_hosted_plugin_s_own_reason() {
    // Both halves of §4.11 and §4.12, driven against each other over a real pipe. The
    // engine's side forwards these four the way it forwards every other method, and the
    // reason that comes back is the *hosted* plugin's — `in-memory`, not `subprocess` —
    // because this side holds no opinion about documents any more than it holds one about
    // anything else.
    let source = a_process_away(hosted_settings()).expect("the handshake succeeds");

    // What the engine reads once, at the handshake, and never asks again.
    assert_eq!(source.capabilities().documents, Support::Unsupported);

    for refusal in [
        source
            .get_document(&NativeId("D-1".to_owned()))
            .await
            .map(|found| format!("{found:?}")),
        source
            .query_documents(&DocumentQuery::default(), &page(2))
            .await
            .map(|answered| format!("{answered:?}")),
    ] {
        let Err(SourceError::Refused { message }) = refusal else {
            panic!("a source with no documents refuses a document read: {refusal:?}");
        };
        assert_eq!(message, "the in-memory plugin has no documents");
    }

    // The two writes refuse for the reason every other write refuses, which this hosted
    // source does have: it is writable, and it has nowhere to put a document.
    assert!(source.writes().is_supported());
    for refusal in [
        source
            .write_document(&ItemWrite {
                target: None,
                item: filed(),
                depends_on: Vec::new(),
            })
            .await
            .map(|id| format!("{id:?}")),
        source
            .delete_document(&NativeId("D-1".to_owned()))
            .await
            .map(|()| String::new()),
    ] {
        let Err(SourceError::Refused { message }) = refusal else {
            panic!("a source with no document side refuses a document write: {refusal:?}");
        };
        assert_eq!(message, "the in-memory plugin cannot be written");
    }
}

#[tokio::test]
async fn a_document_a_peer_really_answers_with_crosses_the_wire_whole() {
    // The refusal is what every source of this build gives, so it is what the shared
    // journey drives. This is the other half: a peer that *does* have documents, answering
    // all four methods successfully over a real pipe, so the engine's own encoding and
    // decoding of them is exercised rather than assumed. Nothing registered here can play
    // that peer yet — no plugin implements documents — which is exactly why it is scripted.
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": documentary(), "writes": "supported"}})
        .to_string(),
        json!({"id": "1", "result": {"document": wire_document()}}).to_string(),
        json!({"id": "2", "result": {"items": [wire_document()], "next": "b2Zmc2V0PTE"}})
            .to_string(),
        json!({"id": "3", "result": {"id": "D-9"}}).to_string(),
        json!({"id": "4", "result": {}}).to_string(),
    ])
    .expect("the handshake succeeds");

    assert_eq!(source.capabilities().documents, Support::Native);

    let found = source
        .get_document(&NativeId("D-1".to_owned()))
        .await
        .expect("the peer holds it")
        .expect("a document, not a null");
    assert_eq!(found.title, "Why the store holds a document");
    assert_eq!(found.project, Some(NativeId("P-1".to_owned())));
    // The member a consumer acts on without knowing the backend, decoded as the one key
    // that was present rather than as a bare string.
    assert_eq!(
        found.location,
        Some(Location::Path("/home/someone/notes/design.md".to_owned()))
    );

    let page = source
        .query_documents(&DocumentQuery::default(), &page(5))
        .await
        .expect("the peer answers a page");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, NativeId("D-1".to_owned()));
    assert_eq!(page.next, Some(Cursor("b2Zmc2V0PTE".to_owned())));

    let written = source
        .write_document(&ItemWrite {
            target: None,
            item: filed(),
            depends_on: Vec::new(),
        })
        .await
        .expect("the peer takes it");
    assert_eq!(written, NativeId("D-9".to_owned()));

    source
        .delete_document(&NativeId("D-9".to_owned()))
        .await
        .expect("the peer removes it");
}

/// A handshake from a peer that says it has documents, unlike anything this build hosts.
fn documentary() -> Value {
    let mut declared = capabilities();
    declared["documents"] = json!("native");
    declared
}

/// One document as a peer puts it on the wire.
fn wire_document() -> Value {
    json!({
        "id": "D-1",
        "title": "Why the store holds a document",
        "content": "A person cannot review a plan node by node.",
        "project": "P-1",
        "labels": [{"id": "L-1", "name": "design", "color": null}],
        "url": "https://example.invalid/D-1",
        "location": {"path": "/home/someone/notes/design.md"},
        "created_at": null,
        "updated_at": null
    })
}

#[tokio::test]
async fn a_handshake_that_says_nothing_about_documents_is_read_as_having_none() {
    // §2.1 and §4.2 together: `documents` is an optional member with a documented default,
    // so a plugin written before there were documents — which is what `capabilities()`
    // below spells, omitting it — is read as the document-free source it is rather than
    // refused for a member it has never heard of.
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
    ])
    .expect("the handshake succeeds");

    assert_eq!(source.capabilities().documents, Support::Unsupported);
}

/// The document a write test hands the far side, which never gets as far as holding one.
fn filed() -> Document {
    Document {
        id: NativeId("D-1".to_owned()),
        title: "Why the store holds a document".to_owned(),
        content: None,
        project: Some(NativeId("P-1".to_owned())),
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: Vec::new(),
    }
}

#[tokio::test]
async fn a_plugin_that_stops_answering_fails_this_call_and_every_later_one() {
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
    ])
    .expect("the handshake succeeds");

    assert!(
        matches!(source.health().await, Err(SourceError::Unavailable { .. })),
        "the first call after the plugin left is unavailable"
    );
    assert!(
        matches!(
            source.labels(&page(5)).await,
            Err(SourceError::Unavailable { .. })
        ),
        "and so is the next one, rather than waiting for an answer nothing will send"
    );
    assert!(
        matches!(source.health().await, Err(SourceError::Unavailable { .. })),
        "and every one after that, from the record that there is no worker left"
    );
}

#[test]
fn a_program_that_does_not_exist_is_reported_with_the_path_that_was_tried() {
    let error = SubprocessSource::connect(
        "onetaskgraph-no-such-plugin-program",
        &[],
        &name(),
        &json!({}),
        BTreeMap::new(),
    )
    .expect_err("nothing to run");

    let SourceError::Unavailable { message } = error else {
        panic!("a program that will not run is unreachable: {error:?}");
    };
    assert!(
        message.contains("onetaskgraph-no-such-plugin-program"),
        "{message}"
    );
}

/// A real program every host running these tests is guaranteed to have: this one.
///
/// It is emphatically *not* a plugin, and that is the point. Proving what the engine does
/// when it spawns something that does not speak the protocol needs a real process — a real
/// `spawn`, real pipes, a real exit and a real kill on drop — and the test binary is the
/// one such program that is present on every platform this suite runs on without being
/// built, shipped or installed for the purpose.
fn not_a_plugin(argument: &str) -> Result<SubprocessSource, SourceError> {
    let program = std::env::current_exe().expect("this test binary has a path");
    SubprocessSource::connect(
        &program.to_string_lossy(),
        &[argument.to_owned()],
        &name(),
        &json!({}),
        BTreeMap::new(),
    )
}

#[test]
fn a_spawned_program_that_does_not_speak_the_protocol_is_refused_rather_than_waited_on() {
    // `--list` makes the test binary print its own test names and exit: a real program,
    // really spawned, that does not speak this protocol. Which refusal it earns is a race
    // it is not worth pretending away — the engine may get the non-envelope line, or it
    // may find the program already gone when it writes — so what this asserts is the part
    // that is not racy, and the part that matters: the run is refused, promptly, and
    // nothing becomes a source. The wording of a non-envelope answer is pinned
    // deterministically by `a_handshake_answer_that_is_not_a_line_of_json_is_a_violation`.
    let Err(error) = not_a_plugin("--list") else {
        panic!("a program that is not a plugin cannot become a source");
    };

    assert!(
        matches!(
            error,
            SourceError::Malformed { .. } | SourceError::Unavailable { .. }
        ),
        "a program that does not speak the protocol is refused as one that cannot: {error:?}"
    );
}

#[test]
fn a_spawned_program_that_fails_at_once_is_reported_with_what_it_wrote() {
    // An argument no test harness accepts: the program exits without writing a response,
    // having explained itself on standard error — which is the one place the reason is.
    let Err(error) = not_a_plugin("--definitely-not-a-flag") else {
        panic!("a program that exits at once cannot become a source");
    };

    let SourceError::Unavailable { message } = error else {
        panic!("a program that answered nothing is unreachable: {error:?}");
    };
    assert!(
        message.contains("definitely-not-a-flag"),
        "the plugin's own words reach the user: {message}"
    );
}

#[cfg(unix)]
#[test]
fn a_silent_handshake_is_stopped_at_its_configured_deadline() {
    // This probe uses the POSIX shell solely to create a portable-on-Unix child that
    // remains alive without writing. Windows has no corresponding system executable;
    // the runtime path itself is platform-neutral and the later-request deadline is
    // exercised over Rust pipes on every platform above.
    let started = Instant::now();
    let Err(error) = subprocess_plugin().build(
        &name(),
        &json!({"command": "/bin/sh", "args": ["-c", "while :; do :; done"],
                "deadline_ms": 20}),
        &NoSecrets,
    ) else {
        panic!("a silent handshake reaches its configured deadline");
    };

    let SourceError::Unavailable { message } = error else {
        panic!("a handshake deadline is a reachability failure: {error:?}");
    };
    assert!(
        message.contains("initialize") && message.contains("20 milliseconds"),
        "{message}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the handshake hung"
    );
}

#[cfg(unix)]
#[test]
fn a_rate_limited_handshake_keeps_its_wait_and_gains_what_the_plugin_wrote() {
    // A plugin that refuses the handshake has usually said why on standard error and
    // nowhere else, and the engine appends that to the refusal it reports. `rate-limited`
    // was the one kind that could not take it — the variant had no message — so what the
    // plugin wrote was dropped. It has one now.
    //
    // What must survive alongside it is `retry_after_seconds`: that is the piece of this
    // refusal the engine acts on, and a diagnostic that replaced it with prose would leave
    // the caller with a reason and no wait. A real child writes both halves.
    let script = r#"read -r _request
printf '%s\n' 'the board is refusing bursts; slow down' >&2
printf '%s\n' '{"id":"0","error":{"kind":"rate-limited","retry_after_seconds":45}}'"#;
    let error = SubprocessSource::connect(
        "/bin/sh",
        &["-c".to_owned(), script.to_owned()],
        &name(),
        &json!({}),
        BTreeMap::new(),
    )
    .expect_err("a handshake the plugin rate-limited");

    let SourceError::RateLimited {
        retry_after_seconds,
        message,
    } = error
    else {
        panic!("a rate-limited handshake reported as {error:?}");
    };
    assert_eq!(
        retry_after_seconds,
        Some(45),
        "the wait the plugin asked for was replaced by the diagnostic"
    );
    let said = message.expect("what the plugin wrote reaches the caller");
    assert!(
        said.contains("the board is refusing bursts"),
        "the plugin's own words were dropped: {said}"
    );
    assert!(
        said.contains("the source rate-limited the request"),
        "the refusal it was appended to is no longer in it: {said}"
    );
}

#[cfg(unix)]
#[test]
fn a_rate_limited_handshake_from_a_silent_plugin_reads_exactly_as_it_did_before() {
    // The other half: a plugin that wrote nothing has nothing to add, and the refusal it
    // earns is the one shape every caller was already written against.
    let script = r#"read -r _request
printf '%s\n' '{"id":"0","error":{"kind":"rate-limited","retry_after_seconds":45}}'"#;
    let error = SubprocessSource::connect(
        "/bin/sh",
        &["-c".to_owned(), script.to_owned()],
        &name(),
        &json!({}),
        BTreeMap::new(),
    )
    .expect_err("a handshake the plugin rate-limited");

    assert_eq!(
        error,
        SourceError::RateLimited {
            retry_after_seconds: Some(45),
            message: None,
        }
    );
    assert_eq!(error.to_string(), "the source rate-limited the request");
}

#[cfg(unix)]
#[test]
fn a_spawned_plugin_inherits_no_unrelated_host_environment() {
    // `HOME` is deliberately not one of the named credentials handed to `connect`.
    // A real child shell reports which side of the boundary it observed through the
    // protocol's own error envelope, so this proves the spawned process's environment
    // rather than inspecting the `Command` that built it.
    let script = r#"read -r _request
if [ -z "${HOME+x}" ]; then
  printf '%s\n' '{"id":"0","error":{"kind":"auth","message":"environment cleared"}}'
else
  printf '%s\n' '{"id":"0","error":{"kind":"auth","message":"HOME leaked"}}'
fi"#;
    let error = SubprocessSource::connect(
        "/bin/sh",
        &["-c".to_owned(), script.to_owned()],
        &name(),
        &json!({}),
        BTreeMap::new(),
    )
    .expect_err("the probe refuses after reporting its environment");

    assert_eq!(
        error,
        SourceError::Auth {
            message: "environment cleared".to_owned()
        }
    );
}

#[tokio::test]
async fn a_source_is_named_in_diagnostics_without_its_connection() {
    let source = a_process_away(hosted_settings()).expect("the handshake succeeds");

    let shown = format!("{source:?}");
    assert!(shown.contains("in-memory"), "{shown}");
    assert!(
        !shown.contains("Connection"),
        "a live child and a forwarded credential stay out of it: {shown}"
    );
}

#[test]
fn a_handshake_answer_that_is_not_a_line_of_json_is_a_violation() {
    let Err(error) = scripted(vec!["not JSON at all".to_owned()]) else {
        panic!("a handshake that is not JSON builds nothing");
    };

    let SourceError::Malformed { message } = error else {
        panic!("a handshake line that is not JSON is a violation: {error:?}");
    };
    assert!(message.contains("not a response envelope"), "{message}");
}

#[tokio::test]
async fn an_answer_that_is_json_but_not_the_promised_shape_is_a_violation() {
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
        json!({"id": "1", "result": {"reachable": "yes please"}}).to_string(),
    ])
    .expect("the handshake succeeds");

    let SourceError::Malformed { message } = source.health().await.expect_err("a wrong shape")
    else {
        panic!("a result of the wrong shape is a violation");
    };
    assert!(message.contains("health"), "the method is named: {message}");
}

#[tokio::test]
async fn an_answer_carrying_both_members_after_the_handshake_is_a_violation() {
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
        json!({"id": "1", "result": {"reachable": true},
               "error": {"kind": "auth", "message": "no"}})
        .to_string(),
    ])
    .expect("the handshake succeeds");

    let SourceError::Malformed { message } = source.health().await.expect_err("both members")
    else {
        panic!("both members at once is a violation");
    };
    assert!(message.contains("both a result and an error"), "{message}");
}

/// A peer that records the handshake it was sent before answering it.
///
/// What crosses the wire is the thing worth asserting on: §3.1 says the handshake carries
/// only the variables this source's configuration names, and the only way to know that is
/// to read the line the engine actually wrote.
fn capturing(secrets: BTreeMap<String, String>) -> (SubprocessSource, Value) {
    let (to_engine, mut from_peer) = pipe().expect("a pipe");
    let (to_peer, from_engine) = pipe().expect("a pipe");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let recorder = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        let mut asked = BufReader::new(to_peer);
        let mut request = String::new();
        if asked.read_line(&mut request).is_err() {
            return;
        }
        recorder
            .lock()
            .expect("nothing else holds this")
            .push_str(&request);
        let answer = json!({"id": "0", "result": {"protocol_version": 2, "kind": "recorder",
                            "capabilities": capabilities()}});
        let _ = writeln!(from_peer, "{answer}");
        let _ = from_peer.flush();
    });
    let source = SubprocessSource::over(
        from_engine,
        to_engine,
        &name(),
        &json!({"root": "/somewhere"}),
        secrets,
    )
    .expect("the recorder answers the handshake");
    let line = seen.lock().expect("the thread has finished").clone();
    let handshake: Value = serde_json::from_str(line.trim()).expect("one JSON request");
    (source, handshake)
}

#[test]
fn the_handshake_carries_the_named_credentials_the_settings_and_the_source_name() {
    let mut secrets = BTreeMap::new();
    secrets.insert("LINEAR_API_KEY".to_owned(), "a value".to_owned());
    let (_source, handshake) = capturing(secrets);

    let params = &handshake["params"];
    assert_eq!(params["protocol_version"], 2);
    assert_eq!(params["source_name"], "work");
    assert_eq!(params["config"], json!({"root": "/somewhere"}));
    // Exactly the one named variable, so a plugin cannot be handed a credential its
    // configuration never asked for.
    assert_eq!(params["secrets"], json!({"LINEAR_API_KEY": "a value"}));
}

#[test]
fn a_handshake_carrying_no_named_credentials_forwards_none_at_all() {
    let (_source, handshake) = capturing(BTreeMap::new());

    assert_eq!(handshake["params"]["secrets"], json!({}));
}

#[test]
fn a_handshake_answer_addressed_to_another_request_is_a_violation() {
    let Err(error) = scripted(vec![
        json!({"id": "17", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
    ]) else {
        panic!("an answer to a request nobody sent builds nothing");
    };

    let SourceError::Malformed { message } = error else {
        panic!("an envelope addressed elsewhere is a violation: {error:?}");
    };
    assert!(
        message.contains("\"17\"") && message.contains("\"0\""),
        "{message}"
    );
}

#[test]
fn a_secret_name_that_could_never_be_a_variable_is_refused_at_the_field() {
    let Err(error) = subprocess_plugin().build(
        &name(),
        &json!({"command": "onetaskgraph-no-such-plugin-program", "secrets": ["not a name"]}),
        &NoSecrets,
    ) else {
        panic!("an unusable variable name builds nothing");
    };

    let SourceError::Config { message } = error else {
        panic!("an unusable variable name is a configuration refusal: {error:?}");
    };
    assert!(message.contains("not a name"), "{message}");
}

#[tokio::test]
async fn a_plugin_that_never_ends_its_line_has_the_connection_closed_on_it() {
    // One byte past the bound, and no newline: the shape of a peer that would otherwise
    // decide how much memory this process uses.
    let unbounded = "x".repeat(usize::try_from(MAX_LINE).expect("the bound fits a usize") + 1);
    let (to_engine, mut from_liar) = pipe().expect("a pipe");
    let (to_liar, from_engine) = pipe().expect("a pipe");
    std::thread::spawn(move || {
        let mut asked = BufReader::new(to_liar);
        let mut request = String::new();
        let _ = asked.read_line(&mut request);
        let answer = json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
                            "capabilities": capabilities()}});
        let _ = writeln!(from_liar, "{answer}");
        let _ = from_liar.flush();
        let mut second = String::new();
        let _ = asked.read_line(&mut second);
        let _ = from_liar.write_all(unbounded.as_bytes());
        let _ = from_liar.flush();
        // Held open: the engine must refuse on the length rather than on the close.
        let mut parked = String::new();
        let _ = asked.read_line(&mut parked);
    });
    let source =
        SubprocessSource::over(from_engine, to_engine, &name(), &json!({}), BTreeMap::new())
            .expect("the handshake succeeds");

    let SourceError::Malformed { message } = source.health().await.expect_err("an endless line")
    else {
        panic!("a line that never ends is a violation");
    };
    assert!(
        message.contains(&MAX_LINE.to_string()),
        "the refusal names the bound: {message}"
    );
}

#[test]
fn a_request_that_never_ends_its_line_closes_the_connection_rather_than_being_framed() {
    let mut input = serde_json::to_string(&handshake(2, hosted_settings())).expect("a request");
    input.push('\n');
    input.push_str(&"x".repeat(usize::try_from(MAX_LINE).expect("the bound fits a usize") + 1));

    let mut output: Vec<u8> = Vec::new();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(serve(input.as_bytes(), &mut output))
        .expect("an in-memory stream cannot fail");

    let answered = String::from_utf8(output).expect("UTF-8");
    // The handshake is answered; nothing after the unterminated line is, because nothing
    // after it can be framed as a request rather than as the tail of one.
    assert_eq!(answered.lines().count(), 1, "{answered}");
    assert!(answered.contains("\"protocol_version\":2"), "{answered}");
}

/// A `Capabilities` the scripted answers can carry, in the protocol's own spelling.
fn capabilities() -> Value {
    json!({
        "projects": "native",
        "orphan_tasks": "native",
        "filter_by_label": "native",
        "filter_by_status": "native",
        "search_title": "native",
        "search_content": "native",
        "task_dependencies": "both-directions",
        "project_dependencies": "both-directions",
        "max_page_size": 50
    })
}

/// A resolver holding exactly one variable.
struct OneSecret(&'static str);

impl SecretResolver for OneSecret {
    fn get(&self, var: &str) -> Option<SecretString> {
        (var == self.0).then(|| SecretString::from("a value nothing prints"))
    }
}

/// The `subprocess` plugin, as a configuration reaches it.
fn subprocess_plugin() -> Box<dyn onetaskgraph_plugin_api::SourcePlugin> {
    plugin_for("subprocess").expect("the subprocess plugin is registered")
}

#[test]
fn a_source_with_no_command_is_refused_before_anything_is_spawned() {
    let Err(error) = subprocess_plugin().build(&name(), &json!({"command": "   "}), &NoSecrets)
    else {
        panic!("a command that names nothing builds nothing");
    };

    let SourceError::Config { message } = error else {
        panic!("an empty command is a configuration refusal: {error:?}");
    };
    assert!(message.contains("command"), "{message}");
}

#[test]
fn a_subprocess_block_defaults_its_deadline_and_refuses_zero() {
    let config: SubprocessConfig =
        serde_json::from_value(json!({"command": "plugin"})).expect("the minimal block is valid");
    assert_eq!(config.deadline_ms, RequestDeadline::DEFAULT.milliseconds());

    let error =
        serde_json::from_value::<SubprocessConfig>(json!({"command": "plugin", "deadline_ms": 0}))
            .expect_err("zero is not a deadline");
    assert!(error.to_string().contains("nonzero u64"), "{error}");
}

#[test]
fn a_block_that_is_not_this_plugins_shape_is_refused_naming_the_source() {
    let Err(error) = subprocess_plugin().build(&name(), &json!({"command": 7}), &NoSecrets) else {
        panic!("a command of the wrong type builds nothing");
    };

    let SourceError::Config { message } = error else {
        panic!("a block of the wrong shape is a configuration refusal: {error:?}");
    };
    assert!(message.contains("work"), "the source is named: {message}");
}

#[test]
fn a_named_credential_nothing_defines_is_refused_naming_the_variable() {
    let Err(error) = subprocess_plugin().build(
        &name(),
        &json!({"command": "onetaskgraph-no-such-plugin-program",
                "secrets": ["LINEAR_API_KEY"]}),
        &NoSecrets,
    ) else {
        panic!("a credential nothing defines builds nothing");
    };

    let SourceError::Auth { message } = error else {
        panic!("an absent credential is an authentication refusal: {error:?}");
    };
    assert!(message.contains("LINEAR_API_KEY"), "{message}");
}

#[test]
fn a_named_credential_that_resolves_is_forwarded_and_the_run_gets_as_far_as_spawning() {
    let Err(error) = subprocess_plugin().build(
        &name(),
        &json!({"command": "onetaskgraph-no-such-plugin-program",
                "secrets": ["LINEAR_API_KEY"]}),
        &OneSecret("LINEAR_API_KEY"),
    ) else {
        panic!("the program does not exist, so nothing builds");
    };

    // Reaching the spawn is the assertion: a credential that resolved is one this build
    // never refused, and the only thing left to fail on is the program itself.
    assert!(
        matches!(error, SourceError::Unavailable { .. }),
        "resolution passed and the spawn failed: {error:?}"
    );
}
