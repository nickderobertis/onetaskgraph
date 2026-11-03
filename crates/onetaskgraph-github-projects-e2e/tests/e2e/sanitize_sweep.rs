//! `sources sanitize` against the loopback board: a sweep that searches rather than lists, the
//! classes of record a search cannot reach reported uncovered, and `--complete` reading the
//! board whole to cover them — every term the released onevcs's own, in an onevcs home this run
//! owns holding the invented private identity `hiddenco/quietharbor`.

use onetaskgraph_e2e_support::{
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    onevcs::OnevcsHome,
};
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
    assert!(
        report["requests"]["total"]
            .as_u64()
            .is_some_and(|total| total > 0)
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
