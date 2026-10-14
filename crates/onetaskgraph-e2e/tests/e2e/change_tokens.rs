//! A task's change token, as every read of the binary reports it.
//!
//! The token is the source's promise that nothing in a task changed: its title, body, status,
//! priority, labels, board fields, metadata, project, repository, edges and comments. So each
//! journey here changes one task of a loopback board by exactly one of those kinds — the way a
//! person changes it on GitHub, through the board's own state — and holds the token to moving:
//! to a later one, by the store's own ordering, for a change one of the token's two times
//! carries, or to one the store does not order at all, for a change of its facets — a board
//! field value, a label — of its relationship count or of its repository, while every task the
//! change did not touch keeps the token it had — a dependency touching both of the tasks it
//! relates, as GitHub records it on both. `docs/plugin-protocol.md` §4.13d states what each
//! plugin's token keys on; the loopback board models exactly those six parts, and moves neither
//! time for a dependency or a project membership, as GitHub moves neither for a dependency.

use std::collections::BTreeMap;

use onetaskgraph_core::version::{VersionOrder, Versioned};
use onetaskgraph_core::{Config, Engine, Environment, GlobalId, Secrets};
use serde_json::{Value, json};

use crate::common::{Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, ROWS, SOURCE, document, github_projects_with_board};

/// The JSON a run printed, having exited zero.
fn answered(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` failed:\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// Every task `task list` reports, by qualified id, with the token it carries.
fn listed(sandbox: &Sandbox, extra: &[&str]) -> BTreeMap<String, Option<String>> {
    let mut arguments = vec!["task", "list", "--json", "--limit", "100"];
    arguments.extend_from_slice(extra);
    answered(sandbox, &arguments)["items"]
        .as_array()
        .expect("a list answers with items")
        .iter()
        .map(|task| {
            (
                task["id"].as_str().expect("a qualified id").to_owned(),
                task["item"]["version"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// A board configured as the source `board`, with its handle and an engine over the same
/// board — as `board` in process, and as `hosted` over the stdio plugin protocol — for the
/// store's own ordering of two of its tokens.
fn board() -> (Sandbox, GitHubBoardFields, Engine) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config}
    })));
    let credential = "GITHUB_PROJECTS_FIXTURE_TOKEN";
    let engine = Engine::build(
        &Config::from_document(json!({"sources": {
            "board": {"plugin": "github-projects", "config": config},
            "hosted": SourceBoundary::Subprocess.source_with_secrets(
                "github-projects",
                config.clone(),
                &[credential],
            ),
        }}))
        .expect("a valid configuration"),
        &Secrets::load(Environment::from_pairs([(credential, "test-token")]))
            .expect("the loopback's credential"),
    );
    (sandbox, board, engine)
}

/// How the token `a` of `item` stands to its token `b`, by the store's own ordering.
fn ordered(engine: &Engine, item: &str, a: &str, b: &str) -> VersionOrder {
    let item = GlobalId::from_str_checked(item);
    engine.compare_versions(
        Versioned {
            item: &item,
            version: a,
        },
        Versioned {
            item: &item,
            version: b,
        },
    )
}

/// One change a person makes to one task on the board.
type Change = Box<dyn Fn(&GitHubBoardFields)>;

#[test]
fn every_change_kind_moves_the_changed_tasks_token_and_no_other() {
    let (sandbox, board, engine) = board();
    // Two comments to edit and to delete, made before anything is read.
    let first = board.add_comment("T-3", "the first word");
    let second = board.add_comment("T-3", "the second word");
    let changes: Vec<(&str, Change)> = vec![
        (
            "a new comment",
            Box::new(|board| {
                board.add_comment("T-3", "a third word");
            }),
        ),
        (
            "an edited comment",
            Box::new(move |board| board.edit_comment(&first, "the first word, edited")),
        ),
        (
            "a deleted comment",
            Box::new(move |board| board.delete_comment(&second)),
        ),
        (
            "a body edit",
            Box::new(|board| board.edit_body("T-3", "a body a person rewrote")),
        ),
        (
            "a title edit",
            Box::new(|board| board.edit_title("T-3", "Gamma, renamed")),
        ),
        (
            "a status change",
            Box::new(|board| board.move_card("T-3", "Doing")),
        ),
        (
            "a priority change",
            Box::new(|board| board.assign_priority("T-3", Some("Low"))),
        ),
        (
            "a label change",
            Box::new(|board| board.set_labels("T-3", &["chore"])),
        ),
        (
            "a project field change",
            Box::new(|board| board.set_card_field("T-3", "Estimate", "3 days")),
        ),
        (
            "a metadata change",
            Box::new(|board| board.set_metadata("T-3", "caller.note", json!("noted"))),
        ),
        (
            "a dependency added",
            Box::new(|board| board.block("T-3", &["T-4".to_owned()])),
        ),
        (
            "a dependency removed",
            Box::new(|board| board.unblock("T-3", "T-2")),
        ),
        (
            "a dependency replaced",
            Box::new(|board| {
                board.unblock("T-3", "T-4");
                board.block("T-3", &["T-1".to_owned()]);
            }),
        ),
        (
            "filed under a project",
            Box::new(|board| board.file_under("T-3", Some("P-1"))),
        ),
        (
            "moved to another project",
            Box::new(|board| board.file_under("T-3", Some("P-2"))),
        ),
        (
            "taken out of its project",
            Box::new(|board| board.file_under("T-3", None)),
        ),
        (
            "the origin field rewritten",
            Box::new(|board| board.set_origin_field("T-3", "elsewhere:X-1")),
        ),
        (
            "a repository transfer",
            Box::new(|board| board.transfer("T-3", TRANSFERRED_TO)),
        ),
    ];
    for (kind, change) in changes {
        let before = listed(&sandbox, &["--source", "board"]);
        change(&board);
        let after = listed(&sandbox, &["--source", "board"]);
        let mut touched = vec!["board:T-3"];
        touched.extend(far_ends(kind));
        for id in &touched {
            let old = before[*id].clone().unwrap_or_else(|| {
                panic!("{kind}: a board task carries a token before: {before:?}")
            });
            let new = after[*id]
                .clone()
                .unwrap_or_else(|| panic!("{kind}: a board task carries a token after: {after:?}"));
            assert_ne!(old, new, "{kind}: two equal tokens of {id} across a change");
            assert_eq!(
                ordered(&engine, id, &new, &old),
                ordered_after(kind),
                "{kind}: how the store orders {id}'s token after the change"
            );
        }
        for (id, token) in &before {
            if !touched.contains(&id.as_str()) {
                assert_eq!(
                    &after[id], token,
                    "{kind}: {id} was not touched, so its token is the one it had"
                );
            }
        }
    }
}

/// The tasks besides T-3 a change of `kind` reaches: a dependency is a relationship of two
/// issues, and GitHub records it on both ends' timelines, so the far end's token moves too —
/// and a copy reads it again. A project membership moves the task's own token alone: the
/// project is no task and carries none. `copy_skip` reads the same set.
pub(crate) fn far_ends(kind: &str) -> &'static [&'static str] {
    match kind {
        "a dependency added" => &["board:T-4"],
        "a dependency removed" => &["board:T-2"],
        "a dependency replaced" => &["board:T-4", "board:T-1"],
        _ => &[],
    }
}

/// Where the change-kind journeys transfer T-3 to.
pub(crate) const TRANSFERRED_TO: &str = "nickderobertis/onetaskgraph-elsewhere";

/// How the store orders a task's token after a change of `kind` against the one before: later
/// for a change one of the token's times carries — a comment, the body, the title and the
/// metadata its body holds — and unordered for every other, which moves its facets, its
/// relationship count or its repository: a `gh3` token compares those and never orders them.
fn ordered_after(kind: &str) -> VersionOrder {
    match kind {
        "a new comment" | "an edited comment" | "a deleted comment" | "a body edit"
        | "a title edit" | "a metadata change" => VersionOrder::Later,
        _ => VersionOrder::Incomparable,
    }
}

/// `GlobalId::from_str`, for an id this module spells itself.
trait Checked {
    fn from_str_checked(text: &str) -> Self;
}

impl Checked for GlobalId {
    fn from_str_checked(text: &str) -> Self {
        text.parse().expect("a qualified id")
    }
}

#[test]
fn every_read_carries_the_token_its_source_gives() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let who = row.name;
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document(&sandbox));
        // Every row whose source spells a token: a board, a folder of Markdown and Linear's
        // `lin1`, each from what its read already answers.
        let tokened = matches!(row.plugin, "github-projects" | "local-md" | "linear");
        let listed = listed(&sandbox, &[]);
        assert!(!listed.is_empty(), "{who}: the dataset holds tasks");
        for (id, token) in &listed {
            assert_eq!(
                token.is_some(),
                tokened,
                "{who}: {id} carries a token exactly when its source gives one"
            );
            let shown = answered(&sandbox, &["task", "show", id, "--json"]);
            assert_eq!(
                shown["items"][0]["item"]["version"].as_str(),
                token.as_deref(),
                "{who}: `task show` reports the token `task list` did"
            );
        }
        let ids: Vec<&str> = listed.keys().map(String::as_str).collect();
        let mut arguments = vec!["task", "show-many", "--json"];
        arguments.extend(&ids);
        let many = answered(&sandbox, &arguments);
        for (detail, id) in many["details"]
            .as_array()
            .expect("details")
            .iter()
            .zip(&ids)
        {
            assert_eq!(
                detail["items"][0]["item"]["version"].as_str(),
                listed[*id].as_deref(),
                "{who}: `task show-many` reports the token `task list` did for {id}"
            );
        }
    }
}

#[test]
fn a_task_kept_for_its_comment_activity_carries_its_token() {
    // A board, whose search names the issues updated since the instant, and a folder of
    // Markdown, which reads each task's own comments.
    let (sandbox, board, _) = board();
    board.add_comment("T-1", "recent activity");
    let since = "2026-09-01T00:00:00Z";
    let kept = listed(&sandbox, &["--source", "board", "--commented-since", since]);
    assert_eq!(
        kept.keys().collect::<Vec<_>>(),
        vec!["board:T-1"],
        "{kept:?}"
    );
    assert!(kept["board:T-1"].is_some(), "{kept:?}");

    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("notes");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: A\nstatus: todo\n---\nBody.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"c1\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### comment — 2026-09-13T15:11:07Z\n\nRecent.\n\n<!-- /onetaskgraph:comment -->\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "local-md", "config": {"root": root}}
    })));
    let kept = listed(&sandbox, &["--commented-since", since]);
    assert!(
        kept["work:A"]
            .as_deref()
            .is_some_and(|token| token.starts_with("md1|")),
        "{kept:?}"
    );
}

#[test]
fn a_board_hosted_over_the_stdio_protocol_has_its_tokens_ordered_by_equality_alone() {
    // A source hosted over the protocol is asked for no ordering (§4.13d), so the store keeps
    // to the trait's default: one token is one version, and two different ones are unordered —
    // which a copy reads as changed, never as unchanged. The same two tokens from the board in
    // process are ordered by the plugin's own reading of them.
    let (sandbox, board, engine) = board();
    let before = listed(&sandbox, &["--source", "board"])["board:T-3"]
        .clone()
        .expect("a board task carries a token");
    board.add_comment("T-3", "a later word");
    let after = listed(&sandbox, &["--source", "board"])["board:T-3"]
        .clone()
        .expect("a board task carries a token");
    assert_eq!(
        ordered(&engine, "board:T-3", &after, &before),
        VersionOrder::Later
    );
    assert_eq!(
        ordered(&engine, "hosted:T-3", &after, &before),
        VersionOrder::Incomparable,
        "two tokens of a hosted source are never put in order"
    );
    assert_eq!(
        ordered(&engine, "hosted:T-3", &before, &before),
        VersionOrder::Equal,
        "one token of a hosted source is one version"
    );
}

#[test]
fn a_later_time_beside_fewer_relationship_events_is_no_version_read_after_the_other() {
    let (sandbox, board, engine) = board();
    // Two dependency events on T-3, so a version of it can claim fewer than it holds.
    board.block("T-3", &["T-4".to_owned()]);
    board.unblock("T-3", "T-4");
    let before = listed(&sandbox, &["--source", "board"])["board:T-3"]
        .clone()
        .expect("a board task carries a token");
    board.edit_body("T-3", "a body a person rewrote");
    let after = listed(&sandbox, &["--source", "board"])["board:T-3"]
        .clone()
        .expect("a board task carries a token");
    assert_eq!(
        ordered(&engine, "board:T-3", &after, &before),
        VersionOrder::Later,
        "the body edit is the later version"
    );
    // The edit's later times, comment count and repository, beside a relationship count behind
    // the earlier version's: GitHub's count only grows, so no read after the earlier one could
    // give it. The count is the fifth `|`-separated field, after the prefix and before the
    // repository.
    let mut fields: Vec<&str> = after.splitn(6, '|').collect();
    assert_eq!(fields.len(), 6, "a GitHub token's fields: {after}");
    assert_eq!(
        fields[4], "2",
        "the two relationship events made above: {after}"
    );
    fields[4] = "0";
    let crossed = fields.join("|");
    assert_eq!(
        ordered(&engine, "board:T-3", &crossed, &before),
        VersionOrder::Incomparable,
        "{crossed} against {before}"
    );
    assert_eq!(
        ordered(&engine, "board:T-3", &before, &crossed),
        VersionOrder::Incomparable,
        "and the other way round"
    );
}
