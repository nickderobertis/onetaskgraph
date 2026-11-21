//! A status write leaves the issue's open or closed state as the status it reports, on every
//! path that writes one, against the loopback board.
//!
//! An open issue's status is read off its card, so an open issue whose card already sits in a
//! terminal column already *reads* that category — and a write of it was once answered as no
//! write at all, leaving the issue open while the board called it finished (oneharness#1347).
//! Each journey here plants the four states that can disagree — an open issue under a
//! `Cancelled` card, an open issue under a `Done` card, a closed issue whose target is an open
//! column it already sits in, and an issue closed as completed under a `Cancelled` card — and
//! writes each the status its card already shows, through `task status set`, `task update`,
//! `task update-many`, a settlement through the linked engine, and `task status set` with the
//! source's cache on. Each issue then has to read back open or closed as its status requires,
//! with the closure reason the category implies, and none of them is sent a card write: every
//! card already showed its target option.

use std::process::Output;

use onetaskgraph_plugin_api::{MetadataKey, SourceName, Status, StatusCategory, TaskUpdate};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One task issue on the board, filed under no project, open at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
           "body":format!("Ticket {id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

/// One planted disagreement: the issue, the category written, and the state and closure
/// reason it has to read back with.
struct Case {
    id: &'static str,
    category: &'static str,
    state: &'static str,
    reason: Option<&'static str>,
}

/// The four, each written the status its card already shows.
fn cases() -> [Case; 4] {
    [
        // An open issue under a terminal card already reading that category (oneharness#1347).
        Case {
            id: "S-1",
            category: "cancelled",
            state: "CLOSED",
            reason: Some("NOT_PLANNED"),
        },
        Case {
            id: "S-2",
            category: "done",
            state: "CLOSED",
            reason: Some("COMPLETED"),
        },
        // A closed issue whose target is the open column its card already sits in.
        Case {
            id: "S-3",
            category: "todo",
            state: "OPEN",
            reason: None,
        },
        // Closed as completed under a `Cancelled` card, written `cancelled`: still closed, its
        // reason corrected.
        Case {
            id: "S-4",
            category: "cancelled",
            state: "CLOSED",
            reason: Some("NOT_PLANNED"),
        },
    ]
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    /// The board holding the four cases, with its cache on when `cached`.
    fn new(cached: bool) -> Self {
        let sandbox = Sandbox::new();
        let items = vec![
            issue("S-1", "Cancelled"),
            issue("S-2", "Done"),
            issue("S-3", "Todo"),
            issue("S-4", "Cancelled"),
        ];
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        // Every category written names its option, so no shipped default is in play.
        config["status_mapping"] = json!({"backlog":"Backlog","todo":"Todo",
            "in-progress":"In Progress","queued":"Queued","done":"Done","cancelled":"Cancelled"});
        board.close("S-3", Some("NOT_PLANNED"));
        board.close("S-4", Some("COMPLETED"));
        let mut source = json!({"plugin":"github-projects","config":config});
        if cached {
            source["cache"] = json!({"enabled": true,
                "dir": sandbox.subdirectory("cache").to_string_lossy()});
        }
        sandbox.project_document(&document(&json!({ "board": source })));
        Self { sandbox, board }
    }

    fn run(&self, arguments: &[&str], stdin: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        if let Some(stdin) = stdin {
            command.write_stdin(stdin.to_owned());
        }
        command.assert().get_output().clone()
    }

    /// One command that has to exit 0, answering its JSON.
    fn ok(&self, arguments: &[&str], stdin: Option<&str>) -> Value {
        let output = self.run(arguments, stdin);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// Every card write the board received, by the issue whose item it names.
    fn card_writes(&self) -> Vec<String> {
        let mut written = Vec::new();
        for (query, variables) in self.board.served() {
            if !query.trim_start().starts_with("mutation")
                || !query.contains("updateProjectV2ItemFieldValue")
            {
                continue;
            }
            let variables = variables.to_string();
            for case in cases() {
                if variables.contains(&format!("\"ITEM-{}\"", case.id)) {
                    written.push(case.id.to_owned());
                }
            }
        }
        written
    }

    /// Each issue reads back as its case says, and no card was written.
    fn assert_consistent(&self, path: &str) {
        for case in cases() {
            assert_eq!(
                self.board.state(case.id),
                (case.state.to_owned(), case.reason.map(str::to_owned)),
                "after {path}, {} written {} is not {} as its status requires",
                case.id,
                case.category,
                case.state
            );
        }
        assert_eq!(
            self.card_writes(),
            Vec::<String>::new(),
            "after {path}, a card already showing its target option was written"
        );
        // Every card still shows the option it showed.
        for (id, option) in [
            ("S-1", "Cancelled"),
            ("S-2", "Done"),
            ("S-3", "Todo"),
            ("S-4", "Cancelled"),
        ] {
            assert_eq!(
                self.board.status(id).as_deref(),
                Some(option),
                "{path}: {id}"
            );
        }
    }

    /// The configured engine the binary would build, linked as a long-lived caller.
    fn engine(&self) -> onetaskgraph_core::Engine {
        let environment = onetaskgraph_core::Environment::from_pairs([(
            "XDG_CONFIG_HOME",
            self.sandbox.config_home().to_string_lossy().into_owned(),
        )]);
        let loaded = onetaskgraph_core::config::load(
            &self.sandbox.project(),
            &environment,
            &onetaskgraph_core::config::Layer::default(),
        )
        .expect("the configuration loads");
        onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets)
    }
}

fn category(word: &str) -> StatusCategory {
    serde_json::from_value(json!(word)).expect("a category")
}

#[test]
fn a_status_set_brings_each_issue_s_state_in_line_with_its_card() {
    let setup = Setup::new(false);
    for case in cases() {
        let answer = setup.ok(
            &[
                "--json",
                "task",
                "status",
                "set",
                &format!("board:{}", case.id),
                case.category,
            ],
            None,
        );
        assert_eq!(answer["status"]["category"], case.category, "{answer:#}");
    }
    setup.assert_consistent("task status set");
    // A second write of each finds nothing out of line and sends no mutation at all.
    let before = setup.board.served().len();
    for case in cases() {
        setup.ok(
            &[
                "--json",
                "task",
                "status",
                "set",
                &format!("board:{}", case.id),
                case.category,
            ],
            None,
        );
    }
    let mutations = setup.board.served()[before..]
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .count();
    assert_eq!(mutations, 0, "a status already held is written again");
}

#[test]
fn a_task_update_naming_the_status_brings_each_issue_s_state_in_line() {
    let setup = Setup::new(false);
    for case in cases() {
        setup.ok(
            &[
                "--json",
                "task",
                "update",
                &format!("board:{}", case.id),
                "--status",
                case.category,
            ],
            None,
        );
    }
    setup.assert_consistent("task update");
}

#[test]
fn an_update_many_naming_the_status_brings_each_issue_s_state_in_line() {
    let setup = Setup::new(false);
    let updates: Vec<Value> = cases()
        .iter()
        .map(|case| json!({"item": format!("board:{}", case.id), "status": case.category}))
        .collect();
    let batch = json!({"schema_version": 1, "updates": updates});
    let answer = setup.ok(
        &["task", "update-many", "board", "--input", "-", "--json"],
        Some(&batch.to_string()),
    );
    for result in answer["results"].as_array().expect("results") {
        assert_eq!(result["outcome"], "written", "{answer:#}");
    }
    setup.assert_consistent("task update-many");
}

#[test]
fn a_settlement_through_the_engine_brings_each_issue_s_state_in_line() {
    let setup = Setup::new(false);
    let engine = setup.engine();
    let board = SourceName::new("board").expect("a name");
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    for case in cases() {
        let id = onetaskgraph_core::GlobalId {
            source: board.clone(),
            native: onetaskgraph_plugin_api::NativeId(case.id.to_owned()),
        };
        // A settlement-shaped update: a status, and the key recording which turn wrote it.
        let mut settlement = TaskUpdate {
            status: Some(Status {
                category: category(case.category),
                name: case.category.to_owned(),
            }),
            ..TaskUpdate::default()
        };
        settlement.metadata_set.insert(
            MetadataKey::new("onepipeline.settlement").expect("a key"),
            json!({"turn": 1}),
        );
        let updated = runtime
            .block_on(engine.update_task(&id, &settlement))
            .unwrap_or_else(|error| panic!("the settlement of {} lands: {error}", case.id));
        assert_eq!(
            updated.task.status.category,
            category(case.category),
            "{}",
            case.id
        );
        runtime
            .block_on(engine.end_command())
            .expect("the command ends");
    }
    setup.assert_consistent("a settlement through the engine");
}

#[test]
fn a_status_set_on_a_cached_source_brings_each_issue_s_state_in_line() {
    let setup = Setup::new(true);
    // Each task read once first, so its entry is filled and the write goes through the cache.
    for case in cases() {
        setup.ok(
            &["--json", "task", "show", &format!("board:{}", case.id)],
            None,
        );
    }
    for case in cases() {
        setup.ok(
            &[
                "--json",
                "task",
                "status",
                "set",
                &format!("board:{}", case.id),
                case.category,
            ],
            None,
        );
    }
    setup.assert_consistent("task status set with the cache on");
    // And a later read, from the cache or the board, reads each at its status.
    for case in cases() {
        let shown = setup.ok(
            &["--json", "task", "show", &format!("board:{}", case.id)],
            None,
        );
        assert_eq!(
            shown["items"][0]["item"]["status"]["category"], case.category,
            "{shown:#}"
        );
    }
}
