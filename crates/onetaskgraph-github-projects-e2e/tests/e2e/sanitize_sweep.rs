//! `sources sanitize` against the loopback board: a sweep that searches rather than lists, the
//! classes of record a search cannot reach reported uncovered, and `--complete` reading the
//! board whole to cover them — every term the released onevcs's own, in an onevcs home this run
//! owns holding the invented private identity `hiddenco/quietharbor`.

use onetaskgraph_e2e_support::{
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    onevcs::OnevcsHome,
};
use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

/// The invented private term, and what the store replaces it with.
const TERM: &str = "quietharbor";
const WITH: &str = "harbor-x";

/// One open task issue on the board, filed under no project.
fn issue(id: &str, title: &str, body: &str, origin: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,"body":body,
           "state":"OPEN","reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
           "status":"Todo","origin":origin,"labels":[]})
}

struct Board {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    home: OnevcsHome,
}

impl Board {
    /// A board holding one item per hit class: the term in a body, only in a comment, and
    /// only in the store's own origin value — beside an item carrying none.
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_items(
            &sandbox,
            vec![
                issue("X-901", "Body", &format!("Retry the way {TERM} does."), ""),
                issue("X-902", "Said", "Nothing private here.", ""),
                issue(
                    "X-903",
                    "Origin",
                    "Nothing private here.",
                    &format!("plan:{TERM}-retry"),
                ),
                issue("X-904", "Clean", "Nothing private here.", ""),
            ],
        );
        board.add_comment("X-902", &format!("Asked {TERM} first."));
        Self::configured(sandbox, config, board)
    }

    fn configured(sandbox: Sandbox, config: Value, board: GitHubBoardFields) -> Self {
        let mut home = OnevcsHome::new(&sandbox);
        // The repository every board item names is public; the term's is private.
        home.private("hiddenco", TERM)
            .public("nickderobertis", "onetaskgraph");
        let mut configured: Value = serde_json::from_str(&document(&json!({
            "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
        })))
        .expect("a configuration document");
        configured["write_policy"] =
            home.sanitizing_write_policy(json!([{ "term": TERM, "with": WITH }]));
        sandbox.project_document(&configured.to_string());
        Self {
            sandbox,
            board,
            home,
        }
    }

    /// `sources sanitize board` with `flags`: its exit code, report, stdout and stderr, and the
    /// documents the board was sent.
    fn sweep(&self, flags: &[&str]) -> (Option<i32>, Value, String, String, Vec<String>) {
        let mark = self.board.served_count();
        let output = self
            .sandbox
            .command()
            .args(["sources", "sanitize", "board", "--json"])
            .args(flags)
            .env("ONEVCS_HOME", self.home.path())
            .assert()
            .get_output()
            .clone();
        let served = self
            .board
            .served_since(mark)
            .into_iter()
            .map(|(query, variables)| format!("{query} {variables}"))
            .collect();
        (
            output.status.code(),
            serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            stdout(&output),
            stderr(&output),
            served,
        )
    }
}

fn ids(report: &Value) -> Vec<String> {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().unwrap_or("<redacted>").to_owned())
        .collect()
}

#[test]
fn a_search_sweep_rewrites_what_it_finds_reads_nothing_else_and_names_what_it_cannot_cover() {
    let board = Board::new();
    let (code, report, out, err, served) = board.sweep(&[]);
    assert_eq!(
        code,
        Some(3),
        "coverage is incomplete and nothing else failed: {out}{err}"
    );
    assert_eq!(report["complete"], json!(false), "{report}");
    assert_eq!(
        report["coverage"],
        json!({"complete": false, "uncovered": ["metadata", "comments", "normalized-spellings"]}),
        "{report}"
    );
    assert_eq!(ids(&report), ["board:X-901"], "{report}");
    assert_eq!(
        report["items"][0]["outcome"],
        json!("rewritten"),
        "{report}"
    );
    assert!(
        board
            .board
            .body("X-901")
            .as_str()
            .is_some_and(|body| body.contains(&format!("Retry the way {WITH} does."))
                && !body.contains(TERM)),
        "{:?}",
        board.board.body("X-901")
    );
    // Searched, never listed: no document read the board's items, and the items its searches
    // did not return were never read or written.
    for document in &served {
        for unread in ["X-902", "X-903", "X-904"] {
            assert!(
                !document.contains(&format!("\"{unread}\"")),
                "an item no search returned was read: {document}"
            );
        }
    }
    assert_eq!(
        report["requests"]["total"].as_u64(),
        Some(served.len() as u64),
        "reported requests match the loopback server ledger: {served:?}"
    );
    assert!(
        board.board.comments_on("X-902")[0]["body"]
            .as_str()
            .is_some_and(|body| body.contains(TERM)),
        "the comment no search reaches is left as it was"
    );
    for said in [&out, &err] {
        assert!(!said.contains(TERM), "{said}");
    }
}

#[test]
fn a_complete_sweep_reads_the_board_whole_and_covers_every_class() {
    let board = Board::new();
    // A dry run first: every read, no write.
    let (code, report, out, err, _) = board.sweep(&["--complete", "--dry-run"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let mut found = ids(&report);
    found.sort();
    assert_eq!(
        found,
        ["board:X-901", "board:X-902", "board:X-903"],
        "{report}"
    );
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["outcome"] == json!("would-rewrite")),
        "{report}"
    );
    assert!(
        board
            .board
            .body("X-901")
            .as_str()
            .is_some_and(|body| body.contains(TERM))
    );

    let (code, report, out, err, _) = board.sweep(&["--complete"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(
        report["coverage"],
        json!({"complete": true, "uncovered": []}),
        "{report}"
    );
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["outcome"] == json!("rewritten")),
        "{report}"
    );
    let said = &board.board.comments_on("X-902")[0]["body"];
    assert!(
        said.as_str().is_some_and(
            |body| body.contains(&format!("Asked {WITH} first.")) && !body.contains(TERM)
        ),
        "{said}"
    );
    for said in [&out, &err] {
        assert!(!said.contains(TERM), "{said}");
    }
    // A rerun finds nothing left to write.
    let (code, report, _, _, _) = board.sweep(&["--complete"]);
    assert_eq!(code, Some(0));
    assert_eq!(report["items"], json!([]), "{report}");
}

#[test]
fn a_complete_sweep_repairs_later_item_and_comment_pages_before_claiming_coverage() {
    let sandbox = Sandbox::new();
    let items = (0..110)
        .map(|index| {
            issue(
                &format!("PAGE-{index}"),
                "Plain",
                if index == 0 || index == 109 {
                    TERM
                } else {
                    "Plain."
                },
                "",
            )
        })
        .collect();
    let (config, backend) = github_projects_with_items(&sandbox, items);
    backend.seed_comments(
        "PAGE-0",
        (0..25).map(|index| {
            if index == 24 {
                format!("Last comment names {TERM}.")
            } else {
                format!("Comment {index}.")
            }
        }),
    );
    let board = Board::configured(sandbox, config, backend);
    let (code, report, out, err, _) = board.sweep(&["--complete"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_eq!(report["coverage"], json!({"complete":true,"uncovered":[]}));
    for id in ["PAGE-0", "PAGE-109"] {
        let body = board.board.body(id);
        assert!(
            body.as_str()
                .is_some_and(|body| body.lines().next() == Some(WITH) && !body.contains(TERM)),
            "{id}: {body}"
        );
        assert!(
            report["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == format!("board:{id}") && item["outcome"] == "rewritten")
        );
    }
    assert_eq!(
        board.board.comments_on("PAGE-0")[24]["body"],
        format!("Last comment names {WITH}.")
    );
    assert!(!out.contains(TERM) && !err.contains(TERM));
}

/// The budgets' board: `size` items with comments, and the term-carrying items of
/// `sanitize-sweep-requests` and `sanitize-complete-sweep-requests` among them.
///
/// Six searchable terms: the three onevcs derives from the invented private identity and three
/// a store's own replacements add. Nine items carry one the searches find — five in the body,
/// one in the body and a comment, one beside a private repository and so unfixable, and two in
/// a comment alone — and two carry one only in a stored origin value or as a spelling with a
/// zero-width character inside it. Every item holds `index % 21` comments, or with `deep`, the
/// two per cent at every fiftieth item hold 25.
pub(super) struct Workload {
    board: Board,
    pub(super) size: usize,
    deep: bool,
    runs: std::cell::Cell<usize>,
}

impl Workload {
    pub(super) fn new(size: usize, deep: bool) -> Self {
        let sandbox = Sandbox::new();
        let mut items: Vec<Value> = (0..size)
            .map(|n| {
                issue(
                    &format!("W-{n}"),
                    &format!("Item {n}"),
                    "Nothing private here.",
                    "",
                )
            })
            .collect();
        let hits: [(usize, &str, &str); 9] = [
            (11, "Retry as emberline does.", ""),
            (222, "Retry as emberline did.", ""),
            (333, "Ask duskfield first.", ""),
            (444, "Ask pinecrest first.", ""),
            (555, &*format!("Mirror {TERM} here."), ""),
            (666, "Nothing private here.", "Seen in emberline."),
            (777, "Nothing private here.", "Seen in duskfield."),
            (888, "Ask duskfield too.", "And pinecrest."),
            (999, &*format!("Mirror {TERM} there."), ""),
        ];
        for (index, body, _) in &hits {
            items[*index]["body"] = json!(body);
        }
        // The unfixable one names the private repository.
        items[999]["repo"] = json!(format!("hiddenco/{TERM}"));
        items[1111]["origin"] = json!("plan:pinecrest-1");
        items[1222]["body"] = json!("Mirror ｑｕｉｅｔ\u{200b}ｈａｒｂｏｒ here.");
        for index in [1300, 1400, 1500] {
            items[index]["body"] = json!("A former emberline reference.");
        }
        let (config, board) = github_projects_with_items(&sandbox, items);
        board.freeze_search_index();
        for index in [1300, 1400, 1500] {
            board.edit_body(&format!("W-{index}"), "An ordinary public description.");
        }
        if !deep {
            for id in ["W-11", "W-222", "W-333"] {
                board.after_request(
                    move |query, variables| {
                        matches!(query, graphql::ISSUE_DETAILS | graphql::ISSUE_COPY_DETAILS)
                            && variables
                                .as_object()
                                .is_some_and(|values| values.values().any(|v| v == id))
                    },
                    0,
                    move |board| {
                        let body = board.body(id);
                        board.edit_body(
                            id,
                            &format!("{}\nPerson's first paragraph.", body.as_str().unwrap()),
                        );
                    },
                );
            }
            board.after_request(
                |query, variables| query == graphql::ISSUE && variables["id"] == "W-333",
                0,
                |board| {
                    let body = board.body("W-333");
                    board.edit_body(
                        "W-333",
                        &format!("{}\nPerson's second paragraph.", body.as_str().unwrap()),
                    );
                },
            );
        }
        for n in 0..size {
            let count = if deep {
                if n % 50 == 0 { 25 } else { n % 21 }
            } else {
                n % 21
            };
            board.seed_comments(
                &format!("W-{n}"),
                (0..count).map(|c| format!("Comment {c}.")),
            );
        }
        for (index, _, said) in &hits {
            if !said.is_empty() {
                board.add_comment(&format!("W-{index}"), said);
            }
        }
        let mut home = OnevcsHome::new(&sandbox);
        home.private("hiddenco", TERM)
            .public("nickderobertis", "onetaskgraph");
        let mut configured: Value = serde_json::from_str(&document(&json!({
            "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
        })))
        .expect("a configuration document");
        configured["write_policy"] = home.sanitizing_write_policy(json!([
            { "term": TERM, "with": WITH },
            { "term": "emberline", "with": "ember-x" },
            { "term": "duskfield", "with": "dusk-x" },
            { "term": "pinecrest", "with": "pine-x" },
        ]));
        sandbox.project_document(&configured.to_string());
        Self {
            board: Board {
                sandbox,
                board,
                home,
            },
            size,
            deep,
            runs: std::cell::Cell::new(0),
        }
    }

    /// Measure the CLI report's source-request total and price the served documents with
    /// the provider's own worst-case GraphQL table, without sending another request.
    pub(super) fn measure(&self, flags: &[&str]) -> (u64, String) {
        let mark = self.board.board.served_count();
        let report = self.sweep(flags);
        let points: u64 = self
            .board
            .board
            .served_since(mark)
            .iter()
            .map(|(document, _)| {
                onetaskgraph_github_projects::worst_case_point_cost(document)
                    .expect("a document priced by the source's own table")
            })
            .sum();
        let (total, detail) = requests(&report);
        (
            total,
            format!("{detail}; {points} modelled worst-case GraphQL points"),
        )
    }

    /// One sweep's report, with `flags`.
    fn sweep(&self, flags: &[&str]) -> Value {
        let (code, report, out, err, _) = self.board.sweep(flags);
        assert!(
            matches!(code, Some(0 | 1 | 3)),
            "the sweep answered a report: {code:?} {out}{err}"
        );
        for private in [
            TERM,
            "hiddenco",
            "emberline",
            "duskfield",
            "pinecrest",
            "quiet\u{200b}harbor",
            "ｑｕｉｅｔ\u{200b}ｈａｒｂｏｒ",
        ] {
            assert!(
                !out.contains(private) && !err.contains(private),
                "{out}{err}"
            );
        }
        if !self.deep {
            let items = report["items"].as_array().expect("reported items");
            assert_eq!(items.len(), 10, "{report}");
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item["outcome"] == "not-a-match")
                    .count(),
                3,
                "{report}"
            );
            let conflicted = items
                .iter()
                .find(|item| item["id"] == "board:W-333")
                .expect("twice-moved item reported");
            assert_eq!(
                conflicted["outcome"],
                if self.runs.get() == 0 {
                    "conflict"
                } else {
                    "rewritten"
                },
                "{report}"
            );
            for id in ["W-11", "W-222", "W-333"] {
                let body = self.board.board.body(id);
                assert!(
                    body.as_str().unwrap().contains("Person's first paragraph."),
                    "{id}: {body}"
                );
            }
            assert!(
                self.board
                    .board
                    .body("W-333")
                    .as_str()
                    .unwrap()
                    .contains("Person's second paragraph.")
            );
            if self.runs.get() > 0 {
                assert_eq!(
                    report["requests"]["rewrite_operations"], 1,
                    "only the previously conflicted item is written: {report}"
                );
                assert_eq!(
                    items
                        .iter()
                        .filter(|item| item["outcome"] == "unchanged")
                        .count(),
                    5,
                    "{report}"
                );
            }
        }
        self.runs.set(self.runs.get() + 1);
        report
    }
}

/// `report`'s request figures, as a detail line.
fn requests(report: &Value) -> (u64, String) {
    let requests = &report["requests"];
    let total = requests["total"].as_u64().expect("a request total");
    (
        total,
        format!(
            "{total} source requests; operations: search/list pages {}, detail batches/comment walks {}, rewritten items {}; {} item(s) reported; coverage {}",
            requests["page_operations"],
            requests["detail_operations"],
            requests["rewrite_operations"],
            report["items"].as_array().map_or(0, Vec::len),
            report["coverage"],
        ),
    )
}
