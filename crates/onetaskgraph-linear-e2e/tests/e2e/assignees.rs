//! A Linear source with `assignees: true`, through the binary against the loopback Linear: one
//! person, or none, read as the issue's `assignee{displayName}` and written as `assigneeId` in
//! the write the command already sends, each read back from a fresh process; a display name
//! Linear holds no user under refused for that item alone; a guard on the people an item holds
//! deciding an update and a copy; exactly the write requests the same command sends without an
//! assignee, and one person lookup per call naming people not yet resolved; a source without
//! the setting refusing every assignee write before anything reaches Linear and never asking
//! for one; and an assignee-only change moving the task's change token under the local cache.

use onetaskgraph_core::{ConfiguredSource, Engine, GlobalId, ResolvedSource};
use onetaskgraph_linear::graphql;
use onetaskgraph_plugin_api::{SourceName, SourcePlugin as _};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};
use crate::linear::{eng_mapping, held, issue, linear};

/// The workspace's three users, by id and display name.
const USERS: [(&str, &str); 3] = [("U-ADA", "ada"), ("U-BOB", "bob"), ("U-CY", "cy")];

/// Whether `query` is a lookup of people by display name: `USERS` itself, or that document
/// aliased once a page of 250 names.
fn is_lookup(query: &str) -> bool {
    query == graphql::USERS
        || (2..=40).any(|connections| graphql::aliased(graphql::USERS, connections) == query)
}

/// Five issues in the project `P-1` — `A-1` assigned to ada, the rest to nobody — and the
/// workspace's users.
fn workspace() -> Value {
    let mut workspace = held(
        (1..=5)
            .map(|at| issue(&format!("A-{at}"), "Todo", json!({"project": "P-1"})))
            .collect(),
        vec![
            json!({"id": "P-1", "title": "Engine", "content": "About the engine.",
                    "status": {"category": "in-progress", "name": "In Progress"},
                    "labels": []}),
        ],
        Vec::new(),
    );
    workspace["_linear_users"] = USERS
        .iter()
        .map(|(id, name)| json!({"id": id, "displayName": name}))
        .collect();
    workspace["_linear_assignees"] = json!({"A-1": "U-ADA"});
    workspace
}

/// One loopback Linear configured twice — `team` with `assignees: true` and `plain` without —
/// beside a folder of Markdown `notes` that holds assignees too.
struct Team {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    notes: std::path::PathBuf,
    cache_home: std::path::PathBuf,
    /// The loopback's own configuration — its endpoint and team.
    config: Value,
}

/// What one command printed and how it exited, and every request the loopback was sent while it
/// ran.
struct Ran {
    code: Option<i32>,
    json: Value,
    stderr: String,
    sent: Vec<(String, Value)>,
}

impl Ran {
    /// The write requests it sent — a batched one counted once.
    fn mutations(&self) -> Vec<&(String, Value)> {
        self.sent
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .collect()
    }

    /// How many person lookups it sent.
    fn lookups(&self) -> usize {
        self.sent
            .iter()
            .filter(|(query, _)| is_lookup(query))
            .count()
    }

    /// Every `assigneeId` its writes carried, in order.
    fn assignee_ids(&self) -> Vec<Value> {
        self.mutations()
            .into_iter()
            .flat_map(|(_, variables)| {
                variables
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(name, _)| name.starts_with("input"))
                    .filter_map(|(_, input)| input.get("assigneeId").cloned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Its requests other than person lookups, each with the assignee taken out of its document
    /// and its inputs — what the same command naming no assignee sends.
    fn without_assignees(&self) -> Vec<(String, Value)> {
        self.sent
            .iter()
            .filter(|(query, _)| !is_lookup(query))
            .map(|(query, variables)| {
                let mut variables = variables.clone();
                for (name, input) in variables.as_object_mut().into_iter().flatten() {
                    if name.starts_with("input")
                        && let Some(input) = input.as_object_mut()
                    {
                        input.remove("assigneeId");
                    }
                }
                (
                    graphql::without_assignees(query).unwrap_or_else(|| query.clone()),
                    variables,
                )
            })
            .collect()
    }
}

impl Team {
    fn new() -> Self {
        Self::with_cache(false)
    }

    fn with_cache(cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace(&sandbox, workspace());
        let loopback = config.clone();
        let mut team = linear(
            &config,
            json!({"status_mapping": eng_mapping(), "assignees": true}),
        );
        if cache {
            team["cache"] = json!({"enabled": true});
        }
        let plain = linear(&config, json!({"status_mapping": eng_mapping()}));
        let notes = sandbox.subdirectory("notes");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(notes.join(kind)).expect("a folder");
        }
        sandbox.project_document(&document(&json!({
            "team": team,
            "plain": plain,
            "notes": {"plugin": "local-md", "config": {"root": notes, "assignees": true}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        Self {
            sandbox,
            workspace,
            notes,
            cache_home,
            config: loopback,
        }
    }

    /// One command, in a process of its own.
    fn run(&self, arguments: &[&str]) -> Ran {
        let mark = self.workspace.served_count();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .args(arguments)
            .arg("--json")
            .write_stdin("")
            .assert()
            .get_output()
            .clone();
        Ran {
            code: output.status.code(),
            json: serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            stderr: stderr(&output),
            sent: self.workspace.served_since(mark),
        }
    }

    fn ok(&self, arguments: &[&str]) -> Ran {
        let ran = self.run(arguments);
        assert_eq!(
            ran.code,
            Some(0),
            "`onetaskgraph {}`: {}\n{}",
            arguments.join(" "),
            ran.json,
            ran.stderr
        );
        ran
    }

    /// The people `id` is assigned to, as `task show` in a fresh process reads them.
    fn assignees(&self, id: &str) -> Value {
        self.ok(&["task", "show", id, "--no-comments"]).json["items"][0]["item"]["assignees"]
            .clone()
    }

    /// A file holding `document`, for `--input`.
    fn file(&self, name: &str, document: &Value) -> String {
        let path = self.sandbox.subdirectory("inputs").join(name);
        std::fs::write(&path, document.to_string()).expect("an input");
        path.to_str().expect("a UTF-8 path").to_owned()
    }

    /// A body file for `task create`.
    fn body(&self) -> String {
        let path = self.sandbox.subdirectory("inputs").join("body.md");
        std::fs::write(&path, "A new task.\n").expect("a body");
        path.to_str().expect("a UTF-8 path").to_owned()
    }

    /// The note `N-1` in `notes`, assigned to `people`.
    fn note(&self, title: &str, people: &[&str]) {
        std::fs::write(
            self.notes.join("tasks/N-1.md"),
            format!(
                "---\ntitle: {title}\nstatus: todo\nassignees: [{}]\n---\nA note.\n",
                people.join(", ")
            ),
        )
        .expect("a note");
    }
}

/// The one result `update-many` answered for `item`.
fn result<'a>(ran: &'a Ran, item: &str) -> &'a Value {
    ran.json["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results: {}", ran.json))
        .iter()
        .find(|result| result["item"] == item || result["id"] == item)
        .unwrap_or_else(|| panic!("{item}: {}", ran.json))
}

#[test]
fn one_assignee_created_by_display_name_reads_back_from_a_fresh_process_at_the_cost_of_one_lookup()
{
    let team = Team::new();
    let body = team.body();
    let create = |assignee: Option<&str>| {
        let mut arguments = vec![
            "task",
            "create",
            "team",
            "--project",
            "P-1",
            "--title",
            "Assigned",
            "--body-file",
            &body,
        ];
        if let Some(person) = assignee {
            arguments.extend(["--assignee", person]);
        }
        team.ok(&arguments)
    };
    let plain = create(None);
    let assigned = create(Some("ada"));
    // The same write requests, each carrying the assignee in the input it already sends, and
    // one lookup resolving the person.
    assert_eq!(
        assigned.mutations().len(),
        plain.mutations().len(),
        "naming an assignee adds no write: {:#?}",
        assigned.sent
    );
    assert_eq!(assigned.lookups(), 1, "{:#?}", assigned.sent);
    assert_eq!(assigned.assignee_ids(), [json!("U-ADA")]);
    assert_eq!(plain.lookups(), 0);
    assert_eq!(
        plain.assignee_ids(),
        Vec::<Value>::new(),
        "a create naming nobody leaves Linear's default in place"
    );
    assert_eq!(assigned.sent.len(), plain.sent.len() + 1);

    let created = |ran: &Ran| {
        ran.json["items"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("an id: {}", ran.json))
            .to_owned()
    };
    let id = created(&assigned);
    assert_eq!(
        assigned.json["items"][0]["item"]["assignees"],
        json!(["ada"]),
        "the create answers what Linear assigned"
    );
    assert_eq!(team.assignees(&id), json!(["ada"]));
    let plain_id = created(&plain);
    assert_eq!(team.assignees(&plain_id), json!([]));
    // Every listing carries each issue's.
    let listing = team.ok(&["task", "list", "--source", "team", "--limit", "100"]);
    let listed = |id: &str| {
        listing.json["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["id"] == id)
            .unwrap_or_else(|| panic!("{id}: {}", listing.json))["item"]["assignees"]
            .clone()
    };
    assert_eq!(listed("team:A-1"), json!(["ada"]));
    assert_eq!(listed("team:A-2"), json!([]));
    assert_eq!(listed(&id), json!(["ada"]));
}

#[test]
fn an_assignee_is_replaced_by_another_person_and_then_cleared_each_in_one_write() {
    let team = Team::new();
    assert_eq!(team.assignees("team:A-1"), json!(["ada"]));

    let replaced = team.ok(&["task", "assignee", "set", "team:A-1", "bob"]);
    assert_eq!(
        replaced.json["assignees"],
        json!(["bob"]),
        "{}",
        replaced.json
    );
    assert_eq!(replaced.mutations().len(), 1, "{:#?}", replaced.sent);
    assert_eq!(replaced.assignee_ids(), [json!("U-BOB")]);
    assert_eq!(replaced.lookups(), 1, "bob, resolved once");
    assert_eq!(team.assignees("team:A-1"), json!(["bob"]));
    assert_eq!(team.workspace.assignee_of("A-1").as_deref(), Some("bob"));

    let cleared = team.ok(&["task", "assignee", "set", "team:A-1"]);
    assert_eq!(cleared.json["assignees"], json!([]), "{}", cleared.json);
    assert_eq!(cleared.mutations().len(), 1, "{:#?}", cleared.sent);
    assert_eq!(cleared.assignee_ids(), [Value::Null], "null unassigns");
    assert_eq!(cleared.lookups(), 0, "unassigning resolves nobody");
    assert_eq!(team.assignees("team:A-1"), json!([]));
    assert_eq!(team.workspace.assignee_of("A-1"), None);

    // Setting the person the issue already holds writes nothing.
    let unchanged = team.ok(&["task", "assignee", "set", "team:A-1"]);
    assert_eq!(unchanged.mutations().len(), 0, "{:#?}", unchanged.sent);
}

#[test]
fn an_update_naming_an_unknown_display_name_is_refused_for_that_item_alone() {
    let team = Team::new();
    let updates = team.file(
        "updates.json",
        &json!({"schema_version": 1, "updates": [
            {"item": "team:A-2", "assignee": "bob"},
            {"item": "team:A-3", "assignee": "nobody-here"},
        ]}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &updates]);
    assert_ne!(ran.code, Some(0), "{}", ran.json);
    assert_eq!(
        result(&ran, "team:A-2")["outcome"],
        "written",
        "{}",
        ran.json
    );
    let refused = result(&ran, "team:A-3");
    assert_eq!(refused["outcome"], "error", "{refused}");
    assert!(
        refused.to_string().contains("nobody-here"),
        "the refusal names the display name: {refused}"
    );
    assert_eq!(
        ran.lookups(),
        1,
        "both names in one lookup: {:#?}",
        ran.sent
    );
    assert_eq!(ran.assignee_ids(), [json!("U-BOB")], "{:#?}", ran.sent);
    assert_eq!(team.assignees("team:A-2"), json!(["bob"]));
    assert_eq!(team.assignees("team:A-3"), json!([]));
}

#[test]
fn an_update_guarded_on_its_assignees_writes_only_while_the_guard_holds() {
    let team = Team::new();
    let updates = team.file(
        "updates.json",
        &json!({"schema_version": 1, "updates": [
            // A-1 holds ada: met.
            {"item": "team:A-1", "assignee": "bob", "when_assignees": ["ada"]},
            // A-2 holds nobody: not met.
            {"item": "team:A-2", "assignee": "bob", "when_assignees": ["cy"]},
        ]}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &updates]);
    assert_eq!(
        result(&ran, "team:A-1")["outcome"],
        "written",
        "{}",
        ran.json
    );
    assert_eq!(result(&ran, "team:A-2")["outcome"], "held", "{}", ran.json);
    assert_eq!(
        ran.assignee_ids(),
        [json!("U-BOB")],
        "the held item sends no assignee: {:#?}",
        ran.sent
    );
    let written: Vec<&str> = ran
        .mutations()
        .into_iter()
        .flat_map(|(_, variables)| {
            variables
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| name.starts_with("id"))
                .filter_map(|(_, id)| id.as_str())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(written, ["A-1"], "{:#?}", ran.sent);
    assert_eq!(team.assignees("team:A-1"), json!(["bob"]));
    assert_eq!(team.assignees("team:A-2"), json!([]));
}

#[test]
fn five_updates_naming_one_person_beside_status_and_metadata_send_the_same_writes_and_one_lookup() {
    let updates = |assignee: bool| {
        json!({"schema_version": 1, "updates": (1..=5).map(|at| {
            let mut update = json!({"item": format!("team:A-{at}"), "status": "in-progress",
                                    "metadata": {"team.state": format!("moved-{at}")}});
            if assignee {
                update["assignee"] = json!("cy");
            }
            update
        }).collect::<Vec<_>>()})
    };
    let plain_team = Team::new();
    let plain_input = plain_team.file("updates.json", &updates(false));
    let plain = plain_team.ok(&["task", "update-many", "team", "--input", &plain_input]);
    let team = Team::new();
    let input = team.file("updates.json", &updates(true));
    let assigned = team.ok(&["task", "update-many", "team", "--input", &input]);

    assert_eq!(
        assigned.mutations().len(),
        plain.mutations().len(),
        "{:#?}",
        assigned.sent
    );
    assert_eq!(
        assigned.mutations().len(),
        1,
        "five updates, one aliased write"
    );
    assert_eq!(
        assigned.lookups(),
        1,
        "one person for five items, one lookup"
    );
    assert_eq!(assigned.assignee_ids(), vec![json!("U-CY"); 5]);
    assert_eq!(
        assigned.without_assignees(),
        plain.without_assignees(),
        "every other request is the one the same call without an assignee sends"
    );
    for at in 1..=5 {
        let task = team.ok(&["task", "show", &format!("team:A-{at}"), "--no-comments"]);
        let task = &task.json["items"][0]["item"];
        assert_eq!(task["assignees"], json!(["cy"]), "{task}");
        assert_eq!(task["status"]["category"], "in-progress", "{task}");
        assert_eq!(task["metadata"]["team.state"], format!("moved-{at}"));
    }
}

#[test]
fn a_person_resolved_once_is_not_looked_up_again_by_a_later_call_of_the_same_process() {
    let team = Team::new();
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            team.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            team.cache_home.to_string_lossy().into_owned(),
        ),
        ("LINEAR_API_KEY", "fixture-key".to_owned()),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &team.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let lookups = |mark: usize| {
        team.workspace
            .served_since(mark)
            .iter()
            .filter(|(query, _)| is_lookup(query))
            .count()
    };
    let assign = |id: &str, person: Option<&str>| {
        let mark = team.workspace.served_count();
        let id: GlobalId = id.parse().unwrap();
        let set = runtime
            .block_on(engine.set_task_assignee(&id, person))
            .expect("the assignee is set");
        assert_eq!(set.assignees, person.into_iter().collect::<Vec<_>>());
        lookups(mark)
    };
    assert_eq!(
        assign("team:A-2", Some("bob")),
        1,
        "the first call resolves bob"
    );
    assert_eq!(
        assign("team:A-3", Some("bob")),
        0,
        "a later call naming only bob resolves nothing"
    );
    assert_eq!(assign("team:A-4", None), 0, "unassigning resolves nobody");
    assert_eq!(
        assign("team:A-5", Some("cy")),
        1,
        "cy is new to this process"
    );
    assert_eq!(team.assignees("team:A-3"), json!(["bob"]));
    assert_eq!(team.assignees("team:A-5"), json!(["cy"]));
    // The same people, through a second instance over the same tracker in the same process.
    let mut config = team.config.clone();
    config["status_mapping"] = eng_mapping();
    config["assignees"] = json!(true);
    let name = SourceName::new("again").unwrap();
    let source = onetaskgraph_linear::Plugin
        .build(&name, &config, &loaded.secrets)
        .expect("the Linear source builds");
    let again = Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        ))],
        vec![name],
    );
    let mark = team.workspace.served_count();
    runtime
        .block_on(again.set_task_assignee(&"again:A-4".parse().unwrap(), Some("cy")))
        .expect("the assignee is set");
    assert_eq!(lookups(mark), 0, "the tracker's people are shared");
}

/// The copy journeys' destination: the note `N-1` copied into `team` once, its counterpart then
/// assigned to ada in Linear, and the note retitled and assigned to cy — so the next copy is an
/// update of an existing item.
fn copied(team: &Team) -> String {
    team.note("Copied", &["bob"]);
    let first = team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    let destination = first.json["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("a destination: {}", first.json))
        .to_owned();
    assert_eq!(
        team.assignees(&destination),
        json!(["bob"]),
        "a create takes the source's"
    );
    team.ok(&["task", "assignee", "set", &destination, "ada"]);
    team.note("Copied again", &["cy"]);
    destination
}

#[test]
fn a_guarded_copy_writes_the_source_assignee_only_while_the_guard_holds_and_sends_no_more_writes() {
    // The same copy over the same existing item, unguarded and guarded.
    let unguarded_team = Team::new();
    let unguarded_at = copied(&unguarded_team);
    let unguarded = unguarded_team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    assert_eq!(unguarded.assignee_ids(), Vec::<Value>::new());
    assert_eq!(unguarded.lookups(), 0);
    assert_eq!(
        unguarded_team.assignees(&unguarded_at),
        json!(["ada"]),
        "without the flag an existing item keeps its own"
    );

    let team = Team::new();
    let at = copied(&team);
    let guarded = team.ok(&[
        "task",
        "copy",
        "notes:N-1",
        "--to",
        "team",
        "--when-destination-assignees",
        "ada",
    ]);
    let outcome = &guarded.json["items"][0];
    assert_eq!(
        outcome["assignee_condition"],
        json!({"held": ["ada"], "met": true}),
        "{outcome}"
    );
    assert_eq!(
        guarded.assignee_ids(),
        [json!("U-CY")],
        "{:#?}",
        guarded.sent
    );
    assert_eq!(guarded.lookups(), 1);
    assert_eq!(
        guarded.mutations().len(),
        unguarded.mutations().len(),
        "the assignee rides in the write the copy sends anyway: {:#?}",
        guarded.sent
    );
    assert_eq!(guarded.without_assignees(), unguarded.without_assignees());
    assert_eq!(team.assignees(&at), json!(["cy"]));
    let show = team.ok(&["task", "show", &at, "--no-comments"]);
    assert_eq!(show.json["items"][0]["item"]["title"], "Copied again");

    // Not met: the copy writes the rest and leaves the assignee as it was.
    let other = Team::new();
    let other_at = copied(&other);
    let declined = other.ok(&[
        "task",
        "copy",
        "notes:N-1",
        "--to",
        "team",
        "--when-destination-assignees",
        "bob",
    ]);
    assert_eq!(
        declined.json["items"][0]["assignee_condition"],
        json!({"held": ["ada"], "met": false}),
        "{}",
        declined.json
    );
    assert_eq!(
        declined.assignee_ids(),
        Vec::<Value>::new(),
        "{:#?}",
        declined.sent
    );
    assert_eq!(declined.lookups(), 0);
    assert_eq!(other.assignees(&other_at), json!(["ada"]));
    let show = other.ok(&["task", "show", &other_at, "--no-comments"]);
    assert_eq!(show.json["items"][0]["item"]["title"], "Copied again");
}

#[test]
fn a_source_without_the_setting_refuses_every_assignee_write_before_linear_and_never_asks_for_one()
{
    let team = Team::new();
    let body = team.body();
    let updates = team.file(
        "updates.json",
        &json!({"schema_version": 1, "updates": [{"item": "plain:A-2", "assignee": "bob"}]}),
    );
    team.note("Copied", &["bob"]);
    for arguments in [
        vec![
            "task",
            "create",
            "plain",
            "--project",
            "P-1",
            "--title",
            "Refused",
            "--body-file",
            &body,
            "--assignee",
            "ada",
        ],
        vec!["task", "assignee", "set", "plain:A-2", "bob"],
        vec!["task", "assignee", "set", "plain:A-2"],
        vec!["task", "update-many", "plain", "--input", &updates],
        vec![
            "task",
            "copy",
            "notes:N-1",
            "--to",
            "plain",
            "--when-destination-assignees",
            "ada",
        ],
    ] {
        let ran = team.run(&arguments);
        assert_ne!(ran.code, Some(0), "`{}`: {}", arguments.join(" "), ran.json);
        assert!(
            ran.sent
                .iter()
                .all(|(query, _)| !query.starts_with("mutation")),
            "`{}` reached Linear with a write: {:#?}",
            arguments.join(" "),
            ran.sent
        );
        assert!(
            ran.sent.is_empty(),
            "`{}` was refused before any request: {:#?}",
            arguments.join(" "),
            ran.sent
        );
        assert!(
            format!("{}{}", ran.json, ran.stderr).contains("assignees"),
            "`{}` names the setting: {}{}",
            arguments.join(" "),
            ran.json,
            ran.stderr
        );
    }
    assert_eq!(team.workspace.assignee_of("A-2"), None);

    // Its reads carry no assignee and ask Linear for none.
    let mark = team.workspace.served_count();
    let listing = team.ok(&["task", "list", "--source", "plain", "--limit", "100"]);
    let show = team.ok(&["task", "show", "plain:A-1"]);
    for item in listing.json["items"].as_array().expect("items") {
        assert!(item["item"].get("assignees").is_none(), "{item}");
    }
    assert!(show.json["items"][0]["item"].get("assignees").is_none());
    let sent = team.workspace.served_since(mark);
    assert!(!sent.is_empty());
    for (query, variables) in sent {
        assert!(
            !query.contains("assignee") && !variables.to_string().contains("assignee"),
            "{query}"
        );
    }
}

#[test]
fn an_assignee_only_change_moves_the_change_token_and_a_pinned_read_is_never_served_stale() {
    let team = Team::with_cache(true);
    let token = |team: &Team| {
        team.ok(&["task", "list", "--source", "team", "--limit", "100"])
            .json["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["id"] == "team:A-1")
            .expect("A-1")["item"]["version"]
            .as_str()
            .expect("a token")
            .to_owned()
    };
    let pinned = |team: &Team, pin: &str| {
        team.ok(&["task", "show", "team:A-1", "--no-comments", "--pin", pin])
    };
    let before = token(&team);
    assert!(before.starts_with("lin1|"), "{before}");
    // Cached at the old token.
    let cold = pinned(&team, &before);
    assert_eq!(cold.json["items"][0]["item"]["assignees"], json!(["ada"]));
    let hit = pinned(&team, &before);
    assert!(hit.sent.is_empty(), "a hit sends nothing: {:#?}", hit.sent);

    let set = team.ok(&["task", "assignee", "set", "team:A-1", "bob"]);
    assert_eq!(set.mutations().len(), 1);
    let after = token(&team);
    assert_ne!(after, before, "an assignee-only change moves the token");
    if let Some(version) = set.json["result"]["version"].as_str() {
        assert_eq!(version, after, "the write answers the token it moved to");
    }

    let current = pinned(&team, &after);
    assert_eq!(
        current.json["items"][0]["item"]["assignees"],
        json!(["bob"])
    );
    assert_eq!(current.json["items"][0]["item"]["version"], after);
    // Pinned to the token the change moved past, the read is answered at the newer one or read
    // afresh — never with the person the old token held.
    let old = pinned(&team, &before);
    assert_eq!(
        old.json["items"][0]["item"]["assignees"],
        json!(["bob"]),
        "{}",
        old.json
    );
}

#[test]
fn a_create_naming_nobody_keeps_the_assignee_linear_gives_it_by_default() {
    let team = Team::new();
    // A Linear template or triage rule assigning every new issue to cy.
    team.workspace.assign_by_default("U-CY");
    let body = team.body();
    let created = team.ok(&[
        "task",
        "create",
        "team",
        "--project",
        "P-1",
        "--title",
        "Defaulted",
        "--body-file",
        &body,
    ]);
    assert_eq!(
        created.assignee_ids(),
        Vec::<Value>::new(),
        "a create naming nobody sends no assigneeId: {:#?}",
        created.sent
    );
    assert_eq!(created.lookups(), 0);
    assert_eq!(
        created.json["items"][0]["item"]["assignees"],
        json!(["cy"]),
        "the create reports the default Linear applied: {}",
        created.json
    );
    let id = created.json["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(team.assignees(&id), json!(["cy"]));

    // A task copied out of a folder that holds it unassigned — an explicit empty set — is a
    // create naming nobody too: it lands with the default, and its answer is not refused for
    // naming somebody.
    team.note("Unassigned at the source", &[]);
    let copied = team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    assert_eq!(
        copied.assignee_ids(),
        Vec::<Value>::new(),
        "{:#?}",
        copied.sent
    );
    let destination = copied.json["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("a destination: {}", copied.json))
        .to_owned();
    assert_eq!(team.assignees(&destination), json!(["cy"]));
}

/// An engine over `team`'s configuration, as a long-lived caller holds one across commands.
fn long_lived(team: &Team) -> (Engine, tokio::runtime::Runtime) {
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            team.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            team.cache_home.to_string_lossy().into_owned(),
        ),
        ("LINEAR_API_KEY", "fixture-key".to_owned()),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &team.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    (engine, runtime)
}

/// Assign `id` to `person` through `engine`: who it then holds, or why it was refused, and how
/// many person lookups the call sent.
fn attempt(
    team: &Team,
    engine: &Engine,
    runtime: &tokio::runtime::Runtime,
    id: &str,
    person: &str,
) -> (Result<Vec<String>, String>, usize) {
    let mark = team.workspace.served_count();
    let id: GlobalId = id.parse().unwrap();
    let answered = runtime
        .block_on(engine.set_task_assignee(&id, Some(person)))
        .map(|set| set.assignees)
        .map_err(|error| error.to_string());
    let lookups = team
        .workspace
        .served_since(mark)
        .iter()
        .filter(|(query, _)| is_lookup(query))
        .count();
    (answered, lookups)
}

#[test]
fn a_mismatched_or_failed_assignment_lets_go_of_the_people_resolved_when_its_call_ends() {
    let team = Team::new();
    let (engine, runtime) = long_lived(&team);
    let attempt = |id: &str, person: &str| attempt(&team, &engine, &runtime, id, person);
    let end_call = || {
        runtime
            .block_on(engine.end_command())
            .expect("the call ends");
    };
    assert_eq!(attempt("team:A-2", "bob"), (Ok(vec!["bob".to_owned()]), 1));
    assert_eq!(
        attempt("team:A-3", "bob"),
        (Ok(vec!["bob".to_owned()]), 0),
        "held: a later write naming bob resolves nothing"
    );

    // Linear answers the assignment with another person than it was sent.
    team.workspace.assign_instead("U-BOB", Some("U-CY"));
    let (answered, lookups) = attempt("team:A-4", "bob");
    let refusal = answered.expect_err("an answer naming another person is refused");
    assert!(
        refusal.contains("\"cy\"") && refusal.contains("\"bob\""),
        "the refusal names both: {refusal}"
    );
    assert_eq!(lookups, 0, "bob was still held when the write was sent");
    team.workspace.assign_instead("U-BOB", None);
    assert_eq!(
        attempt("team:A-5", "bob"),
        (Ok(vec!["bob".to_owned()]), 0),
        "the same call goes on with what its lookup found"
    );
    end_call();
    assert_eq!(
        attempt("team:A-4", "bob"),
        (Ok(vec!["bob".to_owned()]), 1),
        "the next call resolves bob afresh"
    );
    assert_eq!(
        attempt("team:A-2", "cy"),
        (Ok(vec!["cy".to_owned()]), 1),
        "cy is resolved, and held from here"
    );
    end_call();

    // Linear refuses a write naming an assignee.
    team.workspace.refuse_writes_of("A-1");
    let (answered, lookups) = attempt("team:A-1", "cy");
    answered.expect_err("the refused write fails");
    assert_eq!(lookups, 0, "cy was held when the write was sent");
    assert_eq!(team.workspace.assignee_of("A-1").as_deref(), Some("ada"));
    assert_eq!(
        attempt("team:A-3", "cy"),
        (Ok(vec!["cy".to_owned()]), 0),
        "the same call resolves nobody twice"
    );
    end_call();
    assert_eq!(
        attempt("team:A-4", "cy"),
        (Ok(vec!["cy".to_owned()]), 1),
        "after the call with the failure ends, cy is resolved afresh"
    );
    assert_eq!(team.assignees("team:A-3"), json!(["cy"]));
    assert_eq!(team.assignees("team:A-4"), json!(["cy"]));
    assert_eq!(team.assignees("team:A-5"), json!(["bob"]));
}

#[test]
fn a_display_name_unknown_in_one_command_is_assignable_after_the_command_ends() {
    let team = Team::new();
    let (engine, runtime) = long_lived(&team);
    let attempt = |person: &str| attempt(&team, &engine, &runtime, "team:A-2", person);
    let (answered, lookups) = attempt("dee");
    let refusal = answered.expect_err("nobody is displayed as dee");
    assert!(refusal.contains("\"dee\""), "{refusal}");
    assert_eq!(lookups, 1);
    let (answered, lookups) = attempt("dee");
    answered.expect_err("still nobody");
    assert_eq!(
        lookups, 0,
        "a name found unknown is refused again without a request"
    );

    // dee joins the workspace; within the same command the name is still the one refused.
    team.workspace.add_user("U-DEE", "dee");
    let (answered, lookups) = attempt("dee");
    answered.expect_err("refused until the command ends");
    assert_eq!(lookups, 0);

    runtime
        .block_on(engine.end_command())
        .expect("the command ends");
    assert_eq!(
        attempt("dee"),
        (Ok(vec!["dee".to_owned()]), 1),
        "the next command asks Linear again, and assigns dee"
    );
    assert_eq!(team.assignees("team:A-2"), json!(["dee"]));
}

#[test]
fn two_hundred_fifty_two_unseen_names_are_resolved_in_one_lookup_that_answers_each_of_them() {
    // Two hundred and fifty names Linear holds no user under, then two it knows: one lookup —
    // two `users` connections of one document — asks for all 252 and answers the last two.
    let team = Team::new();
    let mut updates: Vec<Value> = (0..250)
        .map(|n| json!({"item": format!("team:A-{}", n % 3 + 3), "assignee": format!("stranger{n}")}))
        .collect();
    updates.push(json!({"item": "team:A-1", "assignee": "bob"}));
    updates.push(json!({"item": "team:A-2", "assignee": "cy"}));
    let input = team.file(
        "many.json",
        &json!({"schema_version": 1, "updates": updates}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &input]);
    assert_ne!(ran.code, Some(0), "{}", ran.json);
    assert_eq!(ran.lookups(), 1, "{:#?}", ran.sent.len());
    let asked: usize = ran
        .sent
        .iter()
        .filter(|(query, _)| is_lookup(query))
        .flat_map(|(_, variables)| {
            variables
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| name.starts_with("names"))
                .map(|(_, names)| names.as_array().map_or(0, Vec::len))
        })
        .sum();
    assert_eq!(asked, 252, "the one lookup names every one of them");
    let results = ran.json["results"].as_array().expect("results");
    assert_eq!(results.len(), 252);
    assert!(
        results[..250]
            .iter()
            .all(|result| result["outcome"] == "error"),
        "{}",
        ran.json
    );
    assert!(
        results[0].to_string().contains("stranger0"),
        "{}",
        results[0]
    );
    assert_eq!(results[250]["outcome"], "written", "{}", results[250]);
    assert_eq!(results[251]["outcome"], "written", "{}", results[251]);
    assert_eq!(team.assignees("team:A-1"), json!(["bob"]));
    assert_eq!(team.assignees("team:A-2"), json!(["cy"]));
}

/// A copy whose create Linear answered assigned to somebody else is refused naming both, and the
/// issue it leaves is an existing item from then on: a later copy keeps whom it holds and sends
/// no assignee, and one guarded on exactly that person writes the note's over them.
#[test]
fn an_issue_a_refused_create_left_keeps_its_assignee_unless_a_copy_is_guarded_on_it() {
    let team = Team::new();
    team.note("Misassigned", &["bob"]);
    team.workspace.assign_instead("U-BOB", Some("U-CY"));
    let refused = team.run(&["task", "copy", "notes:N-1", "--to", "team"]);
    assert_ne!(refused.code, Some(0), "{}", refused.json);
    let said = refused.json.to_string();
    assert!(said.contains("cy") && said.contains("bob"), "{said}");
    team.workspace.assign_instead("U-BOB", None);
    let listed = team.ok(&["task", "list", "--source", "team", "--all"]);
    let made: Vec<String> = listed.json["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["item"]["title"] == "Misassigned")
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(made.len(), 1, "{}", listed.json);
    assert_eq!(team.assignees(&made[0]), json!(["cy"]));

    let kept = team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    assert_eq!(
        kept.json["items"][0]["destination"],
        made[0].as_str(),
        "{}",
        kept.json
    );
    assert!(kept.assignee_ids().is_empty(), "{:#?}", kept.sent);
    assert_eq!(team.assignees(&made[0]), json!(["cy"]));

    let guarded = team.ok(&[
        "task",
        "copy",
        "notes:N-1",
        "--to",
        "team",
        "--when-destination-assignees",
        "cy",
    ]);
    assert_eq!(
        guarded.json["items"][0]["assignee_condition"],
        json!({"held": ["cy"], "met": true}),
        "{}",
        guarded.json
    );
    assert_eq!(
        guarded.assignee_ids(),
        [json!("U-BOB")],
        "{:#?}",
        guarded.sent
    );
    assert_eq!(team.assignees(&made[0]), json!(["bob"]));
}

/// One way an answer is made malformed: what it is, and the change to the member it bends.
type Malformed = (&'static str, fn(&mut Value));

/// The failure kind a command that failed printed: its failure document's, or the first error
/// a read reported its source failed with.
fn failure_kind(ran: &Ran) -> String {
    ran.json["failure"]["kind"]
        .as_str()
        .or_else(|| ran.json["errors"][0]["error"]["kind"].as_str())
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_issue_answered_without_its_assignee_or_with_one_missing_a_name_is_refused_as_malformed() {
    let team = Team::new();
    let bends: [Malformed; 2] = [
        ("no assignee member", |issue| {
            if let Some(issue) = issue.as_object_mut() {
                issue.remove("assignee");
            }
        }),
        ("an assignee with no display name", |issue| {
            issue["assignee"] = json!({"id": "U-ADA"});
        }),
    ];
    for (what, bend) in bends {
        team.workspace.bend_answers_as_sent(
            |query, _| query.contains("issue(id:") && query.contains("assignee{displayName}"),
            1,
            move |_, data| {
                if let Some(issue) = data.get_mut("issue") {
                    bend(issue);
                }
            },
        );
        let ran = team.run(&["task", "show", "team:A-1", "--no-comments"]);
        assert_ne!(ran.code, Some(0), "{what}: {}", ran.json);
        assert_eq!(failure_kind(&ran), "malformed", "{what}: {}", ran.json);
    }
    // Answered as it should be, the same read holds its person.
    assert_eq!(team.assignees("team:A-1"), json!(["ada"]));
}

#[test]
fn a_lookup_answered_malformed_or_naming_one_display_name_twice_is_refused_writing_nothing() {
    let team = Team::new();
    let bends: [Malformed; 3] = [
        ("nodes that are not a list", |users| {
            users["nodes"] = json!("bob")
        }),
        ("a user with no id", |users| {
            users["nodes"] = json!([{"displayName": "bob"}]);
        }),
        ("one name answered as two users", |users| {
            users["nodes"] = json!([
                {"id": "U-BOB", "displayName": "bob"},
                {"id": "U-BOB-2", "displayName": "bob"},
            ]);
        }),
    ];
    for (what, bend) in bends {
        team.workspace.bend_answers_as_sent(
            |query, _| query == graphql::USERS,
            1,
            move |_, data| {
                if let Some(users) = data.get_mut("users") {
                    bend(users);
                }
            },
        );
        let ran = team.run(&["task", "assignee", "set", "team:A-2", "bob"]);
        assert_ne!(ran.code, Some(0), "{what}: {}", ran.json);
        assert_eq!(failure_kind(&ran), "malformed", "{what}: {}", ran.json);
        assert!(ran.mutations().is_empty(), "{what}: {:#?}", ran.sent);
    }
    assert_eq!(team.assignees("team:A-2"), json!([]));
}

#[test]
fn a_failed_assignment_followed_by_a_later_item_naming_the_same_person_costs_one_lookup() {
    // One update-many is one call: the write of A-1 naming bob is refused, and the later item
    // naming bob is written with the id the call's one lookup found.
    let team = Team::new();
    team.workspace.refuse_writes_of("A-1");
    let input = team.file(
        "failed.json",
        &json!({"schema_version": 1, "updates": [
            {"item": "team:A-1", "assignee": "bob"},
            {"item": "team:A-2", "assignee": "bob"},
        ]}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &input]);
    assert_ne!(ran.code, Some(0), "{}", ran.json);
    assert_eq!(result(&ran, "team:A-1")["outcome"], "error", "{}", ran.json);
    assert_eq!(
        result(&ran, "team:A-2")["outcome"],
        "written",
        "{}",
        ran.json
    );
    assert_eq!(ran.lookups(), 1, "{:#?}", ran.sent);
    assert_eq!(team.assignees("team:A-1"), json!(["ada"]));
    assert_eq!(team.assignees("team:A-2"), json!(["bob"]));
}
