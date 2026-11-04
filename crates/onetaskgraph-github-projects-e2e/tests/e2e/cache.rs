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

use onetaskgraph_core::cache::ENTRY_SCHEMA;
use onetaskgraph_e2e_support::sdk::Sdk;
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

/// The one request a caching source's read of one task with its comments sends: the item, its
/// first page of comments and its `blockedBy` together, as a copy reads it, so a write expecting
/// it later reads none of them.
const ONE_READ: &str = graphql::ISSUE_COPY_DETAILS;

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
    assert_eq!(sent, [ONE_READ], "a cold read is the one detail read");
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
    assert_eq!(sent, [ONE_READ]);
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
    assert_eq!(sent, [ONE_READ]);
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
    assert_eq!(sent, [ONE_READ], "nothing cached: the one read");
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
    assert_eq!(sent, [ONE_READ]);
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
    assert_eq!(sent, [ONE_READ]);
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
    assert_eq!(sent, [ONE_READ]);
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
        [ONE_READ],
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
    // What the task holds after the write, and no parts: those are an expected write's.
    assert!(written["result"].get("parts").is_none(), "{written}");
    assert_eq!(written["result"]["priority"], "high", "{written}");
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v1, "--json"]);
    assert_eq!(sent, [ONE_READ], "the written task's entry is not served");
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

/// Run `arguments` with `variables` set, answering the JSON and the documents the board served.
fn ok_env(cached: &Cached, arguments: &[&str], variables: &[(&str, &str)]) -> (Value, Vec<String>) {
    let before = cached.board.served().len();
    let mut command = cached.sandbox.command();
    command.env("XDG_CACHE_HOME", &cached.cache_home);
    for (name, value) in variables {
        command.env(name, value);
    }
    let output = command.args(arguments).assert().get_output().clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let served = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    (
        serde_json::from_str(&stdout(&output)).expect("JSON"),
        served,
    )
}

#[test]
fn cache_enabled_is_read_from_every_layer_and_a_later_false_turns_it_off() {
    // Off in the document, on from the environment.
    let cached = Cached::with(&json!({"enabled": false}));
    let on = [("ONETASKGRAPH_SOURCES__BOARD__CACHE__ENABLED", "true")];
    let (first, _) = ok_env(&cached, &["task", "show", "board:T-3", "--json"], &on);
    assert_eq!(served_by(&first), from_remote());
    let v = version_of(&first);
    let (pinned, sent) = ok_env(
        &cached,
        &["task", "show", "board:T-3", "--pin", &v, "--json"],
        &on,
    );
    assert_eq!(sent, Vec::<String>::new(), "the environment turned it on");
    assert_eq!(served_by(&pinned), from_cache());

    // Off in the document, on from a flag.
    let flagged = Cached::with(&json!({"enabled": false}));
    let set = ["--set", "sources.board.cache.enabled=true"];
    let (first, _) = flagged.ok(&[set[0], set[1], "task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let (_, sent) = flagged.ok(&[
        set[0],
        set[1],
        "task",
        "show",
        "board:T-3",
        "--pin",
        &v,
        "--json",
    ]);
    assert_eq!(sent, Vec::<String>::new(), "--set turned it on");

    // On in the document: an environment's or a flag's `false` over it writes and serves
    // nothing.
    let document_on = Cached::new();
    let off = [("ONETASKGRAPH_SOURCES__BOARD__CACHE__ENABLED", "false")];
    for _ in 0..2 {
        let (shown, sent) = ok_env(&document_on, &["task", "show", "board:T-3", "--json"], &off);
        assert_eq!(sent, [graphql::ISSUE_DETAIL]);
        assert!(shown.get("cache").is_none());
        let (_, sent) = document_on.ok(&[
            "--set",
            "sources.board.cache.enabled=false",
            "task",
            "show",
            "board:T-3",
            "--pin",
            &version_of(&shown),
            "--json",
        ]);
        assert_eq!(sent, [graphql::ISSUE_DETAIL]);
    }
    assert!(
        !document_on.cache_home.join("onetaskgraph").exists(),
        "no entry was created"
    );
}

#[test]
fn cache_dir_from_every_layer_relocates_the_cache_and_a_pinned_read_is_served_from_there() {
    for layer in ["document", "environment", "flag"] {
        // A document's relative path is measured from the document's own directory.
        let cached = match layer {
            "document" => Cached::with(&json!({"enabled": true, "dir": "relocated"})),
            _ => Cached::new(),
        };
        let dir = cached.sandbox.project().join("relocated");
        let dir_text = dir.to_string_lossy().into_owned();
        let variables: Vec<(&str, &str)> = match layer {
            "environment" => vec![("ONETASKGRAPH_SOURCES__BOARD__CACHE__DIR", dir_text.as_str())],
            _ => Vec::new(),
        };
        let setting = format!("sources.board.cache.dir={dir_text}");
        let with = |arguments: &[&str]| -> Vec<String> {
            let mut all: Vec<String> = Vec::new();
            if layer == "flag" {
                all.extend(["--set".to_owned(), setting.clone()]);
            }
            all.extend(arguments.iter().map(|argument| (*argument).to_owned()));
            all
        };
        let first_args = with(&["task", "show", "board:T-3", "--json"]);
        let refs: Vec<&str> = first_args.iter().map(String::as_str).collect();
        let (first, _) = ok_env(&cached, &refs, &variables);
        let v = version_of(&first);
        assert_eq!(
            Cached::entries(&dir).len(),
            1,
            "{layer}: the entry is in the configured dir"
        );
        assert!(
            !cached.cache_home.join("onetaskgraph").exists(),
            "{layer}: nothing under the default location"
        );
        let pinned_args = with(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
        let refs: Vec<&str> = pinned_args.iter().map(String::as_str).collect();
        let (pinned, sent) = ok_env(&cached, &refs, &variables);
        assert_eq!(sent, Vec::<String>::new(), "{layer}");
        assert_eq!(served_by(&pinned), from_cache());
    }
}

#[test]
fn cache_clear_empties_exactly_what_it_names_and_the_next_pinned_read_refills_it() {
    let sandbox = Sandbox::new();
    let (one, first_board) = github_projects_with_board(&sandbox);
    let (two, second_board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "one": {"plugin":"github-projects","config":one,"cache":{"enabled":true}},
        "two": {"plugin":"github-projects","config":two,"cache":{"enabled":true}},
    })));
    let cache_home = sandbox.subdirectory("cache-home");
    let run = |arguments: &[&str]| -> Value {
        let output = sandbox
            .command()
            .env("XDG_CACHE_HOME", &cache_home)
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    let v_one = version_of(&run(&["task", "show", "one:T-3", "--json"]));
    let v_two = version_of(&run(&["task", "show", "two:T-3", "--json"]));
    let dirs: Vec<PathBuf> = std::fs::read_dir(cache_home.join("onetaskgraph"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(dirs.len(), 2, "two remotes, two caches");

    let cleared = run(&["cache", "clear", "one", "--json"]);
    assert_eq!(cleared["cleared"].as_array().unwrap().len(), 1);
    assert_eq!(cleared["cleared"][0]["source"], "one");
    let remaining: usize = dirs.iter().map(|dir| Cached::entries(dir).len()).sum();
    assert_eq!(remaining, 1, "only the named source's cache was emptied");

    let before = (first_board.served().len(), second_board.served().len());
    let shown = run(&["task", "show", "two:T-3", "--pin", &v_two, "--json"]);
    assert_eq!(served_by(&shown), from_cache());
    let shown = run(&["task", "show", "one:T-3", "--pin", &v_one, "--json"]);
    assert_eq!(
        served_by(&shown),
        from_remote(),
        "the cleared cache reads once"
    );
    let shown = run(&["task", "show", "one:T-3", "--pin", &v_one, "--json"]);
    assert_eq!(
        served_by(&shown),
        from_cache(),
        "and is refilled by that read"
    );
    assert_eq!(first_board.served().len() - before.0, 1);
    assert_eq!(second_board.served().len() - before.1, 0);

    let cleared = run(&["cache", "clear", "--json"]);
    assert_eq!(cleared["cleared"].as_array().unwrap().len(), 2);
    let remaining: usize = dirs.iter().map(|dir| Cached::entries(dir).len()).sum();
    assert_eq!(remaining, 0, "every source's cache was emptied");
}

#[test]
fn an_entry_of_an_unknown_schema_or_of_bytes_that_do_not_parse_is_a_miss_and_the_read_succeeds() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let entry = Cached::entries(&cached.default_dir()).remove(0);
    let held: Value = serde_json::from_slice(&std::fs::read(&entry).unwrap()).unwrap();

    let mut later = held.clone();
    later["schema"] = json!(99);
    for bytes in [
        serde_json::to_vec(&later).unwrap(),
        b"{\"schema\": 1, \"not\": \"an entry\"".to_vec(),
        Vec::new(),
    ] {
        std::fs::write(&entry, &bytes).unwrap();
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
        assert_eq!(sent, [ONE_READ], "a miss, read from the board");
        assert_eq!(served_by(&shown), from_remote());
        assert_eq!(shown["items"], first["items"]);
    }
}

#[test]
fn an_older_entry_a_racing_writer_left_is_never_served_to_a_later_pin() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let older_entry = std::fs::read(Cached::entries(&cached.default_dir()).remove(0)).unwrap();
    cached
        .board
        .edit_body("T-3", "changed after the first read");
    let (later, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let pin = version_of(&later);
    assert_ne!(pin, version_of(&first));
    // A second process, still holding what it read before the change, writes its entry after
    // the reader's token was taken.
    let entry = Cached::entries(&cached.default_dir()).remove(0);
    std::fs::write(&entry, &older_entry).unwrap();
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &pin, "--json"]);
    assert_eq!(sent, [ONE_READ], "the older entry is never served");
    assert_eq!(
        shown["items"][0]["item"]["content"],
        "changed after the first read"
    );
}

#[test]
fn two_processes_writing_one_cache_never_leave_a_reader_a_partial_entry() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let dir = cached.default_dir();
    let entry = Cached::entries(&dir).remove(0);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let stop = stop.clone();
        let entry = entry.clone();
        std::thread::spawn(move || {
            let mut reads = 0_u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(bytes) = std::fs::read(&entry) {
                    serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|error| {
                        panic!(
                            "a reader saw a partial entry ({error}): {} bytes",
                            bytes.len()
                        )
                    });
                    reads += 1;
                }
            }
            reads
        })
    };
    let writers: Vec<_> = (0..2)
        .map(|_| {
            let mut command = cached.sandbox.command();
            command.env("XDG_CACHE_HOME", &cached.cache_home).args([
                "task",
                "show",
                "board:T-3",
                "--json",
            ]);
            std::thread::spawn(move || {
                for _ in 0..4 {
                    command.assert().success();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("a writer");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.join().expect("the reader") > 0);
    let (pinned, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(served_by(&pinned), from_cache());
    assert!(
        std::fs::read_dir(dir.join("tasks"))
            .unwrap()
            .filter_map(Result::ok)
            .all(|file| !file.file_name().to_string_lossy().starts_with(".staged")),
        "no staging file is left behind"
    );
}

/// A board of eighty tasks, twenty of them commented on and holding one metadata value, cached
/// or not, with two empty folders of Markdown to copy into.
pub(crate) struct Followups {
    pub(crate) sandbox: Sandbox,
    pub(crate) board: GitHubBoardFields,
    pub(crate) cache_home: PathBuf,
}

/// The twenty tasks the narrowing names.
pub(crate) const FOLLOWUPS: usize = 20;
/// The narrowing a follow-up pass lists them by.
pub(crate) const NARROWED: [&str; 6] = [
    "--source",
    "board",
    "--commented-since",
    "2026-09-01T00:00:00Z",
    "--metadata",
    "orchestrator.follow-up/root_cause=stale-cache",
];

impl Followups {
    pub(crate) fn new(cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = crate::fixtures::github_projects_with_tasks(&sandbox, 76);
        let mut sources = serde_json::Map::new();
        sources.insert(
            "board".to_owned(),
            json!({"plugin":"github-projects","config":config,"cache":{"enabled":cache}}),
        );
        for folder in ["first", "second"] {
            let root = sandbox.subdirectory(folder);
            std::fs::create_dir_all(root.join("tasks")).expect("the folder");
            sources.insert(
                folder.to_owned(),
                json!({"plugin":"local-md","config":{"root":root}}),
            );
        }
        sandbox.project_document(
            &serde_json::to_string_pretty(&json!({"page_size": 5, "sources": sources}))
                .expect("a document"),
        );
        board.answer_search_pages_of(5);
        for n in 0..FOLLOWUPS {
            let task = crate::fixtures::github_extra_task(n);
            board.set_metadata(
                &task,
                "orchestrator.follow-up",
                json!({"root_cause": "stale-cache"}),
            );
            for comment in 0..=n % 5 {
                board.add_comment(&task, &format!("follow-up note {comment}"));
            }
        }
        board.set_metadata(
            "T-4",
            "orchestrator.follow-up",
            json!({"root_cause": "a different one"}),
        );
        let cache_home = sandbox.subdirectory("cache-home");
        Self {
            sandbox,
            board,
            cache_home,
        }
    }

    /// One command that had to succeed: its JSON, and every request the board served for it.
    pub(crate) fn ok(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`{}`: {}",
            arguments.join(" "),
            stderr(&output)
        );
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            self.board.served()[before..].to_vec(),
        )
    }

    /// A copy by the narrowing into `folder`, with comments and no link back to the board.
    pub(crate) fn copy(&self, folder: &str) -> (Value, Vec<(String, Value)>) {
        let mut arguments = vec![
            "task",
            "copy",
            "--to",
            folder,
            "--comments",
            "--no-link",
            "--json",
        ];
        arguments.extend(NARROWED);
        self.ok(&arguments)
    }
}

/// The ids a batched detail read named, slot by slot.
pub(crate) fn batched(served: &[(String, Value)], document: &str) -> Vec<String> {
    let mut ids: Vec<String> = served
        .iter()
        .filter(|(sent, _)| sent == document)
        .flat_map(|(_, variables)| {
            variables
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| name.starts_with("id"))
                .filter_map(|(_, id)| id.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[test]
fn a_copy_by_query_reads_its_tasks_edges_from_the_board_in_the_one_batch_that_carries_them() {
    let followups = Followups::new(true);
    let (first, served) = followups.copy("first");
    assert_eq!(first["items"].as_array().unwrap().len(), FOLLOWUPS);
    assert_eq!(
        batched(&served, graphql::ISSUE_COPY_DETAILS).len(),
        FOLLOWUPS,
        "a cold copy reads every task"
    );
    let changed: Vec<String> = (0..5)
        .map(|n| crate::fixtures::github_extra_task(n * 4))
        .collect();
    for id in &changed {
        followups
            .board
            .edit_title(id, "changed since the first copy");
    }
    // Into a folder holding none of them, so the destination skips nothing. A copy reads each
    // task's forward edges, which a board token does not vouch for — a far end can become a
    // project or a document, or be deleted, without moving it — so every task is read again,
    // in the one batched read that carries its edges: serving the fields from the cache and
    // reading the edges alone would cost a request a task.
    let (second, served) = followups.copy("second");
    assert_eq!(second["items"].as_array().unwrap().len(), FOLLOWUPS);
    assert_eq!(
        batched(&served, graphql::ISSUE_COPY_DETAILS).len(),
        FOLLOWUPS,
        "every task is read again with its edges"
    );
    assert!(
        served
            .iter()
            .all(|(document, _)| document == graphql::SEARCH_ISSUES
                || document == graphql::ISSUE_COPY_DETAILS),
        "the listing and the one batch of changed tasks: {:?}",
        served
            .iter()
            .map(|(document, _)| document)
            .collect::<Vec<_>>()
    );
    // What landed is what the board holds, the changed and the cached alike.
    let file = followups
        .sandbox
        .project()
        .join("second/tasks")
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .filter(|text| text.contains("changed since the first copy"))
        .count();
    assert_eq!(file, changed.len());
}

#[test]
fn a_show_many_fed_by_a_listing_pinned_to_its_versions_reads_only_what_changed() {
    let followups = Followups::new(true);
    let mut listing = vec!["task", "list", "--json", "--limit", "100"];
    listing.extend(NARROWED);
    let (listed, _) = followups.ok(&listing);
    let pins: Vec<String> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            format!(
                "{}={}",
                task["id"].as_str().unwrap(),
                task["item"]["version"].as_str().unwrap()
            )
        })
        .collect();
    let ids: Vec<String> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), FOLLOWUPS);
    let mut show_many = vec!["task".to_owned(), "show-many".to_owned()];
    show_many.extend(ids.iter().cloned());
    show_many.push("--json".to_owned());
    let cold: Vec<&str> = show_many.iter().map(String::as_str).collect();
    let (_, served) = followups.ok(&cold);
    assert_eq!(batched(&served, graphql::ISSUE_DETAILS).len(), FOLLOWUPS);

    let changed = crate::fixtures::github_extra_task(3);
    followups
        .board
        .edit_title(&changed, "changed since the listing's pass");
    let (relisted, _) = followups.ok(&listing);
    let mut pinned = show_many.clone();
    for task in relisted["items"].as_array().unwrap() {
        pinned.push("--pin".to_owned());
        pinned.push(format!(
            "{}={}",
            task["id"].as_str().unwrap(),
            task["item"]["version"].as_str().unwrap()
        ));
    }
    let pinned: Vec<&str> = pinned.iter().map(String::as_str).collect();
    let (details, served) = followups.ok(&pinned);
    // One task left to read is read alone, by the one-item detail read.
    assert_eq!(served.len(), 1, "{served:?}");
    assert_eq!(served[0].0, ONE_READ);
    assert_eq!(served[0].1["id0"], changed.as_str());
    let remote: Vec<&Value> = details["details"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|detail| detail["cache"]["served"] == "remote")
        .collect();
    assert_eq!(remote.len(), 1);
    assert_eq!(remote[0]["items"][0]["id"], format!("board:{changed}"));
    let _ = pins;
}

/// A task of the folder copied onto the cached board, the board's copy read into the cache:
/// the qualified id it landed under and the version it was read at.
fn bound(cached: &Cached, front_matter: &str) -> (String, String) {
    let plans = cached.sandbox.project().join("plans/tasks");
    std::fs::write(
        plans.join("B-1.md"),
        format!("---\n{front_matter}\n---\nthe bound task's body\n"),
    )
    .expect("a task file");
    let (copied, _) = cached.ok(&["task", "copy", "plans:B-1", "--to", "board", "--json"]);
    let landed = copied["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("the copy landed: {copied}"))
        .to_owned();
    let (shown, _) = cached.ok(&["task", "show", &landed, "--json"]);
    (landed, version_of(&shown))
}

/// Change the bound task's title and priority in its file, keeping every other line — its
/// link to the board's copy among them.
fn retitle(cached: &Cached, title: &str, priority: &str) {
    let path = cached.sandbox.project().join("plans/tasks/B-1.md");
    let text = std::fs::read_to_string(&path).expect("the task file");
    let rewritten: String = text
        .lines()
        .map(|line| {
            if line.starts_with("title:") {
                format!("title: {title}\n")
            } else if line.starts_with("priority:") {
                format!("priority: {priority}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(&path, rewritten).expect("the task file");
}

#[test]
fn a_bound_copy_with_expect_sends_only_the_parts_whose_fields_differ_and_no_read() {
    let cached = Cached::new();
    let (landed, version) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
    let native = landed.split_once(':').unwrap().1.to_owned();
    // A person changes the priority on the board after the caller's read.
    cached.board.assign_priority(&native, Some("Low"));
    retitle(&cached, "Bound, retitled", "high");
    let before = cached.board.served().len();
    let (copied, sent) = cached.ok(&[
        "task",
        "copy",
        "plans:B-1",
        "--to",
        "board",
        "--expect",
        &version,
        "--json",
    ]);
    assert_eq!(
        sent,
        [graphql::UPDATE_ISSUE_WHOLE],
        "one part, and no read: {copied}"
    );
    let (_, variables) = &cached.board.served()[before];
    assert_eq!(variables["input"]["title"], "Bound, retitled");
    // The body differs — its metadata slot records the version of the source this copy read —
    // and the state does not, so it is not sent.
    assert!(
        variables["input"].get("stateInput").is_none(),
        "only the fields that differ are sent: {variables}"
    );
    let item = &copied["items"][0];
    assert_eq!(item["result"]["parts"][0]["part"], "issue");
    assert_eq!(
        item["result"]["parts"][0]["fields"]["title"],
        "Bound, retitled"
    );
    assert!(item.get("fallback").is_none(), "{item}");
    assert_eq!(
        cached.board.priority(&native).as_deref(),
        Some("Low"),
        "the person's priority is left as they set it"
    );
}

#[test]
fn a_bound_copy_with_expect_reports_each_part_landed_rejected_or_unknown_and_retries_none() {
    for (refusal, outcome) in [("refuse", "rejected"), ("drop", "unknown")] {
        let cached = Cached::new();
        let (_, version) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
        retitle(&cached, "Bound, retitled", "low");
        match refusal {
            "refuse" => cached.board.refuse_once("updateIssue(input:$input)"),
            _ => cached.board.drop_response_once("updateIssue(input:$input)"),
        }
        let before = cached.board.served().len();
        let output = cached.run_with(
            &[
                "task",
                "copy",
                "plans:B-1",
                "--to",
                "board",
                "--expect",
                &version,
                "--json",
            ],
            None,
        );
        assert_ne!(
            output.status.code(),
            Some(0),
            "{refusal}: {}",
            stdout(&output)
        );
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        let reported = parts(&failure);
        assert_eq!(reported[0], landed("fields"), "{refusal}: {failure}");
        assert_eq!(
            reported[1],
            ("issue".to_owned(), outcome.to_owned()),
            "{failure}"
        );
        if outcome == "rejected" {
            assert!(failure["result"]["parts"][1]["error"].is_string());
        }
        let sent: Vec<String> = cached.board.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        assert_eq!(
            sent.iter()
                .filter(|document| *document == graphql::UPDATE_ISSUE_WHOLE)
                .count(),
            1,
            "{refusal}: the issue part is never sent again: {sent:?}"
        );
    }
}

/// Run one SDK script — Python's or TypeScript's — against the sandbox, with the cache home and
/// the task named, answering what it printed.
pub(crate) fn sdk(
    cached: &Cached,
    which: Sdk,
    script: &str,
    task: &str,
    pin: Option<&str>,
) -> Value {
    onetaskgraph_e2e_support::sdk::run(
        &cached.sandbox,
        &cached.cache_home,
        which,
        script,
        task,
        pin,
    )
}

const PYTHON_SHOW: &str =
    "result=await c.task_show(id=task,pin=pin)\n print(result.model_dump_json(exclude_none=True))";
const TYPESCRIPT_SHOW: &str =
    "console.log(JSON.stringify(await client.taskShow(id, pin === undefined ? {} : {pin})));";

/// One read through a Rust caller of the engine, built from the configuration the binary reads
/// — pinned to `pin` when given — and how it used the cache.
fn engine_read(
    cached: &Cached,
    task: &str,
    pin: Option<&str>,
) -> (Value, Option<onetaskgraph_core::CacheUse>) {
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            cached.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            cached.cache_home.to_string_lossy().into_owned(),
        ),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &cached.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let id: onetaskgraph_core::GlobalId = task.parse().expect("a qualified id");
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let detail = runtime
        .block_on(engine.task_detail_pinned(&id, pin))
        .expect("the read");
    let used = engine.cache_use(&id);
    (serde_json::to_value(&detail).expect("JSON"), used)
}

#[test]
fn one_sources_cache_is_shared_by_the_command_line_both_sdks_and_a_rust_caller() {
    let cached = Cached::new();
    // Written by the command line, served to every other caller with nothing sent.
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    for which in [Sdk::Python, Sdk::TypeScript] {
        let before = cached.board.served().len();
        let script = match which {
            Sdk::TypeScript => TYPESCRIPT_SHOW,
            Sdk::Python => PYTHON_SHOW,
        };
        let shown = sdk(&cached, which, script, "board:T-3", Some(&v));
        assert_eq!(cached.board.served().len(), before, "{which:?}");
        // The Python SDK leaves a null member out of what it prints, so the two are compared
        // by what a caller acts on: where the read was answered from, and what it answered.
        assert_eq!(shown["cache"]["served"], "cache", "{which:?}");
        assert_eq!(shown["items"][0]["id"], first["items"][0]["id"]);
        assert_eq!(version_of(&shown), v);
    }
    let before = cached.board.served().len();
    let (shown, used) = engine_read(&cached, "board:T-3", Some(&v));
    assert_eq!(
        cached.board.served().len(),
        before,
        "a Rust caller sends nothing"
    );
    assert_eq!(
        used.map(|used| used.served),
        Some(onetaskgraph_core::Served::Cache)
    );
    assert_eq!(shown["items"], first["items"]);

    // Written by each other caller, served to a pinned read of the command line.
    for (writer, task) in [
        ("python", "board:T-1"),
        ("typescript", "board:T-2"),
        ("rust", "board:T-4"),
    ] {
        let written = match writer {
            "python" => sdk(&cached, Sdk::Python, PYTHON_SHOW, task, None),
            "typescript" => sdk(&cached, Sdk::TypeScript, TYPESCRIPT_SHOW, task, None),
            _ => engine_read(&cached, task, None).0,
        };
        assert_eq!(written["cache"]["served"], "remote", "{writer}");
        let version = version_of(&written);
        let (pinned, sent) = cached.ok(&["task", "show", task, "--pin", &version, "--json"]);
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "an entry {writer} wrote serves the command line"
        );
        assert_eq!(served_by(&pinned), from_cache());
    }
}

#[test]
fn a_rust_callers_listing_pins_the_reads_that_follow_it_in_the_same_engine() {
    let cached = Cached::new();
    let ids = ["board:T-1", "board:T-3"];
    let mut arguments = vec!["task", "show-many"];
    arguments.extend(ids);
    arguments.push("--json");
    cached.ok(&arguments);
    cached
        .board
        .edit_title("T-3", "changed since it was cached");

    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            cached.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            cached.cache_home.to_string_lossy().into_owned(),
        ),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &cached.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let request = onetaskgraph_core::TaskRequest {
        sources: vec![onetaskgraph_plugin_api::SourceName::new("board".to_owned()).unwrap()],
        filters: onetaskgraph_core::Filters::default(),
        project: onetaskgraph_core::ProjectSelector::Any,
        priorities: Vec::new(),
        commented_since: None,
        metadata: Vec::new(),
        origin: None,
        include_members: false,
        paging: onetaskgraph_core::Paging {
            limit: std::num::NonZeroU32::new(50).unwrap(),
            token: None,
        },
    };
    runtime
        .block_on(engine.tasks(&request))
        .expect("the listing");
    let before = cached.board.served().len();
    let qualified: Vec<onetaskgraph_core::GlobalId> =
        ids.iter().map(|id| id.parse().unwrap()).collect();
    let details = runtime.block_on(engine.task_details(&qualified, true));
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(
        sent,
        [ONE_READ],
        "the listing pinned T-1 to what is cached; T-3 changed"
    );
    let used: Vec<_> = details
        .details
        .iter()
        .map(|detail| detail.cache.as_ref().map(|used| used.served))
        .collect();
    assert_eq!(
        used,
        [
            Some(onetaskgraph_core::Served::Cache),
            Some(onetaskgraph_core::Served::Remote)
        ]
    );
    assert_eq!(
        details.details[1].response.items[0].item.title,
        "changed since it was cached"
    );

    // Ending the command drops what the listing pinned: the next unit of work reads unpinned,
    // from the board.
    runtime
        .block_on(engine.end_command())
        .expect("the command ends");
    let before = cached.board.served().len();
    let again = runtime.block_on(engine.task_details(&qualified[..1], true));
    assert_eq!(
        cached.board.served().len() - before,
        1,
        "read from the board"
    );
    assert_eq!(
        again.details[0].cache.as_ref().map(|used| used.served),
        Some(onetaskgraph_core::Served::Remote)
    );
}

/// Every file under `root`, with its bytes.
fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    files
}

#[test]
fn with_caching_on_the_cache_directory_is_the_only_place_a_file_changes() {
    let cached = Cached::new();
    let root = cached
        .sandbox
        .project()
        .parent()
        .expect("the sandbox")
        .to_path_buf();
    let before = tree(&root);
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    cached.ok(&["task", "show-many", "board:T-1", "board:T-2", "--json"]);
    cached.ok(&[
        "task",
        "comment",
        "list",
        "board:T-3",
        "--pin",
        &v,
        "--json",
    ]);
    cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v,
        "--json",
    ]);
    // Projects and documents, read cold and pinned, and the listings that pin them.
    for (kind, id) in [("project", "board:P-1"), ("document", "board:D-1")] {
        let (read, _) = cached.ok(&[kind, "show", id, "--json"]);
        let pin = read["items"][0]["item"]["version"]
            .as_str()
            .expect("a version")
            .to_owned();
        cached.ok(&[kind, "show", id, "--pin", &pin, "--json"]);
        cached.ok(&[kind, "list", "--source", "board", "--json"]);
    }
    let after = tree(&root);
    let cache = cached.cache_home.join("onetaskgraph");
    let changed: Vec<&PathBuf> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .chain(before.keys().filter(|path| !after.contains_key(*path)))
        .collect();
    assert!(!changed.is_empty(), "the cache was written");
    let outside: Vec<&&PathBuf> = changed
        .iter()
        .filter(|path| !path.starts_with(&cache))
        .collect();
    assert!(
        outside.is_empty(),
        "files changed outside the cache directory: {outside:?}"
    );
}

#[test]
fn every_verb_the_cache_does_not_serve_answers_and_costs_exactly_what_it_does_without_it() {
    let verbs: [&[&str]; 9] = [
        &["project", "list", "--source", "board", "--json"],
        &["project", "show", "board:P-1", "--json"],
        &["project", "deps", "board:P-1", "--json"],
        &["document", "list", "--source", "board", "--json"],
        &["label", "list", "--source", "board", "--json"],
        &["task", "deps", "board:T-1", "--json"],
        &[
            "task",
            "deps",
            "board:T-2",
            "--direction",
            "depended-on-by",
            "--json",
        ],
        &["search", "alpha", "--source", "board", "--json"],
        &["task", "show", "board:T-3", "--no-comments", "--json"],
    ];
    let off = Cached::with(&json!({"enabled": false}));
    let on = Cached::new();
    for verb in verbs {
        let (without, sent_without) = off.ok(verb);
        let (with, sent_with) = on.ok(verb);
        assert_eq!(sent_with.len(), sent_without.len(), "`{}`", verb.join(" "));
        let mut with = with;
        if let Some(members) = with.as_object_mut() {
            members.remove("cache");
        }
        assert_eq!(with, without, "`{}`", verb.join(" "));
    }
}

#[test]
fn every_write_whose_response_shows_only_what_it_wrote_leaves_its_new_version_unserved() {
    let writes: [(&str, &[&str]); 7] = [
        ("comment add", &["task", "comment", "add", "board:T-3"]),
        ("comment edit", &["task", "comment", "edit", "board:T-3"]),
        (
            "priority",
            &["task", "priority", "set", "board:T-3", "high"],
        ),
        (
            "status",
            &["task", "status", "set", "board:T-3", "in-progress"],
        ),
        (
            "content",
            &["task", "content", "set", "board:T-3", "--file"],
        ),
        (
            "metadata",
            &[
                "task",
                "metadata",
                "set",
                "board:T-3",
                "caller.note",
                "\"kept\"",
            ],
        ),
        (
            "update",
            &["task", "update", "board:T-3", "--priority", "low"],
        ),
    ];
    for (name, arguments) in writes {
        let cached = Cached::new();
        let comment = cached
            .board
            .add_comment("T-3", "the comment an edit rewrites");
        let body = cached.sandbox.subdirectory("bodies").join("content.md");
        std::fs::write(&body, "rewritten\n").expect("a body file");
        let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        let v1 = version_of(&first);
        // Between the caller's read and its write, a person edits the labels — which no write
        // here sets — and comments.
        cached.board.set_labels("T-3", &["bug", "a-persons-label"]);
        cached.board.add_comment("T-3", "a person's comment");
        cached.board.answer_writes_partially(true);
        let mut full: Vec<String> = arguments.iter().map(|word| (*word).to_owned()).collect();
        match name {
            "comment edit" => full.push(comment.clone()),
            "content" => full.push(body.to_string_lossy().into_owned()),
            _ => {}
        }
        full.extend(["--expect".to_owned(), v1.clone(), "--json".to_owned()]);
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        let stdin = name
            .starts_with("comment")
            .then_some("written by the caller");
        let (written, _) = cached.ok_with(&refs, stdin);
        let v2 = written_version(&written);
        cached.board.answer_writes_partially(false);
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v2, "--json"]);
        assert_eq!(sent, [ONE_READ], "{name}: the entry is not served at {v2}");
        assert_eq!(served_by(&shown), from_remote(), "{name}");
        let labels: Vec<&str> = shown["items"][0]["item"]["labels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|label| label["name"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"a-persons-label"), "{name}: {labels:?}");
        assert!(
            shown["comments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|comment| comment["body"] == "a person's comment"),
            "{name}: the person's comment is there"
        );
    }
}

#[test]
fn a_bound_copy_whose_response_shows_the_task_whole_advances_the_entry_and_one_that_does_not_leaves_it()
 {
    for partially in [false, true] {
        let cached = Cached::new();
        let (landed, version) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
        let native = landed.split_once(':').unwrap().1.to_owned();
        retitle(&cached, "Bound, retitled", "high");
        cached.board.add_comment(&native, "a person's comment");
        cached.board.answer_writes_partially(partially);
        let (copied, _) = cached.ok(&[
            "task",
            "copy",
            "plans:B-1",
            "--to",
            "board",
            "--expect",
            &version,
            "--json",
        ]);
        cached.board.answer_writes_partially(false);
        let next = written_version(&copied["items"][0]);
        let (shown, sent) = cached.ok(&["task", "show", &landed, "--pin", &next, "--json"]);
        if partially {
            assert_eq!(sent, [ONE_READ], "partial: not served at its version");
        } else {
            assert_eq!(sent, Vec::<String>::new(), "whole: served at its version");
        }
        assert_eq!(shown["items"][0]["item"]["title"], "Bound, retitled");
        assert_eq!(shown["comments"][0]["body"], "a person's comment");
    }
}

#[test]
fn a_github_task_with_no_change_token_is_read_every_time_and_says_why() {
    let cached = Cached::new();
    // Read in the very second it changed: GitHub's token waits for a task to settle.
    cached.board.read_as_it_changes(true);
    cached.board.edit_title("T-3", "changing right now");
    for _ in 0..2 {
        let (shown, sent) = cached.ok(&[
            "task",
            "show",
            "board:T-3",
            "--pin",
            "gh2|anything",
            "--json",
        ]);
        assert_eq!(sent, [ONE_READ]);
        assert_eq!(shown["cache"]["served"], "remote");
        assert!(
            shown["cache"]["bypassed"]
                .as_str()
                .is_some_and(|reason| reason.contains("no change token")),
            "{shown}"
        );
        assert!(shown["items"][0]["item"].get("version").is_none());
    }
}

#[test]
fn a_project_copy_reads_its_tasks_edges_from_the_board_however_the_cache_holds_them() {
    let cached = Cached::new();
    let copy = |folder: &str| {
        let root = cached.sandbox.project().join(folder);
        std::fs::create_dir_all(root.join("tasks")).unwrap();
        let setting = format!("sources.plans.config.root={}", root.display());
        cached.ok(&[
            "--set",
            &setting,
            "--set",
            "sources.plans.config.status_mapping.Shipped=done",
            "project",
            "copy",
            "board:P-1",
            "--to",
            "plans",
            "--comments",
            "--no-link",
            "--json",
        ])
    };
    // Cold: the project's tasks are loaded with their comments and edges, and cached.
    let (first, sent) = copy("first-copy");
    let loaded = batched(
        &sent
            .iter()
            .map(|document| (document.clone(), Value::Null))
            .collect::<Vec<_>>(),
        "",
    );
    let _ = loaded;
    assert!(
        sent.contains(&graphql::ISSUE_COPY_DETAILS.to_owned()),
        "{sent:?}"
    );
    // Into another folder holding none of them: the listing pins each task to the version it
    // carries and the cache holds every one at it — but a board token does not vouch for a
    // task's forward edges, so both are read again in the batch that carries their edges.
    let before = cached.board.served().len();
    let (second, _) = copy("second-copy");
    let served = cached.board.served()[before..].to_vec();
    assert_eq!(
        batched(&served, graphql::ISSUE_COPY_DETAILS),
        ["T-1", "T-2"],
        "every task is read again with its edges"
    );
    assert_eq!(
        first["items"].as_array().unwrap().len(),
        second["items"].as_array().unwrap().len()
    );
    let files = |folder: &str| {
        let mut texts: Vec<String> =
            std::fs::read_dir(cached.sandbox.project().join(folder).join("tasks"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
                .collect();
        texts.sort();
        texts
    };
    assert_eq!(
        files("first-copy"),
        files("second-copy"),
        "the cached copy lands the same tasks"
    );
}

/// A command that had to fail: its exit code, and what it said on stderr.
fn refused(cached: &Cached, arguments: &[&str]) -> (Option<i32>, String) {
    let output = cached.run_with(arguments, None);
    assert_ne!(
        output.status.code(),
        Some(0),
        "`{}` succeeded",
        arguments.join(" ")
    );
    (output.status.code(), stderr(&output))
}

#[test]
fn cache_clear_refuses_a_source_that_keeps_none_and_one_nothing_configures_naming_the_next_step() {
    let cached = Cached::new();
    let (_, said) = refused(&cached, &["cache", "clear", "plans"]);
    assert!(
        said.contains("keeps no cache") && said.contains("cache.enabled"),
        "{said}"
    );
    let (_, said) = refused(&cached, &["cache", "clear", "nowhere"]);
    assert!(said.contains("no source named nowhere"), "{said}");
}

#[test]
fn cache_clear_removes_only_what_the_store_wrote_in_a_directory_holding_other_files() {
    let cached = Cached::with(&json!({"enabled": true, "dir": "shared"}));
    let dir = cached.sandbox.project().join("shared");
    std::fs::create_dir_all(dir.join("tasks")).unwrap();
    std::fs::write(dir.join("notes.txt"), "a person's notes").unwrap();
    std::fs::write(dir.join("tasks/mine.json"), "{\"a person's\": \"file\"}").unwrap();
    std::fs::write(dir.join("board.json"), "not the store's board").unwrap();
    cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(
        Cached::entries(&dir).len(),
        2,
        "the store's entry beside the person's file"
    );
    let (cleared, _) = cached.ok(&["cache", "clear", "board", "--json"]);
    assert_eq!(cleared["cleared"][0]["entries"], 1);
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).unwrap(),
        "a person's notes"
    );
    assert!(dir.join("tasks/mine.json").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("board.json")).unwrap(),
        "not the store's board"
    );
    assert_eq!(Cached::entries(&dir), [dir.join("tasks/mine.json")]);
}

#[test]
fn a_cache_that_cannot_be_written_costs_nothing_but_the_cache() {
    let cached = Cached::with(&json!({"enabled": true, "dir": "blocked"}));
    // A file where the cache's directory belongs: no entry can be written under it.
    std::fs::write(cached.sandbox.project().join("blocked"), "in the way").unwrap();
    for _ in 0..2 {
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        assert_eq!(sent, [ONE_READ]);
        assert_eq!(shown["items"][0]["item"]["title"], "Gamma");
        let v = version_of(&shown);
        let (pinned, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
        assert_eq!(
            sent,
            [ONE_READ],
            "nothing could be cached, so the pin reads"
        );
        assert_eq!(served_by(&pinned), from_remote());
    }
    // A write is not refused over it: with a file where the cache directory belongs, no entry
    // can be there.
    let (_, sent) = cached.ok(&["task", "priority", "set", "board:T-3", "high", "--json"]);
    assert!(!sent.is_empty(), "the write was sent");
    let (shown, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    assert_eq!(shown["items"][0]["item"]["priority"], "high");
}

/// `onetaskgraph <arguments>` run where `dir` is a read-only mount: every entry under it
/// can be read and none removed, overwritten, emptied or re-moded — `EROFS` refuses each, for
/// the file's owner too. `None`, after saying so, on a host that cannot make that mount
/// unprivileged.
fn under_read_only_mount(cached: &Cached, dir: &Path, arguments: &[&str]) -> Option<Output> {
    let mounted = |tail: &[&std::ffi::OsStr]| {
        let mut command = cached.sandbox.subprocess("unshare");
        command
            .env("XDG_CACHE_HOME", &cached.cache_home)
            .args(["--user", "--map-root-user", "--mount", "sh", "-c"])
            .arg(r#"mount --bind -o ro "$1" "$1" && shift && exec "$@""#)
            .arg("sh")
            .arg(dir)
            .args(tail);
        command.output().ok()
    };
    let probe = mounted(&["touch".as_ref(), dir.join("probe").as_os_str()]);
    if !probe.is_some_and(|output| {
        String::from_utf8_lossy(&output.stderr).contains("Read-only file system")
    }) {
        eprintln!(
            "skipped: this host cannot mount a directory read-only without privileges, so no \
             cache entry can be made to refuse every removal, overwrite and re-mode here"
        );
        return None;
    }
    let mut tail = vec![onetaskgraph_e2e_support::binary().as_os_str()];
    tail.extend(arguments.iter().map(std::ffi::OsStr::new));
    mounted(&tail)
}

#[test]
fn a_write_whose_entry_nothing_can_retire_is_refused_unsent_and_no_later_reader_is_served_it() {
    let cached = Cached::with(&json!({"enabled": true, "dir": "held"}));
    let dir = cached.sandbox.project().join("held");
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v1 = version_of(&first);
    assert_ne!(
        first["items"][0]["item"]["priority"], "high",
        "the write changes it"
    );
    let entry = Cached::entries(&dir);
    assert_eq!(entry.len(), 1, "the task is cached");
    let held = std::fs::read(&entry[0]).unwrap();

    for expect in [None, Some(v1.as_str())] {
        let mut arguments = vec!["task", "priority", "set", "board:T-3", "high", "--json"];
        if let Some(version) = expect {
            arguments.extend(["--expect", version]);
        }
        let before = cached.board.served().len();
        let Some(output) = under_read_only_mount(&cached, &dir, &arguments) else {
            return;
        };
        let sent = cached.board.served().len() - before;

        // A separate process, pinned to the version the entry holds, is never served a task
        // the board no longer holds at that version.
        let (pinned, _) = cached.ok(&["task", "show", "board:T-3", "--pin", &v1, "--json"]);
        let (board, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        assert_eq!(
            pinned["items"][0]["item"], board["items"][0]["item"],
            "a pinned reader was served what the board no longer holds (expect: {expect:?})"
        );

        assert_ne!(output.status.code(), Some(0), "the write was refused");
        assert_eq!(
            sent, 0,
            "a refused write sends nothing (expect: {expect:?})"
        );
        let said = String::from_utf8_lossy(&output.stderr);
        assert!(
            said.contains("was not sent") && said.contains(&dir.display().to_string()),
            "the refusal names the cache directory and that nothing was sent: {said}"
        );
        assert_eq!(
            std::fs::read(&entry[0]).unwrap(),
            held,
            "the entry is as it was"
        );
    }

    // With the directory writable again, the same write lands, and a pin to the version it
    // replaced goes to the board for what it wrote.
    let (_, sent) = cached.ok(&["task", "priority", "set", "board:T-3", "high", "--json"]);
    assert!(!sent.is_empty(), "the write was sent");
    let (pinned, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v1, "--json"]);
    assert_eq!(sent, [ONE_READ], "the written task's entry is not served");
    assert_eq!(served_by(&pinned), from_remote());
    assert_eq!(pinned["items"][0]["item"]["priority"], "high");
}

#[test]
fn a_stale_option_in_a_multi_part_write_retries_the_refused_part_once_and_never_resends_a_landed_one()
 {
    let cached = Cached::new();
    let (_, version) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
    retitle(&cached, "Bound, retitled", "high");
    let path = cached.sandbox.project().join("plans/tasks/B-1.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("status: Todo", "status: Doing")).unwrap();
    // A person renames `Doing` on the board after the cache kept its id.
    cached.board.remint_status_option("Doing");
    let (copied, sent) = cached.ok(&[
        "task",
        "copy",
        "plans:B-1",
        "--to",
        "board",
        "--expect",
        &version,
        "--json",
    ]);
    let item = &copied["items"][0];
    let reported = parts(item);
    assert_eq!(reported[0].1, "rejected", "{item}");
    assert_eq!(item["result"]["parts"][0]["stale"], true);
    assert_eq!(
        reported
            .iter()
            .filter(|(part, outcome)| part == "issue" && outcome == "landed")
            .count(),
        1,
        "the issue part lands once: {item}"
    );
    assert_eq!(item["fallback"]["read"], true);
    let reads = sent
        .iter()
        .filter(|document| *document == graphql::ISSUE)
        .count();
    assert_eq!(reads, 1, "one read: {sent:?}");
    let issue_parts = sent
        .iter()
        .filter(|document| *document == graphql::UPDATE_ISSUE_WHOLE)
        .count();
    assert_eq!(issue_parts, 1, "never sent twice: {sent:?}");
    let destination = copied["items"][0]["destination"].as_str().unwrap();
    let native = destination.split_once(':').unwrap().1;
    assert_eq!(cached.board.status(native).as_deref(), Some("Doing"));
}

#[test]
fn a_malformed_pin_and_an_expect_no_cache_can_take_are_refused_before_anything_is_read() {
    let cached = Cached::new();
    let before = cached.board.served().len();
    let (code, said) = refused(
        &cached,
        &["task", "show-many", "board:T-3", "--pin", "board:T-3"],
    );
    assert_eq!(code, Some(1));
    assert!(said.contains("<ID>=<VERSION>"), "{said}");
    let (_, said) = refused(
        &cached,
        &[
            "task",
            "copy",
            "board:T-1",
            "board:T-3",
            "--to",
            "plans",
            "--expect",
            "v",
        ],
    );
    assert!(said.contains("one task alone"), "{said}");
    let (_, said) = refused(
        &cached,
        &[
            "task",
            "copy",
            "board:T-3",
            "--to",
            "plans",
            "--expect",
            "v",
        ],
    );
    assert!(said.contains("source plans keeps no cache"), "{said}");
    let (_, said) = refused(
        &cached,
        &[
            "project",
            "metadata",
            "set",
            "board:P-1",
            "caller.note",
            "1",
            "--expect",
            "v",
        ],
    );
    assert!(
        said.contains("--expect names the version of a task"),
        "{said}"
    );
    let (_, said) = refused(
        &cached,
        &[
            "task",
            "priority",
            "set",
            "plans:X-1",
            "high",
            "--expect",
            "v",
        ],
    );
    assert!(said.contains("source plans keeps no cache"), "{said}");
    assert_eq!(cached.board.served().len(), before, "nothing was read");
}

#[test]
fn the_typescript_sdk_pins_expects_and_clears_through_the_binary() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let before = cached.board.served().len();
    let script = "const many = await client.taskShowMany([id], {pins: {[id]: pin}}); \
         const comments = await client.taskCommentList(id, {pin}); \
         const written = await client.taskPrioritySet(id, 'high', {expect: pin}); \
         const cleared = await client.cacheClear('board'); \
         let refused = false; try { await client.taskShowMany([id], {pins: [pin]}); } \
         catch (error) { refused = error instanceof TypeError; } \
         console.log(JSON.stringify({many, comments, written, cleared, refused}));";
    let answered = sdk(&cached, Sdk::TypeScript, script, "board:T-3", Some(&v));
    assert_eq!(answered["many"]["details"][0]["cache"]["served"], "cache");
    assert_eq!(answered["comments"]["cache"]["served"], "cache");
    assert_eq!(
        answered["written"]["result"]["parts"][0]["outcome"],
        "landed"
    );
    assert_eq!(answered["cleared"]["cleared"][0]["source"], "board");
    assert_eq!(answered["refused"], true);
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(
        sent,
        [graphql::UPDATE_FIELD_WHOLE],
        "the pinned reads sent nothing"
    );
    assert_eq!(Cached::entries(&cached.default_dir()).len(), 0, "cleared");
}

#[test]
fn the_human_rendering_says_when_a_write_read_first_and_what_clear_emptied() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let output = cached.run_with(&["cache", "clear"], None);
    assert_eq!(output.status.code(), Some(0));
    let said = stdout(&output);
    assert!(said.contains("cleared board — 1 entries — at "), "{said}");
    let output = cached.run_with(
        &["task", "comment", "add", "board:T-3", "--expect", &v],
        Some("a reply"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("read first: the cache holds no entry"),
        "{said}"
    );
    let uncached = Cached::with(&json!({"enabled": false}));
    let output = uncached.run_with(&["cache", "clear"], None);
    assert_eq!(stdout(&output).trim(), "no source keeps a cache");
}

/// Rewrite the one task entry under `dir` through `change`.
fn rewrite_entry(dir: &Path, change: impl FnOnce(&mut Value)) {
    let path = Cached::entries(dir).remove(0);
    let mut entry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut entry);
    std::fs::write(&path, serde_json::to_vec(&entry).unwrap()).unwrap();
}

#[test]
fn a_context_another_release_wrote_is_declined_and_the_write_reads_once_instead() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    rewrite_entry(&cached.default_dir(), |entry| {
        entry["context"]["version"] = json!(99)
    });
    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v,
        "--json",
    ]);
    assert_eq!(sent, [graphql::ISSUE, graphql::UPDATE_FIELD_WHOLE]);
    assert!(
        written["fallback"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("could not take")),
        "{written}"
    );
    // A board context of another version is not taken either: the write takes the task's own
    // record, which carries what it needs.
    let v2 = written_version(&written);
    std::fs::write(
        cached.default_dir().join("board.json"),
        serde_json::to_vec(&json!({"schema": ENTRY_SCHEMA, "remote": "elsewhere", "context": {}}))
            .unwrap(),
    )
    .unwrap();
    let (_, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "low",
        "--expect",
        &v2,
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE]);
}

#[test]
fn an_edit_expecting_a_task_read_without_its_comments_reads_which_issue_the_comment_is_on() {
    let cached = Cached::new();
    let comment = cached.board.add_comment("T-3", "a comment to edit");
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--no-comments", "--json"]);
    let v = version_of(&first);
    let (edited, sent) = cached.ok_with(
        &[
            "task",
            "comment",
            "edit",
            "board:T-3",
            &comment,
            "--expect",
            &v,
            "--json",
        ],
        Some("edited"),
    );
    assert_eq!(
        sent,
        [graphql::COMMENT_ISSUE, graphql::UPDATE_COMMENT_WHOLE],
        "{edited}"
    );
    assert_eq!(edited["fallback"]["read"], true);
    assert_eq!(edited["body"], "edited");
}

#[test]
fn cache_clear_leaves_another_remotes_entries_another_releases_and_their_staging_alone() {
    let cached = Cached::new();
    cached.ok(&["task", "show", "board:T-3", "--json"]);
    let dir = cached.default_dir();
    let tasks = dir.join("tasks");
    std::fs::write(
        tasks.join("other.json"),
        json!({"schema": ENTRY_SCHEMA, "remote": "another"}).to_string(),
    )
    .unwrap();
    let ours = Cached::entries(&dir)
        .into_iter()
        .find(|path| !path.ends_with("other.json"))
        .unwrap();
    let mut later: Value = serde_json::from_slice(&std::fs::read(&ours).unwrap()).unwrap();
    // Another release's entry is one of a schema after this build's, whatever this build's is.
    later["schema"] = json!(ENTRY_SCHEMA + 1);
    std::fs::write(
        tasks.join("later.json"),
        serde_json::to_vec(&later).unwrap(),
    )
    .unwrap();
    std::fs::write(tasks.join(".staged-another-1-1"), "another remote's write").unwrap();
    let (cleared, _) = cached.ok(&["cache", "clear", "--json"]);
    assert_eq!(cleared["cleared"][0]["entries"], 1);
    for kept in ["other.json", "later.json", ".staged-another-1-1"] {
        assert!(tasks.join(kept).exists(), "{kept} is left alone");
    }
    assert!(!ours.exists(), "the store's own entry is gone");
}

#[cfg(unix)]
#[test]
fn a_cache_that_cannot_be_emptied_is_refused_naming_where_and_what_to_check() {
    use std::os::unix::fs::PermissionsExt;
    let cached = Cached::new();
    cached.ok(&["task", "show", "board:T-3", "--json"]);
    let tasks = cached.default_dir().join("tasks");
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = cached.run_with(&["cache", "clear", "board"], None);
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_ne!(output.status.code(), Some(0));
    let said = stderr(&output);
    assert!(
        said.contains("could not be emptied") && said.contains("permissions"),
        "{said}"
    );
}

#[test]
fn an_entry_missing_what_a_read_needs_is_not_served_to_it() {
    // Cached without its comments: a pinned read wanting them reads.
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--no-comments", "--json"]);
    let v = version_of(&first);
    let (_, sent) = cached.ok(&[
        "task",
        "show",
        "board:T-3",
        "--no-comments",
        "--pin",
        &v,
        "--json",
    ]);
    assert_eq!(sent, Vec::<String>::new(), "what it holds is served");
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
    assert_eq!(sent, [ONE_READ], "the comments it lacks are read");
    assert_eq!(served_by(&shown), from_remote());

    // More comments than one page holds: the read could not carry them all, so the entry
    // holds none and a pinned read of them reads.
    let long = Cached::new();
    for n in 0..101 {
        long.board.add_comment("T-4", &format!("note {n}"));
    }
    let (first, _) = long.ok(&["task", "show", "board:T-4", "--json"]);
    assert_eq!(first["comments"].as_array().unwrap().len(), 101);
    let v = version_of(&first);
    let (shown, sent) = long.ok(&["task", "show", "board:T-4", "--pin", &v, "--json"]);
    assert!(!sent.is_empty(), "the comments are read again");
    assert_eq!(served_by(&shown), from_remote());
    assert_eq!(shown["comments"].as_array().unwrap().len(), 101);
}

#[test]
fn an_expected_write_the_cache_holds_only_older_or_unordered_reads_once_then_writes() {
    for (name, later) in [("older", true), ("incomparable", false)] {
        let cached = Cached::new();
        let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        let held = version_of(&first);
        let expect = if later {
            // A listing reads the board and writes no entry: it hands the caller a later
            // version than the one cached.
            cached
                .board
                .edit_title("T-3", "changed since it was cached");
            let (listed, _) = cached.ok(&["task", "list", "--source", "board", "--json"]);
            listed["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|task| task["id"] == "board:T-3")
                .unwrap()["item"]["version"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            held.replace("nickderobertis/onetaskgraph", "acme/other")
        };
        let (written, sent) = cached.ok(&[
            "task",
            "priority",
            "set",
            "board:T-3",
            "high",
            "--expect",
            &expect,
            "--json",
        ]);
        assert_eq!(
            sent,
            [graphql::ISSUE, graphql::UPDATE_FIELD_WHOLE],
            "{name}"
        );
        assert!(
            written["fallback"]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("older version")),
            "{name}: {written}"
        );
    }
}

#[test]
fn the_python_sdk_pins_expects_and_clears_through_the_binary() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let before = cached.board.served().len();
    let script = "many=await c.task_show_many(ids=[task],pin=[f'{task}={pin}'])\n \
                  written=await c.task_priority_set(id=task,priority='high',expect=pin)\n \
                  cleared=await c.cache_clear(source='board')\n \
                  every=await c.cache_clear()\n \
                  import json\n \
                  print(json.dumps({'many':many.model_dump(mode='json'),'written':written.model_dump(mode='json'),'cleared':cleared.model_dump(mode='json'),'every':every.model_dump(mode='json')}))";
    let answered = sdk(&cached, Sdk::Python, script, "board:T-3", Some(&v));
    assert_eq!(answered["many"]["details"][0]["cache"]["served"], "cache");
    assert_eq!(
        answered["written"]["result"]["parts"][0]["outcome"],
        "landed"
    );
    assert_eq!(answered["cleared"]["cleared"][0]["source"], "board");
    assert_eq!(answered["every"]["cleared"][0]["entries"], 0);
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE]);
}

#[test]
fn a_stale_id_refused_after_an_earlier_part_landed_retries_that_part_alone() {
    let cached = Cached::new();
    let (_, version) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
    retitle(&cached, "Bound, retitled", "low");
    // The fields land, then the issue part is refused for an id that no longer resolves.
    cached
        .board
        .refuse_as_stale_once("updateIssue(input:$input)");
    let (copied, sent) = cached.ok(&[
        "task",
        "copy",
        "plans:B-1",
        "--to",
        "board",
        "--expect",
        &version,
        "--json",
    ]);
    let item = &copied["items"][0];
    let reported = parts(item);
    assert_eq!(reported[0], landed("fields"), "{item}");
    assert_eq!(reported[1].1, "rejected", "{item}");
    assert_eq!(item["result"]["parts"][1]["stale"], true);
    assert_eq!(reported[2], landed("issue"), "{item}");
    let count = |document: &str| sent.iter().filter(|sent| *sent == document).count();
    assert_eq!(count(graphql::ISSUE), 1, "one read: {sent:?}");
    assert_eq!(
        count(graphql::UPDATE_FIELDS) + count(graphql::UPDATE_FIELD_WHOLE),
        1,
        "the landed fields are never sent again: {sent:?}"
    );
    assert_eq!(
        count(graphql::UPDATE_ISSUE_WHOLE),
        2,
        "the refused part, once more: {sent:?}"
    );
}

#[test]
fn a_bound_copy_carrying_comments_with_expect_reads_neither_the_task_nor_its_comments() {
    let cached = Cached::new();
    let (landed, _) = bound(&cached, "title: Bound\nstatus: Todo\npriority: high");
    // Read the way a copy carrying comments reads its destination, so the cache holds it whole.
    let (shown, _) = cached.ok(&["task", "show", &landed, "--json"]);
    let version = version_of(&shown);
    retitle(&cached, "Bound, retitled", "high");
    let (copied, sent) = cached.ok(&[
        "task",
        "copy",
        "plans:B-1",
        "--to",
        "board",
        "--comments",
        "--expect",
        &version,
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_ISSUE_WHOLE], "{copied}");
    assert!(copied["items"][0].get("fallback").is_none(), "{copied}");
}

#[test]
fn an_expected_write_answered_before_its_task_settles_gives_no_version_and_leaves_the_entry_unserved()
 {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    // As against GitHub itself: the response is stamped in the second of the change it made.
    cached.board.read_as_it_changes(true);
    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "high",
        "--expect",
        &v,
        "--json",
    ]);
    assert_eq!(sent, [graphql::UPDATE_FIELD_WHOLE]);
    assert_eq!(written["result"]["version"], Value::Null, "{written}");
    assert_eq!(written["result"]["parts"][0]["outcome"], "landed");
    cached.board.read_as_it_changes(false);
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
    assert_eq!(
        sent,
        [ONE_READ],
        "the written task's entry is not served, and is refreshed"
    );
    assert_eq!(shown["items"][0]["item"]["priority"], "high");
    let refreshed = version_of(&shown);
    let (_, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &refreshed, "--json"]);
    assert_eq!(sent, Vec::<String>::new());
}

#[test]
fn the_default_cache_lives_under_home_without_xdg_cache_home_and_nowhere_without_either() {
    let cached = Cached::new();
    let home = cached.sandbox.subdirectory("home-dir");
    let run = |home: Option<&Path>| -> Value {
        let mut command = cached.sandbox.command();
        command.env_remove("XDG_CACHE_HOME");
        if let Some(home) = home {
            command.env("HOME", home);
        }
        let output = command
            .args(["task", "show", "board:T-3", "--json"])
            .assert()
            .success()
            .get_output()
            .clone();
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    let shown = run(Some(&home));
    assert_eq!(shown["cache"]["served"], "remote");
    let root = home.join(".cache/onetaskgraph");
    let dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("the cache under HOME")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(dirs.len(), 1);
    assert_eq!(Cached::entries(&dirs[0]).len(), 1);
    // With neither variable there is nowhere to default to, and the source keeps no cache.
    let shown = run(None);
    assert!(shown.get("cache").is_none(), "{shown}");
}

/// The version a later read is pinned to after an expected write whose response left out
/// `missing`: the version the response determined — or, for a response missing what the token's
/// facets are read from, which determines none, the version the caller held before it.
fn pinned_after(written: &Value, held: &str, facet_source: bool, missing: &str) -> String {
    match written["result"]["version"].as_str() {
        Some(version) => {
            assert!(
                !facet_source,
                "a response without {missing} spells no gh3 token: {written}"
            );
            version.to_owned()
        }
        None => {
            assert!(
                facet_source,
                "a response without {missing} still carries every part of the token: {written}"
            );
            held.to_owned()
        }
    }
}

#[test]
fn a_response_carrying_its_token_but_leaving_out_one_member_is_not_taken_for_the_whole_task() {
    // The labels, the body and the parent are what the token's facets are read from, so a
    // response without one determines no version at all; the rest leave it determined.
    for (member, facet_source) in [
        ("labels", true),
        ("body", true),
        ("parent", true),
        ("comments", false),
        ("createdAt", false),
        ("number", false),
    ] {
        let cached = Cached::new();
        cached.board.set_created_at("T-3", CREATED_AT);
        let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        let v = version_of(&first);
        cached.board.omit_from_writes(&[member]);
        let (written, _) = cached.ok(&[
            "task",
            "priority",
            "set",
            "board:T-3",
            "high",
            "--expect",
            &v,
            "--json",
        ]);
        let pin = pinned_after(&written, &v, facet_source, member);
        cached.board.omit_from_writes(&[]);
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &pin, "--json"]);
        assert_eq!(
            sent,
            [ONE_READ],
            "without {member}, the entry is not served at {pin}"
        );
        assert_eq!(served_by(&shown), from_remote());
        assert_eq!(
            shown["items"][0]["item"]["labels"][0]["name"], "bug",
            "{member}"
        );
        assert_eq!(
            shown["items"][0]["item"]["created_at"], CREATED_AT,
            "{member}"
        );
        assert_eq!(shown["items"][0]["item"]["priority"], "high", "{member}");
    }
}

#[test]
fn a_response_carrying_its_token_but_only_a_first_page_of_one_connection_is_not_taken_whole() {
    // A first page of the labels or of the board item's values is a first page of what the
    // facets are read from: no version. One of the comments leaves the token whole.
    for (connection, facet_source) in [
        ("/labels", true),
        ("/comments", false),
        ("/projectItems/nodes/0/fieldValues", true),
    ] {
        let cached = Cached::new();
        let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
        let v = version_of(&first);
        cached.board.truncate_in_writes(&[connection]);
        let (written, _) = cached.ok(&[
            "task",
            "priority",
            "set",
            "board:T-3",
            "high",
            "--expect",
            &v,
            "--json",
        ]);
        let pin = pinned_after(&written, &v, facet_source, connection);
        cached.board.truncate_in_writes(&[]);
        let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &pin, "--json"]);
        assert_eq!(
            sent,
            [ONE_READ],
            "with {connection} going on, the entry is not served at {pin}"
        );
        assert_eq!(served_by(&shown), from_remote());
        assert_eq!(
            shown["items"][0]["item"]["priority"], "high",
            "{connection}"
        );
    }
}

/// When the loopback's issue `T-3` was created, in the spelling a task's JSON reports it.
const CREATED_AT: &str = "2026-01-02T03:04:05Z";

#[test]
fn a_stale_id_retry_that_is_refused_too_reports_both_and_is_never_sent_a_third_time() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    cached.board.remint_priority_option("High");
    // The stale mutation is answered with GitHub's refusal of the old option; the retry after
    // the read is refused as well.
    cached
        .board
        .refuse_after("updateProjectV2ItemFieldValue(input:$input)", 1);
    let before = cached.board.served().len();
    let output = cached.run_with(
        &[
            "task",
            "priority",
            "set",
            "board:T-3",
            "high",
            "--expect",
            &v,
            "--json",
        ],
        None,
    );
    assert_ne!(output.status.code(), Some(0));
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(
        sent,
        [
            graphql::UPDATE_FIELD_WHOLE,
            graphql::ISSUE,
            graphql::UPDATE_FIELD_WHOLE
        ],
        "no third attempt"
    );
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    let reported = parts(&failure);
    assert_eq!(reported.len(), 2, "{failure}");
    assert!(
        reported.iter().all(|(_, outcome)| outcome == "rejected"),
        "{failure}"
    );
    assert_eq!(failure["fallback"]["read"], true);
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &v, "--json"]);
    assert_eq!(sent, [ONE_READ], "the entry was left unserved");
    assert_eq!(
        shown["items"][0]["item"]["priority"], "urgent",
        "nothing landed"
    );
}

#[test]
fn the_human_rendering_of_an_expected_delete_says_it_read_first() {
    let cached = Cached::new();
    let comment = cached.board.add_comment("T-3", "to be removed");
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    cached.ok(&["cache", "clear", "--json"]);
    let output = cached.run_with(
        &[
            "task",
            "comment",
            "delete",
            "board:T-3",
            &comment,
            "--expect",
            &v,
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains(&format!("deleted comment {comment}")),
        "{said}"
    );
    assert!(
        said.contains("read first: the cache holds no entry"),
        "{said}"
    );
}

#[test]
fn every_typescript_sdk_write_forwards_expect_and_sends_only_its_mutation() {
    let cached = Cached::new();
    let body = cached.sandbox.subdirectory("bodies").join("content.md");
    std::fs::write(&body, "rewritten through the sdk\n").unwrap();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let before = cached.board.served().len();
    let script = format!(
        "const file = {}; const steps = []; let expect = pin; \
         const step = (name, written) => {{ steps.push({{name, result: written.result, \
           fallback: written.fallback ?? null}}); expect = written.result.version; return written; }}; \
         const added = step('add', await client.taskCommentAdd(id, {{body: 'a reply', expect}})); \
         step('edit', await client.taskCommentEdit(id, added.id, {{body: 'a corrected reply', expect}})); \
         step('status', await client.taskStatusSet(id, 'in-progress', {{expect}})); \
         step('content', await client.taskContentSet(id, file, {{expect}})); \
         step('metadata', await client.taskMetadataSet(id, 'caller.note', '\"kept\"', {{expect}})); \
         step('update', await client.taskUpdate(id, {{title: 'Gamma, by the sdk', expect}})); \
         const deleted = await client.taskCommentDelete(id, added.id, {{expect}}); \
         steps.push({{name: 'delete', result: deleted.result, fallback: deleted.fallback ?? null}}); \
         console.log(JSON.stringify(steps));",
        serde_json::to_string(&body).unwrap()
    );
    let steps = sdk(&cached, Sdk::TypeScript, &script, "board:T-3", Some(&v));
    for step in steps.as_array().unwrap() {
        assert_eq!(step["result"]["parts"][0]["outcome"], "landed", "{step}");
        assert_eq!(
            step["fallback"],
            Value::Null,
            "{step}: sent only its mutation"
        );
    }
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert_eq!(
        sent,
        [
            graphql::ADD_COMMENT_WHOLE,
            graphql::UPDATE_COMMENT_WHOLE,
            graphql::UPDATE_FIELD_WHOLE,
            graphql::UPDATE_ISSUE_WHOLE,
            graphql::UPDATE_ISSUE_WHOLE,
            graphql::UPDATE_ISSUE_WHOLE,
            graphql::DELETE_COMMENT,
        ],
        "one mutation per write, and no read"
    );
}

#[test]
fn an_expected_priority_clear_sends_only_the_clear_and_advances_the_entry() {
    let cached = Cached::new();
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let v = version_of(&first);
    let (written, sent) = cached.ok(&[
        "task",
        "priority",
        "set",
        "board:T-3",
        "none",
        "--expect",
        &v,
        "--json",
    ]);
    assert_eq!(sent, [graphql::CLEAR_FIELD_WHOLE]);
    assert_eq!(written["priority"], "none");
    let next = written_version(&written);
    let (shown, sent) = cached.ok(&["task", "show", "board:T-3", "--pin", &next, "--json"]);
    assert_eq!(
        sent,
        Vec::<String>::new(),
        "the cleared task is served at its version"
    );
    assert_eq!(shown["items"][0]["item"]["priority"], "none");
}

#[test]
fn cache_clear_removes_a_staging_file_this_remote_left_behind() {
    let cached = Cached::new();
    cached.ok(&["task", "show", "board:T-3", "--json"]);
    let dir = cached.default_dir();
    // The default directory is named for the remote's digest, which this store's staging
    // files carry: one a killed write left behind.
    let remote = dir.file_name().unwrap().to_string_lossy().into_owned();
    let abandoned = dir.join("tasks").join(format!(".staged-{remote}-1-1"));
    std::fs::write(&abandoned, "half a write").unwrap();
    cached.ok(&["cache", "clear", "board", "--json"]);
    assert!(
        !abandoned.exists(),
        "this remote's abandoned staging file is removed"
    );
}
