//! The store's local cache of a GitHub board, driven through the binary against the loopback
//! board: what a pinned read is answered from, what a write leaves the cache holding, and
//! what a write made with `--expect` sends.
//!
//! Every journey counts the requests the board served for each command, because that is the
//! whole of what the cache is for and the one rule it may never break: **it never adds a
//! request, and never serves a task older than the version a caller pinned.** A pin means "at
//! least as fresh as the version I hold", not "current" — one journey proves a pinned read
//! served from the cache while the board holds a change nobody has read.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// One sandbox configuring the loopback board as `board`, caching it, with a folder of
/// Markdown beside it as `plans`.
pub(crate) struct Cached {
    pub(crate) sandbox: Sandbox,
    pub(crate) board: GitHubBoardFields,
    /// Where `XDG_CACHE_HOME` points for every command, so the default cache directory is
    /// inside the sandbox.
    pub(crate) cache_home: PathBuf,
    pub(crate) plans: PathBuf,
}

impl Cached {
    /// The board, with `cache` as its source's `cache` block.
    pub(crate) fn with(cache: &Value) -> Self {
        let sandbox = Sandbox::new();
        let plans = sandbox.subdirectory("plans");
        std::fs::create_dir_all(plans.join("tasks")).expect("the task folder");
        let (config, board) = github_projects_with_board(&sandbox);
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config,"cache":cache},
            "plans": {"plugin":"local-md","config":{"root": plans,
                "status_mapping": {"Todo":"todo","Doing":"in-progress"}}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        Self {
            sandbox,
            board,
            cache_home,
            plans,
        }
    }

    /// The board, cached in its default directory.
    pub(crate) fn new() -> Self {
        Self::with(&json!({"enabled": true}))
    }

    /// One command, with the sandbox's cache home.
    pub(crate) fn run_with(&self, arguments: &[&str], stdin: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command
            .env("XDG_CACHE_HOME", &self.cache_home)
            .args(arguments);
        if let Some(stdin) = stdin {
            command.write_stdin(stdin.to_owned());
        }
        command.assert().get_output().clone()
    }

    /// One command that had to succeed: what it printed, and every document the board served
    /// for it.
    pub(crate) fn ok_with(&self, arguments: &[&str], stdin: Option<&str>) -> (Value, Vec<String>) {
        let before = self.board.served().len();
        let output = self.run_with(arguments, stdin);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.board.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        let printed = serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
            panic!(
                "`onetaskgraph {}` printed no JSON ({error}): {}",
                arguments.join(" "),
                stdout(&output)
            )
        });
        (printed, served)
    }

    pub(crate) fn ok(&self, arguments: &[&str]) -> (Value, Vec<String>) {
        self.ok_with(arguments, None)
    }

    /// Every task entry the cache holds, under `dir`.
    pub(crate) fn entries(dir: &Path) -> Vec<PathBuf> {
        let Ok(listed) = std::fs::read_dir(dir.join("tasks")) else {
            return Vec::new();
        };
        let mut entries: Vec<PathBuf> = listed
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        entries.sort();
        entries
    }

    /// The one cache directory under the default cache home.
    pub(crate) fn default_dir(&self) -> PathBuf {
        let root = self.cache_home.join("onetaskgraph");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|listed| {
                listed
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            dirs.len(),
            1,
            "one cache directory under {}",
            root.display()
        );
        dirs.remove(0)
    }
}

/// The `version` a task read reported.
pub(crate) fn version_of(shown: &Value) -> String {
    shown["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("the read gave the task a version: {shown}"))
        .to_owned()
}

/// The `cache` member a read reported.
pub(crate) fn served_by(shown: &Value) -> Value {
    shown["cache"].clone()
}

fn from_cache() -> Value {
    json!({"served": "cache", "bypassed": null})
}

fn from_remote() -> Value {
    json!({"served": "remote", "bypassed": null})
}

#[test]
fn a_pinned_read_of_a_task_cached_at_that_version_sends_nothing_and_an_unpinned_one_goes_to_the_board()
 {
    let cached = Cached::new();
    let (first, sent) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(
        sent,
        [graphql::ISSUE_DETAIL],
        "a cold read is the one detail read"
    );
    assert_eq!(served_by(&first), from_remote());
    let version = version_of(&first);

    // Pinned to the version it holds: answered from disk, with nothing sent.
    let (pinned, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &version, "--json"]);
    assert_eq!(
        sent,
        Vec::<String>::new(),
        "a pinned read of a cached task sends nothing"
    );
    assert_eq!(served_by(&pinned), from_cache());
    assert_eq!(pinned["items"], first["items"]);
    assert_eq!(pinned["comments"], first["comments"]);
    assert_eq!(pinned["assets"], first["assets"]);

    // The comments alone, and many tasks at once, are answered the same way.
    let (comments, sent) = cached.ok(&[
        "task",
        "comment",
        "list",
        "board:T-3",
        "--pin",
        &version,
        "--json",
    ]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(comments["comments"], first["comments"]);
    assert_eq!(served_by(&comments), from_cache());
    let pin = format!("board:T-3={version}");
    let (many, sent) = cached.ok(&["task", "show-many", "board:T-3", "--pin", &pin, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(served_by(&many["details"][0]), from_cache());
    assert_eq!(many["details"][0]["items"], first["items"]);

    // Unpinned, the read goes to the board as it always did.
    let (unpinned, sent) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert_eq!(served_by(&unpinned), from_remote());
}

#[test]
fn a_pin_is_at_least_as_fresh_as_the_version_held_and_never_discovers_a_change_nobody_read() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let held = version_of(&first);
    // A person edits the task on the board; the store does not see it.
    cached.board.edit_title("T-3", "Gamma, renamed by a person");

    // Pinned to the version the caller holds, the cache answers with that version: a pin
    // means "at least as fresh as what I hold", and nothing was asked.
    let (pinned, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &held, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(served_by(&pinned), from_cache());
    assert_eq!(pinned["items"][0]["item"]["title"], "Gamma");

    // Read unpinned, the change is there, and the entry is refreshed with it.
    let (current, sent) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert_eq!(
        current["items"][0]["item"]["title"],
        "Gamma, renamed by a person"
    );
    let later = version_of(&current);
    assert_ne!(later, held);

    // Pinned to the earlier version now, the cache answers with the later one it holds.
    let (fresher, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &held, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(
        fresher["items"][0]["item"]["title"],
        "Gamma, renamed by a person"
    );
    assert_eq!(version_of(&fresher), later);
}

#[test]
fn a_pinned_read_the_cache_cannot_vouch_for_makes_exactly_the_one_remote_read() {
    let cached = Cached::new();
    // Never cached: one read, which fills the entry.
    let (listed, _) = cached.ok(&["task", "list", "--source", "board", "--json"]);
    let version = |listed: &Value, id: &str| {
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == format!("board:{id}"))
            .unwrap()["item"]["version"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (read, sent) = cached.ok(&[
        "task",
        "show",
        "board:T-3",
        "--pin",
        &version(&listed, "T-3"),
        "--json",
    ]);
    assert_eq!(
        sent,
        [graphql::ISSUE_DETAIL],
        "nothing cached: the one read"
    );
    assert_eq!(served_by(&read), from_remote());

    // Cached only at an older version than the pin: the one read, and never the entry. A
    // listing reads the board and writes no entry, so it hands the caller a later version
    // than the one the cache holds.
    cached
        .board
        .edit_body("T-3", "a change the store has not read");
    let (relisted, _) = cached.ok(&["task", "list", "--source", "board", "--json"]);
    let later = version(&relisted, "T-3");
    let (behind, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &later, "--json"]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert_eq!(served_by(&behind), from_remote());
    assert_eq!(
        behind["items"][0]["item"]["content"],
        "a change the store has not read"
    );
    // And that read refreshed the entry, so the same pin is now answered from it.
    let (now, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &later, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(served_by(&now), from_cache());

    // A pin the source cannot put in order beside what is cached — a token naming another
    // repository than the task's — is incomparable: the one read.
    let incomparable = later.replace("nickderobertis/onetaskgraph", "acme/other");
    assert_ne!(incomparable, later);
    let (unordered, sent) = cached.ok(&[
        "task",
        "show",
        "board:T-3",
        "--pin",
        &incomparable,
        "--json",
    ]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert_eq!(served_by(&unordered), from_remote());
}

#[test]
fn with_caching_off_nothing_is_written_and_every_read_goes_to_the_board() {
    let cached = Cached::with(&json!({"enabled": false}));
    let (first, sent) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert!(
        first.get("cache").is_none(),
        "no cache member without a cache: {first}"
    );
    let version = version_of(&first);
    let (_, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &version, "--json"]);
    assert_eq!(
        sent,
        [graphql::ISSUE_DETAIL],
        "a pin on a source keeping no cache reads"
    );
    assert!(
        !cached.cache_home.join("onetaskgraph").exists(),
        "no cache directory is created"
    );
}

/// The version an expected write's `result` says the task is at now.
pub(crate) fn written_version(written: &Value) -> String {
    written["result"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("the write's response determined a version: {written}"))
        .to_owned()
}

/// Each part an expected write's `result` reports, as `(part, outcome)`.
fn parts(written: &Value) -> Vec<(String, String)> {
    written["result"]["parts"]
        .as_array()
        .unwrap_or_else(|| panic!("the write reports its parts: {written}"))
        .iter()
        .map(|part| {
            (
                part["part"].as_str().unwrap().to_owned(),
                part["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn landed(part: &str) -> (String, String) {
    (part.to_owned(), "landed".to_owned())
}

#[test]
fn an_expected_comment_write_sends_only_its_mutation_and_advances_the_entry_its_response_shows_whole()
 {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);

    let (added, sent) = cached.ok_with(
        &[
            "task",
            "comment",
            "add",
            "board:T-3",
            "--expect",
            &v1,
            "--json",
        ],
        Some("evidence, appended"),
    );
    assert_eq!(
        sent,
        [graphql::ADD_COMMENT_WHOLE],
        "one mutation and no read"
    );
    assert!(added.get("fallback").is_none(), "{added}");
    assert_eq!(parts(&added), [landed("comment")]);
    // The created comment's id and address, so a caller needs no lookup for either.
    let comment = added["id"].as_str().expect("the comment's id").to_owned();
    assert!(
        added["url"]
            .as_str()
            .is_some_and(|url| url.contains(&comment))
    );
    assert_eq!(
        added["result"]["parts"][0]["fields"]["comment"]["id"],
        comment
    );
    let v2 = written_version(&added);

    // The entry is the issue as that response showed it whole: a read pinned to its version
    // sends nothing, and shows the comment.
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v2, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(served_by(&shown), from_cache());
    assert_eq!(shown["comments"][0]["body"], "evidence, appended");

    let (edited, sent) = cached.ok_with(
        &[
            "task",
            "comment",
            "edit",
            "board:T-3",
            &comment,
            "--expect",
            &v2,
            "--json",
        ],
        Some("evidence, corrected"),
    );
    assert_eq!(
        sent,
        [graphql::UPDATE_COMMENT_WHOLE],
        "no COMMENT_ISSUE, no ISSUE"
    );
    let v3 = written_version(&edited);
    let (shown, sent) = cached.ok(&[
        "task",
        "comment",
        "list",
        "board:T-3",
        "--pin",
        &v3,
        "--json",
    ]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(shown["comments"][0]["body"], "evidence, corrected");

    // GitHub's comment delete answers with nothing about the issue, so it determines no
    // version: the entry is left unserved, and the next pinned read goes to the board.
    let (deleted, sent) = cached.ok(&[
        "task",
        "comment",
        "delete",
        "board:T-3",
        &comment,
        "--expect",
        &v3,
        "--json",
    ]);
    assert_eq!(sent, [graphql::DELETE_COMMENT]);
    assert_eq!(deleted["result"]["version"], Value::Null);
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v3, "--json"]);
    assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    assert_eq!(served_by(&shown), from_remote());
    assert_eq!(shown["comments"], json!([]));
}

#[test]
fn each_expected_task_write_sends_one_mutation_and_a_read_pinned_to_its_version_sends_nothing() {
    let cached = Cached::new();
    let body = cached.sandbox.subdirectory("bodies").join("content.md");
    std::fs::write(&body, "rewritten by an expected write\n").expect("a body file");
    let body = body.to_string_lossy().into_owned();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let mut version = version_of(&first);
    let writes: Vec<(Vec<&str>, &str)> = vec![
        (
            vec!["task", "priority", "set", "board:T-3", "high"],
            graphql::UPDATE_FIELD_WHOLE,
        ),
        (
            vec!["task", "status", "set", "board:T-3", "in-progress"],
            graphql::UPDATE_FIELD_WHOLE,
        ),
        (
            vec!["task", "content", "set", "board:T-3", "--file", &body],
            graphql::UPDATE_ISSUE_WHOLE,
        ),
        (
            vec![
                "task",
                "metadata",
                "set",
                "board:T-3",
                "caller.note",
                "\"kept\"",
            ],
            graphql::UPDATE_ISSUE_WHOLE,
        ),
        (
            vec!["task", "update", "board:T-3", "--title", "Gamma, updated"],
            graphql::UPDATE_ISSUE_WHOLE,
        ),
    ];
    for (arguments, mutation) in writes {
        let mut arguments = arguments.clone();
        arguments.extend(["--expect", &version, "--json"]);
        let (written, sent) = cached.ok(&arguments);
        assert_eq!(
            sent,
            [mutation],
            "`{}` sends its one mutation",
            arguments.join(" ")
        );
        assert!(written.get("fallback").is_none(), "{written}");
        let next = written_version(&written);
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &next, "--json"]);
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "after `{}`",
            arguments.join(" ")
        );
        assert_eq!(served_by(&shown), from_cache());
        version = next;
    }
    let (shown, _) = cached.ok(&["task", "show", "board:T-3", "--pin", &version, "--json"]);
    let task = &shown["items"][0]["item"];
    assert_eq!(task["priority"], "high");
    assert_eq!(task["status"]["category"], "in-progress");
    assert_eq!(task["content"], "rewritten by an expected write\n");
    assert_eq!(task["metadata"]["caller.note"], "kept");
    assert_eq!(task["title"], "Gamma, updated");
    // And the board holds exactly that.
    let (read, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(read["items"], shown["items"]);
}

#[test]
fn a_write_whose_response_shows_only_what_it_wrote_leaves_the_entry_unserved_at_its_version() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    // Between the caller's read and its write, a person edits a field the write does not
    // touch and comments on the task.
    cached
        .board
        .edit_title("T-3", "Gamma, retitled by a person");
    cached.board.add_comment("T-3", "a person's comment");
    cached.board.answer_writes_partially(true);

    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v1,
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE]);
    // The response determines the new version, and shows the priority it wrote — and not the
    // whole issue, so the cache does not take it for one.
    let v2 = written_version(&written);
    assert_eq!(written["result"]["parts"][0]["fields"]["Priority"], "High");
    cached.board.answer_writes_partially(false);

    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v2, "--json"]);
    assert_eq!(
        sent,
        [graphql::ISSUE_DETAIL],
        "never an entry labelled v2 that lacks them"
    );
    assert_eq!(served_by(&shown), from_remote());
    let task = &shown["items"][0]["item"];
    assert_eq!(task["title"], "Gamma, retitled by a person");
    assert_eq!(task["priority"], "high");
    assert_eq!(shown["comments"][0]["body"], "a person's comment");
}

#[test]
fn a_write_without_expect_reads_as_it_always_did_and_leaves_the_entry_unserved() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    let (written, sent) = cached.ok(&["task", "priority", "set", "board:T-3", "high", "--json"]);
    assert_eq!(
        sent.first().map(String::as_str),
        Some(graphql::ISSUE),
        "a write without --expect reads its target first, as the cost table pins: {sent:?}"
    );
    assert!(
        !sent.contains(&graphql::UPDATE_FIELD_WHOLE.to_owned()),
        "{sent:?}"
    );
    assert!(written.get("result").is_none(), "{written}");
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v1, "--json"]);
    assert_eq!(
        sent,
        [graphql::ISSUE_DETAIL],
        "the written task's entry is not served"
    );
    assert_eq!(shown["items"][0]["item"]["priority"], "high");
}

#[test]
fn an_expected_write_with_nothing_cached_reads_once_then_writes_and_says_so() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    let (cleared, sent) = cached.ok(&["cache", "clear", "board", "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(cleared["cleared"][0]["source"], "board");
    assert_eq!(cleared["cleared"][0]["entries"], 1);

    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v1,
        "--json",
    ]);
    assert_eq!(
        sent,
        [graphql::ISSUE, graphql::UPDATE_FIELD_WHOLE],
        "one read, then the write"
    );
    assert_eq!(written["fallback"]["read"], true);
    assert!(
        written["fallback"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no entry")),
        "{written}"
    );
    assert_eq!(
        written["spent"]["requests"], 2,
        "the read is counted in spent"
    );
    assert_eq!(parts(&written), [landed("fields")]);
}

#[test]
fn a_re_minted_option_costs_the_refused_mutation_one_read_and_one_retry_and_then_nothing_more() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    // T-3 sits at `Urgent`; its cached record carries the Priority field's options as they
    // were. A person renames `High` on the board, which re-mints its id.
    cached.board.remint_priority_option("High");
    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v1,
        "--json",
    ]);
    assert_eq!(
        sent,
        [
            graphql::UPDATE_FIELD_WHOLE,
            graphql::ISSUE,
            graphql::UPDATE_FIELD_WHOLE
        ],
        "the refused mutation, one read, one retry — and nothing else"
    );
    let reported = parts(&written);
    assert_eq!(reported.len(), 2, "{written}");
    assert_eq!(reported[0].1, "rejected");
    assert_eq!(written["result"]["parts"][0]["stale"], true);
    assert_eq!(reported[1], landed("fields"));
    assert_eq!(written["fallback"]["read"], true);
    assert_eq!(written["spent"]["requests"], 3);
    let v2 = written_version(&written);

    // The read refreshed the board's cached option ids: the next expected write needing the
    // same option, of another task, sends only its mutation.
    let (other, _) = cached.ok(&["task", "show", "board:T-4", "--json"]);
    let (again, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-4",
        "high",
        "--expect",
        &version_of(&other),
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE], "{again}");
    assert!(again.get("fallback").is_none(), "{again}");
    let _ = v2;
}

#[test]
fn a_lost_answer_is_reported_unknown_and_never_sent_again() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    cached
        .board
        .drop_response_once("updateProjectV2ItemFieldValue(input:$input)");
    let before = cached.board.served().len();
    let output = cached.run_with(
        &[
            "task",
            "priority",
            "set",
            "board:T-3",
            "high",
            "--expect",
            &v1,
            "--json",
        ],
        None,
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(
        sent,
        [graphql::UPDATE_FIELD_WHOLE],
        "sent once, and never again"
    );
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(
        failure["result"]["parts"][0]["outcome"], "unknown",
        "{failure}"
    );
    assert_eq!(failure["result"]["version"], Value::Null);
    // The write did land on the board: only its answer was lost.
    assert_eq!(cached.board.priority("T-3").as_deref(), Some("High"));
}

#[test]
fn an_expected_write_sends_only_the_field_it_sets_and_leaves_a_persons_edit_to_another_alone() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    // A person moves the card after the caller's read.
    cached.board.move_card("T-3", "Doing");
    let before = cached.board.served().len();
    let (_, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v1,
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE]);
    let (_, variables) = &cached.board.served()[before];
    assert_eq!(
        variables["input"]["fieldId"], "FIELD-priority",
        "the one mutation carries the priority alone: {variables}"
    );
    assert_eq!(
        cached.board.status("T-3").as_deref(),
        Some("Doing"),
        "the person's card stays"
    );
    assert_eq!(cached.board.priority("T-3").as_deref(), Some("High"));
}
