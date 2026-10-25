//! How the binary paces its requests under GitHub's documented secondary limits, against a
//! loopback board that enforces them: at most 80 content creations in any minute and 500 in any
//! hour, and 2,000 secondary points in any minute, a mutation request five and a query one.
//!
//! Every journey here drives one process and holds each request it sent to one rule, read off
//! the board's own arrival record on the simulated clock and the pacer's record of each wait it
//! made — the cause and length the binary named on that clock: **a request leaves the moment the
//! one before it is answered, unless a window it charges has no room for its whole charge, and
//! then exactly when every one it charges does.** Each aliased mutation and each upload counts
//! one creation. And the board, counting each request at its arrival as GitHub does, refuses
//! none of them.
//!
//! The waits GitHub asks for — a `retry-after`, an empty primary bucket's reset — and the
//! backing off from a refusal naming none are held the same way: by their cause and length.

use super::asset_board::{Board, Call, GraphqlRefusal, creations, secondary_points};
use super::plan_copy::{Plan, SMALL, counterparts, success};
use onetaskgraph_e2e_support::{
    binary,
    clock::{SimulatedClock, Slept},
    common::{Sandbox, assert_spawnable_on_windows, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
};
use serde_json::{Value, json};
use std::{
    process::{Output, Stdio},
    time::Duration,
};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);

/// The comments the points journey lists: 2,000 of GitHub's largest pages and one more.
const COMMENTS: usize = 2_000 * onetaskgraph_github_projects::MAX_PAGE_SIZE as usize + 1;

/// The waits a window holds a request for, by the cause the pacer names each.
const WINDOWS: [&str; 3] = [
    "content-creation-minute",
    "content-creation-hour",
    "secondary-points-minute",
];

/// A folder holding a project and `tasks` tasks, beside a loopback board that enforces GitHub's
/// secondary windows, on a simulated clock.
struct Paced {
    sandbox: Sandbox,
    board: Board,
    github: GitHubBoardFields,
    clock: SimulatedClock,
}

impl Paced {
    fn new(tasks: usize, pacing: Value) -> Self {
        let clock = SimulatedClock::start(1);
        let sandbox = Sandbox::new();
        let (mut config, github) = github_projects_with_items(&sandbox, Vec::new());
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
        board.enforce_limit();
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = pacing;
        let root = sandbox.subdirectory("notes");
        for directory in ["projects", "tasks"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(
            root.join("projects/P.md"),
            "---\ntitle: A plan of many tasks\nstatus: todo\n---\nThe plan.\n",
        )
        .unwrap();
        for n in 1..=tasks {
            std::fs::write(
                root.join(format!("tasks/T-{n}.md")),
                format!("---\ntitle: Task {n}\nstatus: todo\nproject: P\n---\nStep {n}.\n"),
            )
            .unwrap();
        }
        sandbox.project_document(&document(&json!({
            "notes": {"plugin":"local-md","config":{"root":root}},
            "board": {"plugin":"github-projects","config": config}
        })));
        Self {
            sandbox,
            board,
            github,
            clock,
        }
    }

    /// One command through the binary on the clock: what it printed, and every request the
    /// board answered and every wait the binary made for it.
    fn run(&self, arguments: &[String]) -> (Output, Vec<Call>, Vec<Slept>) {
        let calls = self.board.calls().len();
        let slept = self.clock.slept().len();
        let lease = self.clock.lease(0);
        let mut command = self.sandbox.subprocess(binary());
        command
            .args(arguments)
            .envs(self.clock.client_env(0))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // What used to fail Windows alone: the spawn is held to its bound on every platform.
        assert_spawnable_on_windows(&command);
        let output = command.spawn().unwrap().wait_with_output().unwrap();
        lease.wait_for_detach();
        drop(lease);
        (
            output,
            self.board.calls()[calls..].to_vec(),
            self.clock.slept()[slept..].to_vec(),
        )
    }

    fn copy_project(&self) -> (Output, Vec<Call>, Vec<Slept>) {
        self.run(&[
            "project".into(),
            "copy".into(),
            "notes:P".into(),
            "--to".into(),
            "board".into(),
            "--json".into(),
        ])
    }
}

/// The earliest moment at or after `ready` at which `entries`, counted at their times, leave
/// room under `limit` for `wanted` more inside a window of `window`.
fn room(
    entries: &[(Duration, u64)],
    ready: Duration,
    window: Duration,
    limit: u64,
    wanted: u64,
) -> Duration {
    let mut inside: Vec<(Duration, u64)> = entries
        .iter()
        .copied()
        .filter(|(at, _)| *at + window > ready)
        .collect();
    let mut total: u64 = inside.iter().map(|(_, amount)| amount).sum();
    if wanted == 0 || total + wanted <= limit {
        return ready;
    }
    inside.sort();
    for (at, amount) in &inside {
        total -= amount;
        if total + wanted <= limit {
            return (*at + window).max(ready);
        }
    }
    ready
}

/// Hold one process's requests to the windows: each left the moment the one before it was
/// answered unless a window it charges had no room — counted, as the pacer counts, at each
/// earlier request's answer — and then exactly when every one did, the pacer naming that wait
/// by the window that held it longest and for exactly that long. None was refused. Answers how
/// many requests each window held.
fn held_exactly_until_room(
    calls: &[Call],
    slept: &[Slept],
) -> std::collections::BTreeMap<&'static str, usize> {
    assert!(
        calls.iter().all(|call| call.status != 403),
        "the board refused a request: {:?}",
        calls
            .iter()
            .filter(|call| call.status == 403)
            .map(|call| call.path.clone())
            .collect::<Vec<_>>()
    );
    let mut waits = slept
        .iter()
        .filter(|slept| WINDOWS.contains(&slept.cause.as_deref().unwrap_or_default()));
    let mut held = std::collections::BTreeMap::new();
    let mut counted: Vec<(Duration, u64)> = Vec::new();
    let mut priced: Vec<(Duration, u64)> = Vec::new();
    let mut previous: Option<Duration> = None;
    for call in calls {
        let charged = creations(&call.path, &call.bytes);
        let points = secondary_points(&call.path, &call.bytes);
        let ready = previous.unwrap_or(call.at);
        let mut free = (ready, None);
        for (entries, window, limit, wanted, cause) in [
            (
                &counted,
                MINUTE,
                onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE,
                charged,
                WINDOWS[0],
            ),
            (
                &counted,
                HOUR,
                onetaskgraph_github_projects::CONTENT_CREATION_PER_HOUR,
                charged,
                WINDOWS[1],
            ),
            (
                &priced,
                MINUTE,
                onetaskgraph_github_projects::SECONDARY_POINTS_PER_MINUTE,
                points,
                WINDOWS[2],
            ),
        ] {
            let at = room(entries, ready, window, limit, wanted);
            if at > free.0 {
                free = (at, Some(cause));
            }
        }
        assert_eq!(
            call.at, free.0,
            "{} ({charged} creations, {points} points) left at {:?}, and the windows had room at {:?}",
            call.path, call.at, free.0
        );
        if let Some(cause) = free.1 {
            let wait = waits
                .next()
                .unwrap_or_else(|| panic!("no wait was recorded for {cause}"));
            assert_eq!(wait.cause.as_deref(), Some(cause), "{wait:?}");
            assert_eq!(wait.length, free.0 - ready, "{wait:?}");
            assert_eq!(wait.at, ready, "{wait:?}");
            *held.entry(cause).or_default() += 1;
        }
        if charged > 0 {
            counted.push((call.answered, charged));
        }
        if points > 0 {
            priced.push((call.answered, points));
        }
        previous = Some(call.answered);
    }
    assert_eq!(waits.next(), None, "a pacing wait no window called for");
    held
}

#[test]
fn a_first_copy_of_the_plan_is_neither_refused_nor_held_by_the_boards_windows() {
    // The generated plan's first copy is about forty creations and far fewer points: no window
    // comes near its limit, so nothing waits, and the board refuses nothing.
    let plan = Plan::new(SMALL, Vec::new());
    plan.board.enforce_limit();
    let slept = plan.clock.slept().len();
    let (project, document, calls) = plan.copy();
    counterparts(&project, &document);
    assert!(
        calls.iter().all(|call| call.status != 403),
        "a request was refused"
    );
    let waits: Vec<Slept> = plan.clock.slept()[slept..].to_vec();
    assert_eq!(waits, Vec::new(), "the copy waited: {waits:?}");
}

#[test]
fn filling_the_minutes_creations_holds_the_next_until_the_oldest_have_left() {
    // Sixty tasks are sixty-one creates and sixty-one origin field values: past the minute's 80,
    // so the field values wait for the creates to leave it.
    let paced = Paced::new(60, json!({}));
    let (output, calls, slept) = paced.copy_project();
    success(&output);
    let held = held_exactly_until_room(&calls, &slept);
    assert!(
        held.get(WINDOWS[0]).is_some_and(|held| *held > 0),
        "{held:?}"
    );
    assert_eq!(held.get(WINDOWS[1]), None, "{held:?}");
}

#[test]
fn filling_the_hours_creations_holds_the_next_until_the_first_has_left_the_hour() {
    // Two hundred and sixty tasks are 521 creations and as many again: the minute paces them,
    // eighty at a time, until the hour's 500 are spent, and the rest wait for the first of those
    // to leave the hour.
    let paced = Paced::new(260, json!({}));
    let (output, calls, slept) = paced.copy_project();
    success(&output);
    let held = held_exactly_until_room(&calls, &slept);
    assert!(
        held.get(WINDOWS[0]).is_some_and(|held| *held > 0),
        "{held:?}"
    );
    assert!(
        held.get(WINDOWS[1]).is_some_and(|held| *held > 0),
        "{held:?}"
    );
}

#[test]
fn filling_the_minutes_points_with_queries_alone_holds_the_next_query() {
    // One task's comments listed past 2,000 of GitHub's largest pages are 2,001 page reads, a
    // point each, sent back to back on a board answering at once: the 2,001st waits for the
    // first to leave the minute. A short command line reaches all of them, so the spawn stays
    // inside the length Windows allows a child's command line.
    let paced = Paced::new(0, json!({}));
    paced.board.hold_answers_for(Duration::ZERO);
    // Inside one second: more comments than the board's one-day clock stamps a second apart.
    paced.github.hold_clock();
    for n in 0..COMMENTS {
        paced.github.add_comment("T-3", &format!("comment {n}"));
    }
    paced.github.release_clock();
    let arguments: Vec<String> = ["task", "comment", "list", "board:T-3", "--json"]
        .map(String::from)
        .to_vec();
    let (output, calls, slept) = paced.run(&arguments);
    success(&output);
    assert_eq!(calls.len(), 2_001);
    assert!(
        calls
            .iter()
            .all(|call| creations(&call.path, &call.bytes) == 0)
    );
    let held = held_exactly_until_room(&calls, &slept);
    assert_eq!(held.get(WINDOWS[2]), Some(&1), "{held:?}");
    assert_eq!(held.len(), 1, "{held:?}");
}

#[test]
fn a_nineteen_alias_write_with_79_creations_counted_waits_until_eighteen_have_left_the_minute() {
    // The project and its 78 tasks are 79 creates — the project's own, then seven requests of
    // eleven and one of one — and the first request of their field values and edges is nineteen
    // more: 98, eighteen past the minute. It is held until eighteen counted creations have
    // left: the project's create and the first two requests of eleven, the second of which is
    // the one that made room.
    let paced = Paced::new(78, json!({}));
    let (output, calls, slept) = paced.copy_project();
    success(&output);
    held_exactly_until_room(&calls, &slept);
    let creates: Vec<&Call> = calls
        .iter()
        .filter(|call| creations(&call.path, &call.bytes) > 0)
        .collect();
    let counted: Vec<u64> = creates
        .iter()
        .map(|call| creations(&call.path, &call.bytes))
        .collect();
    assert_eq!(
        &counted[..10],
        &[1, 11, 11, 11, 11, 11, 11, 11, 1, 19],
        "{counted:?}"
    );
    let second_eleven = creates[2];
    let held = creates[9];
    assert_eq!(held.at, second_eleven.answered + MINUTE);
    let wait = slept
        .iter()
        .find(|slept| slept.cause.as_deref() == Some(WINDOWS[0]))
        .expect("the request was held");
    assert_eq!(wait.at + wait.length, held.at, "{wait:?}");
    // Held from the moment the request before it — the read of the created items' board
    // items — was answered, which is when it would otherwise have left.
    let before = calls
        .iter()
        .position(|call| std::ptr::eq(call, held))
        .expect("the held request");
    assert_eq!(wait.at, calls[before - 1].answered, "{wait:?}");
}

#[test]
fn a_configured_interval_still_spaces_mutations_on_top_of_the_windows() {
    let paced = Paced::new(3, json!({"min_mutation_interval_ms": 750}));
    let (output, calls, slept) = paced.copy_project();
    success(&output);
    let mutations: Vec<&Call> = calls
        .iter()
        .filter(|call| secondary_points(&call.path, &call.bytes) == 5)
        .collect();
    assert!(mutations.len() > 2, "{}", mutations.len());
    for pair in mutations.windows(2) {
        assert!(
            pair[1].at >= pair[0].answered + Duration::from_millis(750),
            "{} left {:?} after the mutation before it finished",
            pair[1].path,
            pair[1].at - pair[0].answered
        );
    }
    assert!(
        slept
            .iter()
            .filter(|slept| slept.cause.as_deref() == Some("min-mutation-interval"))
            .all(|slept| slept.length <= Duration::from_millis(750))
            && slept
                .iter()
                .any(|slept| slept.cause.as_deref() == Some("min-mutation-interval")),
        "{slept:?}"
    );
}

/// A board refusing the copy's `createIssue` with `headers` and `body`, `times` times in a row.
fn refused_repeatedly(
    headers: &str,
    body: &str,
    times: usize,
) -> (Paced, Output, Vec<Call>, Vec<Slept>) {
    let paced = Paced::new(1, json!({}));
    paced.board.refuse_graphql(GraphqlRefusal {
        matching: "createIssue(input:$input)",
        status: 403,
        headers: headers.to_owned(),
        body: body.to_owned(),
        remaining: times,
    });
    let (output, calls, slept) = paced.copy_project();
    (paced, output, calls, slept)
}

/// The project's `createIssue` requests, in the order the board received them.
fn creates_of(calls: &[Call]) -> Vec<&Call> {
    calls
        .iter()
        .filter(|call| String::from_utf8_lossy(&call.bytes).contains("createIssue(input:$input)"))
        .collect()
}

#[test]
fn a_secondary_refusal_naming_a_retry_after_is_waited_out_for_that_long_then_resent() {
    let (paced, output, calls, slept) = refused_repeatedly(
        "Retry-After: 7\r\n",
        r#"{"message":"You have exceeded a secondary rate limit"}"#,
        1,
    );
    let report = success(&output);
    assert_eq!(report["items"][0]["action"], "created", "{report}");
    let waits: Vec<&Slept> = slept.iter().filter(|slept| slept.cause.is_some()).collect();
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(waits[0].cause.as_deref(), Some("retry-after"));
    assert_eq!(waits[0].length, Duration::from_secs(7));
    let creates = creates_of(&calls);
    assert_eq!(creates[0].status, 403);
    assert_eq!(creates[1].at, creates[0].answered + Duration::from_secs(7));
    assert!(paced.github.item("ISSUE-1").is_some());
}

#[test]
fn a_secondary_refusal_naming_no_wait_is_retried_on_a_backoff_doubling_from_a_second() {
    let (_, output, calls, slept) = refused_repeatedly(
        "",
        r#"{"message":"You have exceeded a secondary rate limit"}"#,
        3,
    );
    success(&output);
    let waits: Vec<(Option<&str>, Duration)> = slept
        .iter()
        .filter(|slept| slept.cause.is_some())
        .map(|slept| (slept.cause.as_deref(), slept.length))
        .collect();
    assert_eq!(
        waits,
        [1, 2, 4].map(|seconds| (Some("backoff"), Duration::from_secs(seconds)))
    );
    let creates = creates_of(&calls);
    for (pair, seconds) in creates.windows(2).zip([1, 2, 4]) {
        assert_eq!(pair[1].at, pair[0].answered + Duration::from_secs(seconds));
    }
}

#[test]
fn an_empty_primary_bucket_is_waited_out_until_its_reset() {
    let reset = chrono::Utc::now().timestamp() + 30;
    let (_, output, _, slept) = refused_repeatedly(
        &format!("x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {reset}\r\n"),
        r#"{"message":"API rate limit exceeded for user ID 1."}"#,
        1,
    );
    success(&output);
    let waits: Vec<&Slept> = slept.iter().filter(|slept| slept.cause.is_some()).collect();
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(waits[0].cause.as_deref(), Some("primary-reset"));
    // The reset is a wall-clock second, which the source reads against the wall clock once the
    // refusal is in hand: the wait is to it, which a second boundary passed meanwhile shortens.
    assert!(
        [29, 30].map(Duration::from_secs).contains(&waits[0].length),
        "{waits:?}\n{}\n{}",
        stdout(&output),
        stderr(&output)
    );
}
