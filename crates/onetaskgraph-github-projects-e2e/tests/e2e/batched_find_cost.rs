//! One batched `task find` of a follow-up prepare's duplicate searches, through the binary
//! against a loopback board of the real board's size: what it answers, what it asks the board,
//! and — priced with the plugin's own table — the recording behind the
//! `github-batched-find-points` budget.
//!
//! The workload is the searches `spike-real-board-cost` measured `onefollowup prepare` sending
//! for ten drafts against the `followups` board: 77 searches of 74 texts, three asked twice,
//! each text matching as many issues as it matched there (`HITS`, in the order the spike
//! recorded them, one of them made to match exactly 21) and ten origin lookups beside them. A
//! text's hits are drawn from one board-wide order — GitHub's best match stands in for the
//! board's own order here — with a mild preference for later issues, so the issues the searches
//! keep at 21 overlap as the spike measured: 643 distinct. Every draw is integer arithmetic on
//! one seeded generator, so every platform builds the same board.
//!
//! The recording holds no threshold of its own: `budget_runner::report_batched_find` reads it
//! and hands the figure to onebudgetspec, which alone judges it. This journey asserts what the
//! find answers and what it asks, and no point figure.

use std::collections::{BTreeMap, BTreeSet};

use onetaskgraph_github_projects::{DETAIL_BATCH, SEARCH_BATCH, SEARCH_BATCH_ORIGINS, graphql};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::github_projects_with_items;

/// The budget the journey records for.
pub(super) const BUDGET: &str = "github-batched-find-points";
/// The workload it records, which the report refuses any other recording for.
pub(super) const WORKLOAD: &str = "one batched find of a ten-draft follow-up prepare's 77 \
    duplicate searches and 10 origin lookups on a board of 2,138 issues, 465 open";

/// Each text's matches on the board, in the order the spike recorded the texts
/// (`hits-per-query.json`), the one that matched 24 there matching exactly 21 here.
pub(super) const HITS: [usize; 74] = [
    0, 290, 0, 342, 364, 424, 8, 7, 0, 36, 7, 1, 515, 112, 1, 0, 50, 88, 44, 122, 233, 8, 45, 0,
    470, 85, 140, 490, 70, 33, 1, 0, 456, 385, 2, 27, 0, 234, 317, 94, 135, 321, 3, 6, 0, 1305,
    170, 173, 14, 328, 1, 18, 0, 91, 73, 70, 81, 41, 83, 1, 0, 21, 2, 1, 545, 90, 4, 3, 0, 73, 266,
    134, 33, 3,
];
/// The texts asked twice, as the spike's prepare asked them: `retry`, `prerequisite` and
/// `publication`.
pub(super) const ASKED_TWICE: [usize; 3] = [4, 28, 32];
/// The text that matches 1,305 issues, which keeps 250 — more than one page of GitHub's.
const KEEPS_250: usize = 45;
/// The text of 545 matches that keeps 100.
const KEEPS_100: usize = 64;
/// A text of 70 matches that keeps 1.
const KEEPS_1: usize = 55;
/// What every other search keeps.
const KEEPS: usize = 21;
/// The issues on the board, and how many of them are open.
pub(super) const ISSUES: usize = 2_138;
pub(super) const OPEN: usize = 465;
/// The origin lookups beside the searches, and the two of them a board issue carries.
pub(super) const ORIGINS: usize = 10;
const CARRIED: [(usize, usize); 2] = [(3, 100), (7, 1_500)];
/// The page size the find walks at: GitHub's page maximum, so a search keeping 250 takes three.
const PAGE_SIZE: usize = 100;
/// The issues the shared dataset puts on every loopback board, beside this journey's own.
const DATASET: usize = 9;
/// The seed and the weight offset whose draws keep 643 distinct issues at 21.
const SEED: u64 = 62;
const WEIGHT_OFFSET: u64 = 256;

/// One step of splitmix64: the next state and the value it gives.
fn splitmix(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (state, z ^ (z >> 31))
}

/// Each text's matches, by the board position of the issue — its place in every search's order
/// — drawn from one generator: each issue at weight `isqrt(position + WEIGHT_OFFSET)`.
pub(super) fn matches() -> Vec<BTreeSet<usize>> {
    let own = ISSUES - DATASET;
    let mut prefix = Vec::with_capacity(own);
    let mut total = 0_u64;
    for position in 0..own {
        total += (position as u64 + WEIGHT_OFFSET).isqrt();
        prefix.push(total);
    }
    let mut state = SEED;
    HITS.iter()
        .map(|count| {
            let mut chosen = BTreeSet::new();
            while chosen.len() < *count {
                let (next, value) = splitmix(state);
                state = next;
                let ticket = value % total;
                chosen.insert(prefix.partition_point(|sum| *sum <= ticket));
            }
            chosen
        })
        .collect()
}

/// The board id of the issue at `position`.
fn issue_id(position: usize) -> String {
    format!("BF-{position:04}")
}

/// The word text `text` searches for, which no other text's word holds.
fn word(text: usize) -> String {
    format!("fq{text:03}w")
}

/// The draft origin lookup `n` asks after.
fn origin(n: usize) -> String {
    format!("notes:draft-{n}")
}

/// What search `text` keeps.
fn keeps(text: usize) -> usize {
    match text {
        KEEPS_250 => 250,
        KEEPS_100 => 100,
        KEEPS_1 => 1,
        _ => KEEPS,
    }
}

/// The board: one open or closed issue per position, its body every word of the texts that
/// match it.
fn board_items(matched: &[BTreeSet<usize>], open: usize) -> Vec<Value> {
    let own = ISSUES - DATASET;
    (0..own)
        .map(|position| {
            let words: Vec<String> = matched
                .iter()
                .enumerate()
                .filter(|(_, hits)| hits.contains(&position))
                .map(|(text, _)| word(text))
                .collect();
            // Spread the open issues over the board rather than bunching them.
            let is_open = (position * 7_919) % own < open;
            let carried = CARRIED
                .iter()
                .find(|(_, at)| *at == position)
                .map(|(n, _)| origin(*n));
            json!({"item": format!("ITEM-{}", issue_id(position)), "id": issue_id(position),
                   "type": "Issue", "title": format!("Follow-up {position}"),
                   "body": format!("Observed while working. {}", words.join(" ")),
                   "state": if is_open { "OPEN" } else { "CLOSED" },
                   "reason": if is_open { Value::Null } else { json!("COMPLETED") },
                   "parent": null, "repo": "nickderobertis/onetaskgraph",
                   "status": if is_open { "Todo" } else { "Done" },
                   "origin": carried.unwrap_or_default(), "labels": []})
        })
        .collect()
}

/// The find request: every text's search in the spike's order, each text asked twice asked
/// again beside it, then the origin lookups.
fn request() -> Value {
    let mut queries = Vec::new();
    for text in 0..HITS.len() {
        let times = if ASKED_TWICE.contains(&text) { 2 } else { 1 };
        for time in 0..times {
            queries.push(
                json!({"key": format!("text-{text}-{time}"), "text": word(text),
                                "in": "both", "limit": keeps(text)}),
            );
        }
    }
    for n in 0..ORIGINS {
        queries.push(json!({"key": format!("origin-{n}"), "origin": origin(n)}));
    }
    json!({"schema_version": 1, "queries": queries})
}

/// The phase a request of the find is, by its document.
fn phase(document: &str, variables: &Value) -> &'static str {
    if document == graphql::SEARCH_BATCH {
        let searches = (0..SEARCH_BATCH).any(|slot| {
            variables[format!("ask{slot}")] == true
                && !variables[format!("search{slot}")]
                    .as_str()
                    .unwrap_or_default()
                    .contains(" in:body ")
        });
        if searches {
            "search rounds"
        } else {
            "origin lookups"
        }
    } else if document == graphql::ISSUE_DETAILS || document == graphql::ISSUE {
        "detail batches"
    } else {
        "other reads"
    }
}

/// The issues one detail read reads whole: a batch's distinct ids, whose unused slots repeat
/// its last, or a single read's one.
fn read_whole(document: &str, variables: &Value) -> Vec<String> {
    if document == graphql::ISSUE_DETAILS {
        let mut ids: Vec<String> = (0..DETAIL_BATCH)
            .filter_map(|slot| variables[format!("id{slot}")].as_str().map(str::to_owned))
            .collect();
        ids.dedup();
        ids
    } else if document == graphql::ISSUE {
        vec![variables["id"].as_str().expect("an id").to_owned()]
    } else {
        Vec::new()
    }
}

/// The qualified ids an answer holds, in order.
fn answered_ids(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .expect("the items")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn a_batched_find_of_a_prepares_searches_answers_each_whole_and_pages_each_to_its_limit() {
    let matched = matches();
    // The workload, as the spike measured it.
    let kept: BTreeSet<usize> = matched
        .iter()
        .flat_map(|hits| hits.iter().take(KEEPS))
        .copied()
        .collect();
    assert_eq!(
        kept.len(),
        643,
        "the searches keep 643 distinct issues at 21"
    );
    assert_eq!(HITS.iter().filter(|hits| **hits == 0).count(), 11);
    assert_eq!(HITS.iter().filter(|hits| **hits < 21).count(), 30);
    assert_eq!(HITS.iter().filter(|hits| **hits >= 21).count(), 44);
    assert_eq!(HITS.iter().filter(|hits| **hits == 21).count(), 1);
    assert_eq!(HITS.iter().filter(|hits| **hits >= 100).count(), 24);
    assert_eq!(HITS[KEEPS_250], 1_305);

    // The shared dataset's open issues count toward the board's.
    let probe = Sandbox::new();
    let (_, dataset) = github_projects_with_items(&probe, Vec::new());
    let dataset = dataset.issues();
    assert_eq!(dataset.len(), DATASET);
    let dataset_open = dataset
        .iter()
        .filter(|item| item["state"] == "OPEN")
        .count();
    let sandbox = Sandbox::new();
    let (config, board) =
        github_projects_with_items(&sandbox, board_items(&matched, OPEN - dataset_open));
    let held = board.issues();
    assert_eq!(
        held.iter().filter(|item| item["type"] == "Issue").count(),
        ISSUES
    );
    assert_eq!(
        held.iter().filter(|item| item["state"] == "OPEN").count(),
        OPEN
    );
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({
            "sources": {"board": {"plugin": "github-projects", "config": config.clone()}},
        }))
        .expect("a document"),
    );
    board.pass_time(120);

    let run = |arguments: &[&str], stdin: &str| {
        let output = sandbox
            .command()
            .args(arguments)
            .write_stdin(stdin)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str::<Value>(&stdout(&output)).expect("JSON")
    };
    let page_size = PAGE_SIZE.to_string();
    let mark = board.served_count();
    let started = std::time::Instant::now();
    let found = run(
        &[
            "task",
            "find",
            "board",
            "--queries",
            "-",
            "--json",
            "--page-size",
            &page_size,
        ],
        &request().to_string(),
    );
    let seconds = started.elapsed().as_secs_f64();
    let served = board.served_since(mark);

    // What it answers: each search exactly the board's matches in its order up to its limit,
    // whether more matched past it, and each origin the one issue copied from it.
    let answers: BTreeMap<String, &Value> = found["answers"]
        .as_array()
        .expect("the answers")
        .iter()
        .map(|answer| (answer["key"].as_str().expect("a key").to_owned(), answer))
        .collect();
    assert_eq!(answers.len(), 77 + ORIGINS);
    let mut answered: BTreeSet<String> = BTreeSet::new();
    for (text, hits) in matched.iter().enumerate() {
        let expected: Vec<String> = hits
            .iter()
            .take(keeps(text))
            .map(|position| format!("board:{}", issue_id(*position)))
            .collect();
        let times = if ASKED_TWICE.contains(&text) { 2 } else { 1 };
        for time in 0..times {
            let answer = answers[&format!("text-{text}-{time}")];
            assert_eq!(answered_ids(answer), expected, "text {text}");
            assert_eq!(answer["complete"], true, "text {text}: {answer}");
            assert_eq!(
                answer["limited"],
                hits.len() > keeps(text),
                "text {text}: more matched past its limit exactly when the board holds more"
            );
        }
        answered.extend(expected);
    }
    for n in 0..ORIGINS {
        let answer = answers[&format!("origin-{n}")];
        let expected: Vec<String> = CARRIED
            .iter()
            .filter(|(carried, _)| *carried == n)
            .map(|(_, position)| format!("board:{}", issue_id(*position)))
            .collect();
        assert_eq!(answered_ids(answer), expected, "origin {n}");
        assert_eq!(answer["complete"], true);
        assert_eq!(answer["limited"], false);
        answered.extend(expected);
    }
    // Each whole: every field as the board holds it, and every member exactly as a listing of
    // the whole board reads the same task.
    let listed = run(
        &[
            "task",
            "list",
            "--source",
            "board",
            "--all",
            "--json",
            "--page-size",
            "100",
        ],
        "",
    );
    let listed: BTreeMap<String, Value> = listed["items"]
        .as_array()
        .expect("the listing's items")
        .iter()
        .map(|item| (item["id"].as_str().expect("an id").to_owned(), item.clone()))
        .collect();
    let by_id: BTreeMap<String, &Value> = held
        .iter()
        .map(|item| {
            (
                format!("board:{}", item["id"].as_str().expect("an id")),
                item,
            )
        })
        .collect();
    for answer in answers.values() {
        for item in answer["items"].as_array().expect("the items") {
            let id = item["id"].as_str().expect("an id");
            let built = by_id[id];
            let task = &item["item"];
            assert_eq!(task["title"], built["title"], "{id}");
            assert_eq!(task["content"], built["body"], "{id}");
            assert_eq!(task["status"]["name"], built["status"], "{id}");
            assert_eq!(
                task["repositories"],
                json!(["github.com/nickderobertis/onetaskgraph"]),
                "{id}"
            );
            assert_eq!(*item, listed[id], "{id} answers what a listing reads of it");
        }
    }

    // What it asks: each search's pages, at most what it still lacks and GitHub's page
    // maximum, none once its limit is answered or it has ended — and each text once.
    let mut pages: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (document, variables) in &served {
        if document != graphql::SEARCH_BATCH {
            continue;
        }
        for slot in 0..SEARCH_BATCH {
            if variables[format!("ask{slot}")] != true {
                continue;
            }
            let search = variables[format!("search{slot}")]
                .as_str()
                .expect("a search")
                .to_owned();
            let after = variables[format!("after{slot}")]
                .as_str()
                .map_or(0, |after| after.parse().expect("a loopback cursor"));
            let first = variables[format!("first{slot}")]
                .as_u64()
                .expect("a page size") as usize;
            pages.entry(search).or_default().push((after, first));
        }
        for slot in 0..SEARCH_BATCH_ORIGINS {
            if variables[format!("origin{slot}")] == true {
                assert!(variables[format!("itemsFirst{slot}")].as_u64() <= Some(PAGE_SIZE as u64));
            }
        }
    }
    let mut rows_asked = 0;
    let mut rows_answered = 0;
    let mut pages_of = BTreeMap::new();
    let mut ended_at_limit = 0;
    for (text, hits) in matched.iter().enumerate() {
        let search = pages
            .iter()
            .find(|(search, _)| search.ends_with(&format!("\"{}\"", word(text))))
            .map(|(_, pages)| pages.clone())
            .unwrap_or_else(|| panic!("text {text} was never asked"));
        let limit = keeps(text);
        let mut held = 0;
        for (at, (after, first)) in search.iter().enumerate() {
            assert_eq!(
                *after, held,
                "text {text}: page {at} resumes where the last ended"
            );
            assert!(
                *first <= (limit - held).min(PAGE_SIZE),
                "text {text}: page {at} asked {first} rows lacking {}",
                limit - held
            );
            rows_asked += first;
            let answered_rows = (*first).min(hits.len() - held);
            rows_answered += answered_rows;
            held += answered_rows;
            let ended = held == hits.len() || held == limit;
            assert_eq!(
                ended,
                at + 1 == search.len(),
                "text {text}: asked nothing more once it ended or held its limit: {search:?}"
            );
        }
        ended_at_limit += usize::from(held == limit && hits.len() > limit);
        pages_of.insert(text, search.len());
        assert_eq!(
            search.len(),
            if text == KEEPS_250 { 3 } else { 1 },
            "text {text}: {search:?}"
        );
    }
    // Each issue a search hit is read whole in exactly one detail slot.
    let read: Vec<String> = served
        .iter()
        .flat_map(|(document, variables)| read_whole(document, variables))
        .collect();
    let distinct: BTreeSet<&String> = read.iter().collect();
    assert_eq!(distinct.len(), read.len(), "an issue read whole twice");
    let completed: BTreeSet<String> = distinct.iter().map(|id| format!("board:{id}")).collect();
    assert_eq!(
        completed, answered,
        "every hit read whole, and nothing else"
    );

    // The recording: every request priced with the plugin's own table.
    let mut phases: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for (document, variables) in &served {
        assert!(
            !document.trim_start().starts_with("mutation"),
            "a find writes nothing"
        );
        let points = onetaskgraph_github_projects::request_point_cost(document, variables)
            .expect("a document the plugin prices");
        let spent = phases.entry(phase(document, variables)).or_default();
        spent.0 += 1;
        spent.1 += points;
    }
    let points: u64 = phases.values().map(|(_, points)| points).sum();
    let by_phase: serde_json::Map<String, Value> = phases
        .iter()
        .map(|(phase, (requests, points))| {
            (
                (*phase).to_owned(),
                json!({"requests": requests, "points": points}),
            )
        })
        .collect();
    let searches_asked = 77;
    let texts_sent = pages.len() - ORIGINS;
    let packed = json!({"search rounds": 4, "detail batches": 40, "origin lookups": 1,
                        "points": 45});
    let recorded = json!({
        "board_issues": ISSUES,
        "open_issues": OPEN,
        "searches": searches_asked,
        "distinct_texts": HITS.len(),
        "texts_sent": texts_sent,
        "asked_twice": ASKED_TWICE.len(),
        "origin_lookups": ORIGINS,
        "kept_at_21": kept.len(),
        "limits": {"250": 1, "100": 1, "1": 1, "21": 77 - 3},
        "page_size": PAGE_SIZE,
        "phases": Value::Object(by_phase),
        "packed": packed,
        "rows_asked": rows_asked,
        "rows_answered": rows_answered,
        "pages_of_the_250_search": pages_of[&KEEPS_250],
        "pages_of_every_other_search": pages_of
            .iter()
            .filter(|(text, _)| **text != KEEPS_250)
            .map(|(_, pages)| *pages)
            .max(),
        "ended_at_limit": ended_at_limit,
        "distinct_hits_completed": completed.len(),
        "requests": served.len(),
        "seconds": seconds,
    });
    assert_eq!(texts_sent, HITS.len(), "each text asked twice sent once");
    let detail = format!(
        "one batched `task find` of 77 searches (74 texts, 3 asked twice) and 10 origin lookups \
         on a loopback board of 2,138 issues, pages of 100, priced with the plugin's table: {recorded}"
    );
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": points,
            "detail": detail,
            "workload": WORKLOAD,
            "recorded": recorded,
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
    eprintln!("{detail}");

    // Each answer's cursor resumes: the same find's batch asked of the source itself, whose
    // answers carry the cursor the command line walks internally, then every limited answer
    // walked on from it to the search's end.
    resume_every_cursor(&config, &matched, &found);
}

/// The token the fixture configuration names.
struct FixtureToken;

impl onetaskgraph_plugin_api::SecretResolver for FixtureToken {
    fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
        (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
    }
}

/// The find's own batch — every search at the page the find first asks it, beside the origin
/// lookups — asked of a source built from `config`, its first pages exactly what the command
/// line answered, and each answer that reports more resumed from its cursor at page sizes that
/// leave the next cursor anywhere inside a page, until the search ends: the pages together are
/// every match the board holds, in the board's order, nothing repeated and nothing skipped.
/// Then a cursor 22 rows deep inside a page a larger search of the same text shared, resumed
/// the same way.
fn resume_every_cursor(config: &Value, matched: &[BTreeSet<usize>], found: &Value) {
    use onetaskgraph_plugin_api::{
        PageRequest, SourceName, SourcePlugin as _, TaskQuery, TaskSource, TextFields, TextQuery,
    };
    let build = || {
        onetaskgraph_github_projects::Plugin
            .build(&SourceName::new("board").unwrap(), config, &FixtureToken)
            .expect("the plugin builds")
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let searching = |text: usize| TaskQuery {
        text: Some(TextQuery {
            terms: word(text),
            fields: TextFields::TitleOrContent,
        }),
        ..TaskQuery::default()
    };
    let every = |text: usize| -> Vec<String> {
        matched[text]
            .iter()
            .map(|position| issue_id(*position))
            .collect()
    };
    let ids = |tasks: &[onetaskgraph_plugin_api::Task]| -> Vec<String> {
        tasks.iter().map(|task| task.id.0.clone()).collect()
    };
    // Resume sizes cycled so cursors land at the page's start, deep inside it and at its end.
    const RESUMES: [u32; 4] = [37, 100, 1, 63];
    let walk_on = |source: &dyn TaskSource, text: usize, held: Vec<String>, cursor| {
        let mut walked = held;
        let mut cursor = Some(cursor);
        let mut turn = 0;
        while let Some(at) = cursor {
            let limit = RESUMES[turn % RESUMES.len()];
            turn += 1;
            assert!(turn <= 2 * HITS[text], "text {text}: the walk never ends");
            let page = runtime
                .block_on(source.query_tasks(
                    &searching(text),
                    &PageRequest {
                        cursor: Some(at),
                        limit,
                    },
                ))
                .unwrap_or_else(|error| panic!("text {text}: the cursor resumes: {error:?}"));
            assert!(page.items.len() <= limit as usize, "text {text}");
            walked.extend(ids(&page.items));
            cursor = page.next;
        }
        let distinct: BTreeSet<&String> = walked.iter().collect();
        assert_eq!(
            distinct.len(),
            walked.len(),
            "text {text}: a match repeated"
        );
        assert_eq!(
            walked,
            every(text),
            "text {text}: every match in order, none skipped"
        );
    };

    // The find's batch, in its order.
    let mut asks = Vec::new();
    let mut texts = Vec::new();
    for text in 0..HITS.len() {
        let times = if ASKED_TWICE.contains(&text) { 2 } else { 1 };
        for time in 0..times {
            let first = u32::try_from(keeps(text).min(PAGE_SIZE)).expect("a page size");
            asks.push((
                searching(text),
                PageRequest {
                    cursor: None,
                    limit: first,
                },
            ));
            texts.push(Some((text, time)));
        }
    }
    for n in 0..ORIGINS {
        asks.push((
            TaskQuery {
                origin: Some(origin(n)),
                ..TaskQuery::default()
            },
            PageRequest {
                cursor: None,
                limit: PAGE_SIZE as u32,
            },
        ));
        texts.push(None);
    }
    let source = build();
    let answers = runtime.block_on(source.query_tasks_batch(&asks));
    assert_eq!(answers.len(), asks.len());
    let mut resumed = 0;
    for ((answer, (_, page)), asked) in answers.into_iter().zip(&asks).zip(&texts) {
        let answer = answer.expect("the batch answers every search");
        let Some((text, time)) = *asked else {
            assert_eq!(answer.next, None, "an origin lookup is one page");
            continue;
        };
        let first = ids(&answer.items);
        let answered: Vec<String> = answer_items(found, &format!("text-{text}-{time}"))
            .into_iter()
            .take(page.limit as usize)
            .collect();
        assert_eq!(
            first, answered,
            "text {text}: the page the command line answered"
        );
        assert_eq!(
            answer.next.is_some(),
            HITS[text] > page.limit as usize,
            "text {text}: a cursor exactly when the board holds more"
        );
        match answer.next {
            Some(cursor) => {
                walk_on(source.as_ref(), text, first, cursor);
                resumed += 1;
            }
            None => assert_eq!(first, every(text), "text {text}: every match"),
        }
    }
    // Every search the board holds more of than its first page was walked on: the 43 texts
    // matching more than 21 — those keeping 1, 100 and 250 among them — and the three of them
    // asked twice once more each.
    assert_eq!(resumed, 43 + 3, "the answers that report more");

    // A cursor deep inside a shared page: the text the find keeps at 100 asked beside itself
    // kept at 22, by a source that has read nothing, so one page of 100 answers both and the
    // narrower stops 22 rows into it.
    let source = build();
    let mut answers = runtime.block_on(source.query_tasks_batch(&[
        (
            searching(KEEPS_100),
            PageRequest {
                cursor: None,
                limit: 100,
            },
        ),
        (
            searching(KEEPS_100),
            PageRequest {
                cursor: None,
                limit: 22,
            },
        ),
    ]));
    let narrow = answers.pop().unwrap().expect("the narrower search");
    let wide = answers.pop().unwrap().expect("the wider search");
    assert_eq!(ids(&wide.items), every(KEEPS_100)[..100]);
    assert_eq!(ids(&narrow.items), every(KEEPS_100)[..22]);
    walk_on(
        source.as_ref(),
        KEEPS_100,
        ids(&narrow.items),
        narrow.next.expect("more matched past 22"),
    );
}

/// The native ids of the command line's answer under `key`, in order.
fn answer_items(found: &Value, key: &str) -> Vec<String> {
    found["answers"]
        .as_array()
        .expect("the answers")
        .iter()
        .find(|answer| answer["key"] == key)
        .map(|answer| {
            answered_ids(answer)
                .into_iter()
                .map(|id| id.trim_start_matches("board:").to_owned())
                .collect()
        })
        .unwrap_or_else(|| panic!("no answer under {key}"))
}
