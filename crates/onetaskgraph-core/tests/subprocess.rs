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

use onetaskgraph_core::{SubprocessSource, plugin_for, serve};
use onetaskgraph_plugin_api::{
    Direction, LabelFilter, NativeId, PageRequest, ProjectFilter, ProjectQuery, SecretResolver,
    SourceError, SourceName, TaskQuery, TaskSource,
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
async fn a_hosted_source_reports_its_own_kind_and_its_own_capabilities() {
    let there = a_process_away(hosted_settings()).expect("the handshake succeeds");

    // The transport is not the kind: what a caller sees is the source that answered, so a
    // plan naming `in-memory` is telling the truth about where the rows came from.
    assert_eq!(there.kind(), "in-memory");
    assert_eq!(there.capabilities(), in_process().capabilities());
    assert_eq!(there.capabilities().max_page_size, 2);
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
    let answers = served(&[handshake(2, hosted_settings())]);

    assert_eq!(
        answers.len(),
        1,
        "the plugin exits after refusing: {answers:?}"
    );
    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("version 2") && because(&answers[0]).contains("version 1"),
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
        handshake(1, hosted_settings()),
        handshake(1, hosted_settings()),
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
    input.push_str(&serde_json::to_string(&handshake(1, hosted_settings())).expect("a request"));
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
    assert!(answered.contains("\"protocol_version\":1"), "{answered}");
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
        handshake(1, hosted_settings()),
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
        handshake(1, hosted_settings()),
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
    let answers = served(&[handshake(1, json!({"kind": "jira", "config": {}}))]);

    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("jira") && because(&answers[0]).contains("in-memory"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn settings_this_host_cannot_read_are_refused_for_what_they_are_missing() {
    let answers = served(&[handshake(1, json!({"config": {}}))]);

    assert_eq!(refusal(&answers[0]), "config");
    assert!(
        because(&answers[0]).contains("kind"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_hosted_plugin_that_refuses_to_build_answers_with_its_own_error() {
    let answers = served(&[handshake(1, json!({"kind": "linear", "config": {}}))]);

    // `linear` is registered and its source has not landed, so it refuses by name — which
    // is the case that proves a build failure crosses the wire as the plugin's own error
    // rather than as a transport failure.
    assert!(answers[0]["error"].is_object(), "{answers:?}");
    assert!(
        because(&answers[0]).contains("linear"),
        "{}",
        because(&answers[0])
    );
}

#[test]
fn a_plugin_answering_in_a_version_the_engine_did_not_ask_for_is_refused() {
    let error = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 2, "kind": "made-up",
               "capabilities": capabilities()}})
        .to_string(),
    ])
    .expect_err("a version the engine does not speak");

    let SourceError::Config { message } = error else {
        panic!("a version disagreement is a configuration refusal: {error:?}");
    };
    assert!(
        message.contains("version 1") && message.contains("version 2"),
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
        json!({"id": "0", "result": {"protocol_version": 1, "kind": "made-up",
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
        json!({"id": "0", "result": {"protocol_version": 1, "kind": "made-up",
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
async fn a_plugin_that_stops_answering_fails_this_call_and_every_later_one() {
    let source = scripted(vec![
        json!({"id": "0", "result": {"protocol_version": 1, "kind": "made-up",
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
fn a_spawned_program_that_does_not_speak_the_protocol_is_a_violation_rather_than_a_hang() {
    // `--list` makes the test binary print its own test names and exit: a real program,
    // really spawned, whose first line of standard output is not a response envelope.
    let Err(error) = not_a_plugin("--list") else {
        panic!("a program that is not a plugin cannot become a source");
    };

    let SourceError::Malformed { message } = error else {
        panic!("a line that is not an envelope is a protocol violation: {error:?}");
    };
    assert!(message.contains("not a response envelope"), "{message}");
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
        json!({"id": "0", "result": {"protocol_version": 1, "kind": "made-up",
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
        json!({"id": "0", "result": {"protocol_version": 1, "kind": "made-up",
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
