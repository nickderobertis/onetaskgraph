//! `Engine::end_command`: what a caller holding one engine for many units of work calls
//! between them, and what each source of this repository does with it.
//!
//! These drive the engine the binary links, as a long-lived caller — onepipeline's settlement
//! write-back worker — does, because the call exists only for a caller that outlives one
//! command: the binary's command is its whole process. Each source is reached across its real
//! boundary: the loopback GitHub board and Linear workspace over HTTP, a folder of Markdown on
//! disk, and the GitHub Projects plugin again in a second process over the stdio protocol, so
//! the call is proven to cross that pipe rather than stop at it.
//!
//! The GitHub journeys run each scenario twice, with the call and without it. Without it the
//! source answers the second write from what it held, which is the hazard the call removes;
//! a journey asserting only the first half would pass whether or not the call did anything.

use std::num::NonZeroU32;

use onetaskgraph_core::{
    ConfiguredSource, Engine, Filters, GlobalId, Paging, ProjectSelector, ResolvedSource, Secrets,
    TaskRequest,
};
use onetaskgraph_plugin_api::{
    MetadataKey, MetadataMatch, SourceName, Status, StatusCategory, TaskUpdate,
};
use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::fixtures::{GitHubBoardFields, github_projects_with_board, linear_workspace_with};
use crate::linear_vocabulary::{PROJECT_STATUSES, TEAM_STATES, example_mapping};

/// The fixtures' credentials and nothing of the host's.
fn secrets() -> Secrets {
    Secrets::load(onetaskgraph_core::Environment::from_pairs([
        ("GITHUB_PROJECTS_FIXTURE_TOKEN", "test-token"),
        ("LINEAR_API_KEY", "fixture-key"),
    ]))
    .expect("an environment with no credentials file")
}

fn engine(kind: &str, config: &Value) -> Engine {
    let name = SourceName::new("work").unwrap();
    let source = onetaskgraph_core::plugin_for(kind)
        .unwrap_or_else(|| panic!("{kind} is registered"))
        .build(&name, config, &secrets())
        .unwrap_or_else(|error| panic!("the {kind} source builds: {error}"));
    Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        ))],
        vec![name],
    )
}

/// The GitHub Projects plugin served by the shipped host in a child process, over the stdio
/// protocol, against the board `block` configures.
fn hosted_github(block: &Value) -> Value {
    json!({
        "command": onetaskgraph_e2e_support::binary(),
        "args": ["plugin-serve", "github-projects"],
        "secrets": ["GITHUB_PROJECTS_FIXTURE_TOKEN"],
        "settings": block,
    })
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

fn global(id: &str) -> GlobalId {
    id.parse().expect("a qualified id")
}

fn in_progress() -> Option<Status> {
    Some(Status {
        category: StatusCategory::InProgress,
        name: String::new(),
    })
}

/// A settlement-shaped update: a status, and one `onepipeline.*` key recording which turn.
fn settlement(turn: u32) -> TaskUpdate {
    let mut update = TaskUpdate {
        status: in_progress(),
        ..TaskUpdate::default()
    };
    update.metadata_set.insert(
        MetadataKey::new("onepipeline.settlement").unwrap(),
        json!({"turn": turn}),
    );
    update
}

/// What a person wrote into T-1's body between the two settlements, in place of the words
/// the fixture gave it.
const PERSONS_WORDS: &str = "the engine core, reworded by a person between two settlements";

/// Which of the two ways a caller drives one engine across two units of work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Between {
    EndsTheCommand,
    /// It does not, which is the hazard.
    Nothing,
}

/// Two settlements of T-1 through one engine over the board, while the board — as a person
/// would on GitHub — has T-1's body reworded and its card dragged back to `Todo` between
/// them. Answers the board's body and `Status` option for T-1 once both have landed.
fn two_settlements(
    engine: &Engine,
    board: &GitHubBoardFields,
    between: Between,
) -> (String, Option<String>) {
    let task = global("work:T-1");
    runtime().block_on(async {
        engine
            .update_task(&task, &settlement(1))
            .await
            .expect("the first settlement lands");
        assert_eq!(board.status("T-1").as_deref(), Some("Doing"));

        let body = board.body("T-1");
        let body = body.as_str().expect("a body");
        assert!(body.contains("\"onepipeline.settlement\""), "{body}");
        board.edit_body("T-1", &body.replacen("the engine core", PERSONS_WORDS, 1));
        board.move_card("T-1", "Todo");

        if between == Between::EndsTheCommand {
            engine.end_command().await.expect("the command ends");
        }
        engine
            .update_task(&task, &settlement(2))
            .await
            .expect("the second settlement lands");
    });
    let body = board.body("T-1").as_str().expect("a body").to_owned();
    (body, board.status("T-1"))
}

/// `config` turns the loopback board's block into `kind`'s configuration, so one journey
/// drives the plugin in process and again behind the stdio host.
fn the_second_settlement_reads_the_item_as_a_person_left_it(
    kind: &str,
    config: impl Fn(&Value) -> Value,
) {
    let sandbox = Sandbox::new();
    let (block, board) = github_projects_with_board(&sandbox);
    let (body, status) = two_settlements(
        &engine(kind, &config(&block)),
        &board,
        Between::EndsTheCommand,
    );
    assert!(
        body.contains(PERSONS_WORDS),
        "the second settlement kept the person's edit: {body}"
    );
    assert!(
        body.contains(r#""turn":2"#),
        "the second settlement landed: {body}"
    );
    assert_eq!(
        status.as_deref(),
        Some("Doing"),
        "the second settlement moved the card from where the person left it"
    );

    // The same two settlements without the call: the source answers the second from the
    // record the first left, so the person's words are written over and the card stays
    // where they dragged it. This is what makes the half above say something.
    let sandbox = Sandbox::new();
    let (block, board) = github_projects_with_board(&sandbox);
    let (body, status) = two_settlements(&engine(kind, &config(&block)), &board, Between::Nothing);
    assert!(
        !body.contains(PERSONS_WORDS),
        "without the call the held record would overwrite the edit: {body}"
    );
    assert_eq!(
        status.as_deref(),
        Some("Todo"),
        "without the call the held record says the card is already in Doing"
    );
}

#[test]
fn a_github_board_settlement_after_the_call_keeps_a_persons_edit_and_moves_the_card_from_where_it_is()
 {
    the_second_settlement_reads_the_item_as_a_person_left_it("github-projects", Value::clone);
}

#[test]
fn the_call_crosses_the_stdio_protocol_to_a_hosted_github_board() {
    the_second_settlement_reads_the_item_as_a_person_left_it("subprocess", hosted_github);
}

/// A page of every task on the board narrowed by `filters` and `metadata`.
fn listing(filters: Filters, metadata: Vec<MetadataMatch>) -> TaskRequest {
    TaskRequest {
        sources: Vec::new(),
        filters,
        project: ProjectSelector::Any,
        priorities: Vec::new(),
        commented_since: None,
        metadata,
        origin: None,
        include_members: false,
        paging: Paging {
            limit: NonZeroU32::new(100).expect("not zero"),
            token: None,
        },
    }
}

/// Refuses a source failure rather than reading it as an answer: a failed search lists nothing,
/// which would pass the half of a journey that expects nothing to match.
async fn listed(engine: &Engine, request: &TaskRequest) -> Vec<String> {
    let response = engine.tasks(request).await.expect("the listing runs");
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    response
        .items
        .iter()
        .map(|task| task.item.id.0.clone())
        .collect()
}

/// The in-progress tasks of one board listed twice through one engine, with T-1's card dragged
/// to `Doing` by a person between the two listings. Answers the second listing.
fn two_listings(kind: &str, config: impl Fn(&Value) -> Value, between: Between) -> Vec<String> {
    let sandbox = Sandbox::new();
    let (block, board) = github_projects_with_board(&sandbox);
    let engine = engine(kind, &config(&block));
    let doing = listing(
        Filters {
            statuses: vec![StatusCategory::InProgress],
            ..Filters::default()
        },
        Vec::new(),
    );
    runtime().block_on(async {
        assert_eq!(listed(&engine, &doing).await, ["T-4"]);
        board.move_card("T-1", "Doing");
        if between == Between::EndsTheCommand {
            engine.end_command().await.expect("the command ends");
        }
        listed(&engine, &doing).await
    })
}

/// The tasks one board holds at `onepipeline.settlement.run = "a"`, asked twice through one
/// engine: T-1 is written there, then a person rewrites that value in the issue's body to
/// `"b"`. Answers the second answer.
fn two_metadata_searches(
    kind: &str,
    config: impl Fn(&Value) -> Value,
    between: Between,
) -> Vec<String> {
    let sandbox = Sandbox::new();
    let (block, board) = github_projects_with_board(&sandbox);
    let engine = engine(kind, &config(&block));
    let at_a = listing(
        Filters::default(),
        vec![
            MetadataMatch::new("onepipeline.settlement", vec!["run".into()], "a")
                .expect("a location"),
        ],
    );
    let mut written = TaskUpdate::default();
    written.metadata_set.insert(
        MetadataKey::new("onepipeline.settlement").unwrap(),
        json!({"run": "a"}),
    );
    runtime().block_on(async {
        engine
            .update_task(&global("work:T-1"), &written)
            .await
            .expect("the key lands");
        assert_eq!(listed(&engine, &at_a).await, ["T-1"]);
        let body = board.body("T-1");
        let body = body.as_str().expect("a body");
        assert!(body.contains(r#""run":"a""#), "{body}");
        board.edit_body("T-1", &body.replace(r#""run":"a""#, r#""run":"b""#));
        if between == Between::EndsTheCommand {
            engine.end_command().await.expect("the command ends");
        }
        listed(&engine, &at_a).await
    })
}

/// A listing after the call reads the board and its search afresh; without it the same
/// listing answers from the board and search it read before.
fn a_board_listing_reads_a_moved_card_only_after_the_call(kind: &str, config: fn(&Value) -> Value) {
    assert_eq!(
        two_listings(kind, config, Between::EndsTheCommand),
        ["T-1", "T-4"],
        "the card the person moved is read where they left it"
    );
    assert_eq!(
        two_listings(kind, config, Between::Nothing),
        ["T-4"],
        "without the call the held board still has the card where it was"
    );
}

/// A narrowed search after the call asks GitHub again and reads the item as a person left
/// it; without it the answer comes from the search held and the record this source wrote.
fn a_metadata_search_reads_an_edited_slot_only_after_the_call(
    kind: &str,
    config: fn(&Value) -> Value,
) {
    assert!(
        two_metadata_searches(kind, config, Between::EndsTheCommand).is_empty(),
        "the value the person rewrote no longer matches"
    );
    assert_eq!(
        two_metadata_searches(kind, config, Between::Nothing),
        ["T-1"],
        "without the call the held answer still matches"
    );
}

#[test]
fn a_github_board_listing_after_the_call_reads_a_card_a_person_moved() {
    a_board_listing_reads_a_moved_card_only_after_the_call("github-projects", Value::clone);
    a_board_listing_reads_a_moved_card_only_after_the_call("subprocess", hosted_github);
}

#[test]
fn a_github_metadata_search_after_the_call_reads_a_value_a_person_rewrote() {
    a_metadata_search_reads_an_edited_slot_only_after_the_call("github-projects", Value::clone);
    a_metadata_search_reads_an_edited_slot_only_after_the_call("subprocess", hosted_github);
}

/// The example team's workspace over three issues in `Todo`, and the source configuration that
/// reaches it.
fn example_workspace(sandbox: &Sandbox) -> (Value, crate::fixtures::LinearWorkspace) {
    let tasks = ["L-1", "L-2", "L-3"]
        .iter()
        .map(|id| {
            json!({"id": id, "title": format!("Issue {id}"),
                   "content": "A person's own words.",
                   "status": {"name": "Todo", "category": "unknown"},
                   "_linear_state": {"name": "Todo", "type": "unstarted"},
                   "labels": [], "priority": "none"})
        })
        .collect::<Vec<_>>();
    let held = json!({"tasks": tasks, "projects": [], "documents": [], "labels": [],
                      "task_dependencies": [], "project_dependencies": []});
    let (mut config, workspace) =
        linear_workspace_with(sandbox, held, TEAM_STATES, PROJECT_STATUSES);
    config["status_mapping"] = example_mapping();
    (config, workspace)
}

#[test]
fn a_linear_source_keeps_its_resolution_vocabulary_across_the_call() {
    let sandbox = Sandbox::new();
    let (config, workspace) = example_workspace(&sandbox);
    let engine = engine("linear", &config);
    let mut sent = Vec::new();
    runtime().block_on(async {
        for (id, update) in [
            (
                "L-1",
                TaskUpdate {
                    status: in_progress(),
                    ..TaskUpdate::default()
                },
            ),
            (
                "L-2",
                TaskUpdate {
                    status: in_progress(),
                    ..TaskUpdate::default()
                },
            ),
            ("L-3", settlement(1)),
        ] {
            let from = workspace.served().len();
            engine
                .update_task(&global(&format!("work:{id}")), &update)
                .await
                .unwrap_or_else(|error| panic!("{id} lands: {error}"));
            engine.end_command().await.expect("the command ends");
            sent.push(
                workspace.served()[from..]
                    .iter()
                    .map(|(document, _)| document.clone())
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                workspace.state_of(id).as_deref(),
                Some("In Progress"),
                "{id}"
            );
        }
    });
    let resolution = onetaskgraph_linear::graphql::RESOLUTION;
    assert!(
        sent[0].iter().any(|document| document == resolution),
        "the first write reads the vocabulary: {:#?}",
        sent[0]
    );
    for after in &sent[1..] {
        assert!(
            !after.iter().any(|document| document == resolution),
            "a write after the call reads no team, workflow state or project status: {after:#?}"
        );
    }
    assert_eq!(
        sent[1].len(),
        1,
        "a warm status write is its mutation: {:#?}",
        sent[1]
    );
    assert!(
        sent[2].len() <= 2,
        "a warm settlement is a read and a mutation: {:#?}",
        sent[2]
    );
}

#[test]
fn a_markdown_status_write_after_the_call_keeps_a_persons_edit_to_the_file() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plan");
    for kind in ["tasks", "projects", "documents"] {
        std::fs::create_dir_all(root.join(kind)).expect("a folder");
    }
    let file = root.join("tasks/t1.md");
    std::fs::write(&file, "---\ntitle: One\nstatus: todo\n---\nThe body.\n").expect("the task");
    let engine = engine("local-md", &json!({"root": root}));
    let task = global("work:t1");
    runtime().block_on(async {
        engine
            .set_task_status(&task, StatusCategory::InProgress)
            .await
            .expect("the first status lands");
        let held = std::fs::read_to_string(&file).expect("the task");
        std::fs::write(
            &file,
            held.replace("The body.", "The body, edited by hand."),
        )
        .expect("a person's edit");
        engine.end_command().await.expect("the command ends");
        engine
            .set_task_status(&task, StatusCategory::Done)
            .await
            .expect("the second status lands");
    });
    let after = std::fs::read_to_string(&file).expect("the task");
    assert!(after.contains("The body, edited by hand."), "{after}");
    assert!(after.contains("status: done"), "{after}");
}

#[test]
fn an_in_memory_source_keeps_its_own_store_across_the_call() {
    let engine = engine(
        "in-memory",
        &json!({"tasks": [{"id": "T-1", "title": "Alpha",
                           "status": {"category": "todo", "name": "Todo"}, "labels": []}]}),
    );
    let task = global("work:T-1");
    let status = runtime().block_on(async {
        engine
            .set_task_status(&task, StatusCategory::Done)
            .await
            .expect("the status lands");
        // Its store is the work itself, not a copy of anybody else's, so the call leaves it.
        engine.end_command().await.expect("the command ends");
        engine.task(&task).await.expect("the task reads").items[0]
            .item
            .status
            .category
    });
    assert_eq!(status, StatusCategory::Done);
}
