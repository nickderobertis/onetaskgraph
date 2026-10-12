//! What the board cache spends against what the same work spends without it, recorded as
//! telemetry for the `github-cache-points-not-above-uncached` budget.
//!
//! The workload is the budget's: a loopback board of eighty tasks, twenty of which carry one
//! to five comments and the follow-up metadata a pass narrows by, driven through four
//! sequences once with the cache off and once with it on — a cold detail read of the twenty,
//! a warm re-read pinned to what it returned, a narrowed listing followed by the full loads
//! of its twenty results after five changed, and a follow-ups comment pass of a listing, a
//! pinned read, forty comments and one verifying read. The stale-option fallback, the cache's
//! one named exception, is measured beside them and kept out of the figure. Points are the
//! plugin's own worst-case price of each document the board served.

use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

use crate::budget_runner::directory;
use crate::cache::{Cached, FOLLOWUPS, Followups, NARROWED, version_of, written_version};
use crate::fixtures::github_extra_task;

/// The budget this suite records.
pub(crate) const BUDGET: &str = "github-cache-points-not-above-uncached";
/// The workload it records, which the reporter holds the recording to.
pub(crate) const WORKLOAD: &str = "board-80-tasks-20-commented-four-sequences";
/// The four sequences, in the order measured.
pub(crate) const SEQUENCES: [&str; 4] = [
    "cold-read",
    "warm-reread",
    "search-then-loads",
    "comment-pass",
];

/// What a list of served requests cost: requests and points.
fn cost(served: &[(String, Value)]) -> Value {
    let points: u64 = served
        .iter()
        .map(|(document, _)| {
            worst_case_point_cost(document).expect("every document this source sends prices")
        })
        .sum();
    json!({"requests": served.len(), "points": points})
}

fn add(total: &mut Value, more: &Value) {
    for key in ["requests", "points"] {
        total[key] = json!(total[key].as_u64().unwrap_or(0) + more[key].as_u64().unwrap_or(0));
    }
}

/// The ids of the twenty follow-ups, qualified.
fn followup_ids() -> Vec<String> {
    (0..FOLLOWUPS)
        .map(|n| format!("board:{}", github_extra_task(n)))
        .collect()
}

/// `task show-many` of `ids`, pinned to `pins` when the cache is on.
fn show_many(ids: &[String], pins: &[(String, String)], cache: bool) -> Vec<String> {
    let mut arguments = vec!["task".to_owned(), "show-many".to_owned()];
    arguments.extend(ids.iter().cloned());
    if cache {
        for (id, version) in pins {
            arguments.push("--pin".to_owned());
            arguments.push(format!("{id}={version}"));
        }
    }
    arguments.push("--json".to_owned());
    arguments
}

fn run(followups: &Followups, arguments: &[String]) -> (Value, Vec<(String, Value)>) {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    followups.ok(&arguments)
}

/// Each detail's id and version, from a `show-many` or a listing.
fn versions(answer: &Value, key: &str) -> Vec<(String, String)> {
    answer[key]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            let task = if key == "details" {
                &entry["items"][0]
            } else {
                entry
            };
            (
                task["id"].as_str().expect("an id").to_owned(),
                task["item"]["version"]
                    .as_str()
                    .expect("a version")
                    .to_owned(),
            )
        })
        .collect()
}

// llmlint: ignore-block[budgets_reuse_gate_telemetry] The budget compares the same workload with the cache on and with it off, which no behavioural journey runs — the cache's journeys hold small boards to exact requests, cache on alone — and its task fixes the design: the e2e journeys record the four sequences in both modes, and the reporter only reads what they recorded.
/// The four sequences against one board, with the cache on or off: what each cost.
fn sequences(cache: bool) -> Vec<Value> {
    let followups = Followups::new(cache);
    let ids = followup_ids();
    let mut costs = Vec::new();

    // 1. A cold read of the twenty with their comments.
    let (cold, served) = run(&followups, &show_many(&ids, &[], cache));
    costs.push(cost(&served));
    let held = versions(&cold, "details");

    // 2. The same read, pinned to the versions it returned, nothing changed.
    let (warm, served) = run(&followups, &show_many(&ids, &held, cache));
    if cache {
        assert!(
            served.is_empty(),
            "a warm pinned re-read sends nothing: {served:?}"
        );
        assert!(
            warm["details"]
                .as_array()
                .unwrap()
                .iter()
                .all(|detail| detail["cache"]["served"] == "cache")
        );
    }
    costs.push(cost(&served));

    // 3. Five change, then a narrowed listing over its pages and the full loads of its twenty
    //    results, pinned to the versions the listing carried.
    for n in 0..5 {
        followups
            .board
            .edit_title(&github_extra_task(n * 3), "changed since the last pass");
    }
    let mut listing = vec!["task".to_owned(), "list".to_owned(), "--json".to_owned()];
    listing.extend(NARROWED.iter().map(|word| (*word).to_owned()));
    listing.extend(["--limit".to_owned(), "100".to_owned()]);
    let (listed, mut served) = run(&followups, &listing);
    let listed_versions = versions(&listed, "items");
    assert_eq!(listed_versions.len(), FOLLOWUPS);
    let listed_ids: Vec<String> = listed_versions.iter().map(|(id, _)| id.clone()).collect();
    let (_, loads) = run(&followups, &show_many(&listed_ids, &listed_versions, cache));
    served.extend(loads);
    costs.push(cost(&served));

    // 4. The follow-ups comment pass: a narrowed listing, a pinned read of its twenty, two
    //    comments on each, and one batched, unpinned read of them all as verification.
    let (listed, mut served) = run(&followups, &listing);
    let listed_versions = versions(&listed, "items");
    let (read, loads) = run(&followups, &show_many(&listed_ids, &listed_versions, cache));
    served.extend(loads);
    let mut current: Vec<(String, String)> = versions(&read, "details");
    let mut unvouched = 0_u64;
    for round in 0..2 {
        for (id, version) in &mut current {
            let mut arguments = vec![
                "task".to_owned(),
                "comment".to_owned(),
                "add".to_owned(),
                id.clone(),
                "--json".to_owned(),
            ];
            if cache {
                arguments.extend(["--expect".to_owned(), version.clone()]);
            }
            let before = followups.board.served().len();
            let output = followups
                .sandbox
                .command()
                .env("XDG_CACHE_HOME", &followups.cache_home)
                .args(&arguments)
                .write_stdin(format!("reply {round}"))
                .assert()
                .success()
                .get_output()
                .clone();
            served.extend(followups.board.served()[before..].to_vec());
            if cache {
                let written: Value =
                    serde_json::from_slice(&output.stdout).expect("the comment's JSON");
                match written["result"]["version"].as_str() {
                    Some(next) => *version = next.to_owned(),
                    None => unvouched += 1,
                }
            }
        }
    }
    let (_, verify) = run(&followups, &show_many(&listed_ids, &[], cache));
    served.extend(verify);
    let mut pass = cost(&served);
    pass["writes_left_unvouched"] = json!(unvouched);
    costs.push(pass);
    costs
}

/// The refresh read a written task costs when its write's response cannot vouch for a version,
/// as against GitHub itself, where a response is stamped in the second of the change it made:
/// an expected write answered before its task settles, then a read pinned to the version the
/// caller held, which the cache cannot answer and the board does.
fn refresh_after_unsettled_write() -> Value {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let version = version_of(&first);
    cached.board.read_as_it_changes(true);
    let (written, _) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &version,
        "--json",
    ]);
    let unvouched = written["result"]["version"].is_null();
    cached.board.read_as_it_changes(false);
    let before = cached.board.served().len();
    cached.ok(&["task", "show", "board:T-3", "--pin", &version, "--json"]);
    let mut refresh = cost(&cached.board.served()[before..]);
    refresh["version_unvouched"] = json!(unvouched);
    refresh
}

/// The stale-option fallback, with the cache on and off: a priority write after the option it
/// names was re-minted.
fn stale_option(cache: bool) -> Value {
    let cached = Cached::with(&json!({"enabled": cache}));
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let version = version_of(&first);
    cached.board.remint_priority_option("High");
    let before = cached.board.served().len();
    let mut arguments = vec!["task", "priority", "set", "board:T-3", "high", "--json"];
    if cache {
        arguments.extend(["--expect", &version]);
    }
    let (written, _) = cached.ok(&arguments);
    if cache {
        let _ = written_version(&written);
    }
    cost(&cached.board.served()[before..])
}

// llmlint: ignore-end[budgets_reuse_gate_telemetry]

/// A figure `copy_recopy`'s re-copy journeys recorded in this crate's telemetry, when they have.
fn copy_figure(name: &str) -> Value {
    std::fs::read(directory().join(format!("{name}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map_or(
            Value::Null,
            |recorded| json!({"requests": recorded["requests"], "points": recorded["points"]}),
        )
}

#[test]
fn the_cache_spends_no_more_board_points_than_no_cache_across_the_four_sequences() {
    let off = sequences(false);
    let on = sequences(true);
    let mut figure = i64::MIN;
    let mut breakdown = serde_json::Map::new();
    for (index, name) in SEQUENCES.iter().enumerate() {
        let difference =
            on[index]["points"].as_i64().unwrap() - off[index]["points"].as_i64().unwrap();
        figure = figure.max(difference);
        breakdown.insert(
            (*name).to_owned(),
            json!({"on": on[index], "off": off[index], "difference": difference}),
        );
    }
    let mut total_off = json!({"requests": 0, "points": 0});
    let mut total_on = json!({"requests": 0, "points": 0});
    for index in 0..SEQUENCES.len() {
        add(&mut total_off, &off[index]);
        add(&mut total_on, &on[index]);
    }
    let stale = json!({"on": stale_option(true), "off": stale_option(false)});
    let refresh = refresh_after_unsettled_write();
    let recopy = json!({
        "unchanged_recopy_beyond_listing": copy_figure("github-copy-recopy-unchanged"),
        "five_changed_recopy": copy_figure("github-copy-recopy-five-changed"),
    });
    let detail = format!(
        "largest difference, cache on minus cache off, in points over the four sequences: \
         {figure}; per sequence {}; all four, off {total_off} and on {total_on}; the \
         stale-option fallback, the cache's one named exception and outside the figure: {stale}; \
         otg-copy-comments' recorded re-copies: {recopy}; a written task whose response vouches \
         for no version costs one refresh read at its next pinned read, measured on a write \
         answered before its task settled as against GitHub itself: {refresh} — on this \
         loopback {} of forty comment writes were left so, against GitHub itself every one, \
         which the pass's verifying read already makes",
        Value::Object(breakdown.clone()),
        on[3]["writes_left_unvouched"]
    );
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": figure,
            "detail": detail,
            "workload": WORKLOAD,
            "sequences": Value::Object(breakdown),
            "stale_option": stale,
            "refresh_read_per_written_item": refresh,
            "recopy": recopy,
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
}
