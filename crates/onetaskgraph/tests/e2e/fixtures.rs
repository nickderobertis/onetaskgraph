//! The one table every journey is written against.
//!
//! A journey is written once and run against **every** source kind, so no plugin is ever
//! proven by a suite of its own writing. A row says which registry plugin it stands for,
//! how to configure one over the shared dataset below, and what that configuration
//! *declares* — which is what lets one journey assert both the rows and the plan against
//! a source that filters natively and one that does not.
//!
//! `scripts/check-journey-matrix.sh` fails, naming the plugin, when a plugin the registry
//! knows has no row here. Every registered plugin is implemented, so every row carries a
//! working source fixture.

use onetaskgraph_plugin_api::{Capabilities, DependencySupport, Support};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

use crate::common::{Sandbox, SourceBoundary};

/// One row: a source kind, in one configuration, over the shared dataset.
pub struct Row {
    /// The registry plugin kind this row stands for.
    pub plugin: &'static str,
    /// This row's own name, unique across rows, used in failure messages.
    pub name: &'static str,
    /// How to build it.
    pub fixture: Ready,
}

/// Everything a journey needs in order to drive one configured source.
pub struct Ready {
    /// The `config:` block, given a sandbox to write into if the source needs files.
    pub block: fn(&Sandbox) -> Value,
    /// What this configuration declares it applies itself.
    pub declared: Declared,
    /// Whether this source can represent the complete cross-plugin dataset.
    pub complete_dataset: bool,
}

/// What one row's source declares, so a journey can assert the plan as well as the rows.
///
/// One field per field of [`Capabilities`], in that type's own order and spelled with that
/// type's own values, so a capability no journey happens to drive is still represented
/// here — a predicate a plugin declares and then ignores narrows the answer silently, and
/// a field this table has no room for is a field nothing above the plugin can catch.
///
/// A struct of its own rather than a `Capabilities`, because a row's entry is a *claim
/// about* what the plugin reports rather than a copy of it. [`Declared::claimed`] is where
/// the two are made comparable, and `every_row_declares_exactly_what_its_plugin_reports`
/// in `journeys.rs` fails, naming the row and the field, when they disagree.
pub struct Declared {
    /// Whether the source filters tasks to a named project itself.
    pub projects: Support,
    /// Whether the source can select tasks belonging to no project.
    pub orphan_tasks: Support,
    /// Whether the source filters by label itself.
    pub filter_by_label: Support,
    /// Whether the source filters by status itself.
    pub filter_by_status: Support,
    /// Whether the source searches titles itself.
    pub search_title: Support,
    /// Whether the source searches bodies itself.
    pub search_content: Support,
    /// How far the source walks task dependencies itself.
    pub task_dependencies: DependencySupport,
    /// How far the source walks project dependencies itself.
    pub project_dependencies: DependencySupport,
    /// The largest page the source will serve.
    pub max_page_size: u32,
}

impl Declared {
    /// This declaration as the capability value a plugin would report.
    ///
    /// Spelled out field by field over a type with no `Default`, so a field added to the
    /// contract fails to compile here rather than going unreconciled.
    #[must_use]
    pub fn claimed(&self) -> Capabilities {
        Capabilities {
            projects: self.projects,
            orphan_tasks: self.orphan_tasks,
            filter_by_label: self.filter_by_label,
            filter_by_status: self.filter_by_status,
            search_title: self.search_title,
            search_content: self.search_content,
            task_dependencies: self.task_dependencies,
            project_dependencies: self.project_dependencies,
            max_page_size: self.max_page_size,
        }
    }

    /// Every field where this declaration and `reported` differ, named.
    ///
    /// One entry per disagreeing field rather than one whole-value comparison, because
    /// the failure a reader needs is *which capability* the table is lying about.
    #[must_use]
    pub fn disagreements(&self, reported: &Capabilities) -> Vec<String> {
        let claimed = self.claimed();
        let support = |field: &'static str, claimed: Support, reported: Support| {
            (claimed != reported).then(|| {
                format!("{field}: the table declares {claimed:?}, the plugin reports {reported:?}")
            })
        };
        let dependencies = |field: &'static str,
                            claimed: DependencySupport,
                            reported: DependencySupport| {
            (claimed != reported).then(|| {
                format!("{field}: the table declares {claimed:?}, the plugin reports {reported:?}")
            })
        };
        [
            support("projects", claimed.projects, reported.projects),
            support("orphan_tasks", claimed.orphan_tasks, reported.orphan_tasks),
            support(
                "filter_by_label",
                claimed.filter_by_label,
                reported.filter_by_label,
            ),
            support(
                "filter_by_status",
                claimed.filter_by_status,
                reported.filter_by_status,
            ),
            support("search_title", claimed.search_title, reported.search_title),
            support(
                "search_content",
                claimed.search_content,
                reported.search_content,
            ),
            dependencies(
                "task_dependencies",
                claimed.task_dependencies,
                reported.task_dependencies,
            ),
            dependencies(
                "project_dependencies",
                claimed.project_dependencies,
                reported.project_dependencies,
            ),
            (claimed.max_page_size != reported.max_page_size).then(|| {
                format!(
                    "max_page_size: the table declares {}, the plugin reports {}",
                    claimed.max_page_size, reported.max_page_size
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

impl Row {
    /// This row as a configuration document naming one source, `work`.
    ///
    /// Written as JSON, which the YAML reader accepts, so a fixture is a value rather
    /// than a string a test has to indent correctly.
    pub fn document(&self, sandbox: &Sandbox) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block}
        }))
    }

    /// This row as a document naming `work` and one writable Markdown folder beside it.
    ///
    /// The copy journeys need a destination, and a folder of Markdown is the one every
    /// row can be copied into: it is the source this plan makes writable, and it is what
    /// the user's own flow writes into and edits.
    pub fn document_with_folder(&self, sandbox: &Sandbox, folder: &str) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block},
            folder: {"plugin": "local-md", "config": empty_folder(sandbox, folder)},
        }))
    }

    /// What this row declares.
    pub fn declared(&self) -> &Declared {
        &self.fixture.declared
    }
}

/// An empty Markdown folder, ready to be copied into.
///
/// The status mapping covers every status name the shared dataset spells, because a
/// destination that would read a written status back as something else refuses the write
/// rather than narrowing it — which is right, and is not what these journeys are about.
pub fn empty_folder(sandbox: &Sandbox, relative: &str) -> Value {
    json!({
        "root": sandbox.subdirectory(relative),
        "status_mapping": {"todo": "todo", "doing": "in-progress", "shipped": "done"},
    })
}

pub fn document(sources: &Value) -> String {
    serde_json::to_string_pretty(&json!({ "sources": sources })).expect("a fixture renders")
}

/// The name every single-source journey configures its source under.
pub const SOURCE: &str = "work";

/// The name the two-source journeys give the row that applies everything itself.
pub const NATIVE: &str = "native";

/// The name they give the row that applies none of it and walks forwards only.
pub const SCANNED: &str = "scanned";

/// The capability pair built on either side of the process boundary.
pub fn pair_at(sandbox: &Sandbox, boundary: SourceBoundary) -> String {
    let mut sources = serde_json::Map::new();
    for (name, row) in [(NATIVE, &ROWS[0]), (SCANNED, &ROWS[1])] {
        sources.insert(
            name.to_owned(),
            boundary.source(row.plugin, (row.fixture.block)(sandbox)),
        );
    }
    document(&Value::Object(sources))
}

/// `<source>:<native>`, the form a user types.
///
/// Spelled here rather than inline so a journey asserting on an id is asserting on the
/// rendering under test rather than on its own `format!`.
pub fn qualified(source: &str, native: &str) -> String {
    format!("{source}:{native}")
}

/// Every row a journey runs against.
///
/// The two `in-memory` rows are the pair that proves pushdown and compensation return
/// one correct answer by two different plans: they hold the same dataset and the same
/// dependency graph, and differ only in what they declare. One answers reverse
/// dependencies itself and one does not, which is what makes the engine's emulated
/// reverse scan exercised deliberately here rather than incidentally by whichever plugin
/// happens to be poor at it.
pub const ROWS: &[Row] = &[
    Row {
        plugin: "in-memory",
        name: "in-memory (declares everything native)",
        fixture: Ready {
            block: native_block,
            complete_dataset: true,
            declared: EVERYTHING_NATIVE,
        },
    },
    Row {
        plugin: "in-memory",
        // `projects` is the one field this row cannot drop, and the name says so rather
        // than reading as an oversight somebody would helpfully "fix": in the contract
        // that field means *this source has projects at all*, so a source declaring it
        // unsupported contributes no project rows and the engine reports the predicate
        // unreachable instead of compensating. Every field where compensation is sound is
        // unsupported here on purpose — this row is the engine's compensation path's only
        // coverage, not a plugin someone forgot to finish.
        name: "in-memory (compensated: nothing native but its project table, forward-only)",
        fixture: Ready {
            block: compensated_block,
            complete_dataset: true,
            declared: Declared {
                projects: Support::Native,
                orphan_tasks: Support::Unsupported,
                filter_by_label: Support::Unsupported,
                filter_by_status: Support::Unsupported,
                search_title: Support::Unsupported,
                search_content: Support::Unsupported,
                task_dependencies: DependencySupport::ForwardOnly,
                project_dependencies: DependencySupport::ForwardOnly,
                max_page_size: 2,
            },
        },
    },
    Row {
        plugin: "subprocess",
        name: "subprocess (the in-memory source over a real pipe)",
        fixture: Ready {
            block: hosted_block,
            complete_dataset: true,
            declared: EVERYTHING_NATIVE,
        },
    },
    Row {
        plugin: "local-md",
        name: "local-md",
        fixture: Ready {
            block: local_md_block,
            complete_dataset: true,
            declared: Declared {
                max_page_size: 200,
                ..EVERYTHING_NATIVE
            },
        },
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Ready {
            block: linear_block,
            // Linear models the whole table: two projects, an orphan, and dependencies in
            // both directions, so it drives the shared complete-dataset journeys.
            complete_dataset: true,
            // The two searches are unsupported, which is what makes this row the one that
            // proves the engine's text compensation against a real remote protocol. That
            // is a ruling rather than a finding: Linear's own API has issue search, so
            // this is unimplemented rather than unsupportable — see the verdict table in
            // `onetaskgraph-linear`'s own module documentation and `docs/follow-ups.md`.
            declared: Declared {
                search_title: Support::Unsupported,
                search_content: Support::Unsupported,
                max_page_size: 250,
                ..EVERYTHING_NATIVE
            },
        },
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Ready {
            block: github_projects_block,
            // A board is a container of projects, not a project: a project is an issue and
            // its tasks are that issue's sub-issues, which is what this source's own module
            // documentation records and what the fixture board below is built as. So one
            // GitHub source represents the whole shared dataset — two projects with tasks
            // filed under each, and a task filed under neither — and drives every shared
            // journey rather than a subset chosen for it.
            complete_dataset: true,
            // The source walks the whole board before it answers anything, so it applies
            // every predicate a query carries itself; `GitHubProjectsSource`'s own module
            // documentation records why that is what `Native` means here.
            declared: Declared {
                max_page_size: 100,
                ..EVERYTHING_NATIVE
            },
        },
    },
];

/// The declaration a source that applies every predicate itself carries.
///
/// A named constant because five of the six rows differ from it in at most two fields,
/// and a row spelled out in full is a row whose one interesting difference is buried.
/// `max_page_size` is the in-memory default; a row whose plugin picks its own overrides it.
const EVERYTHING_NATIVE: Declared = Declared {
    projects: Support::Native,
    orphan_tasks: Support::Native,
    filter_by_label: Support::Native,
    filter_by_status: Support::Native,
    search_title: Support::Native,
    search_content: Support::Native,
    task_dependencies: DependencySupport::BothDirections,
    project_dependencies: DependencySupport::BothDirections,
    max_page_size: 50,
};
fn github_projects_block(sandbox: &Sandbox) -> Value {
    github_projects_server(sandbox, None)
}

/// The same board, with `T-1` recording `recorded` under the reserved dependency key.
///
/// The journeys that drive a key holding something it must not need a board that holds
/// it, and the shared row cannot be that board — it is the one every other journey reads.
pub fn github_projects_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    github_projects_server(sandbox, Some(recorded))
}

/// One item on the fixture board, in the shape the fixture keeps it between requests.
///
/// A board is a container of projects: `T-1`..`T-4` are task issues, and `P-1` and `P-2`
/// are project issues, readable as projects because they carry this source's own kind
/// marker rather than because the board is one.
///
/// [`Placement::parent`] is what makes that structure real rather than asserted, and the
/// board below sets it: the shared dataset's two projects hold their own tasks and one
/// task is filed under neither. Before it carried parents, every task on this board was an
/// orphan and no filter scoped to a project could have separated anything.
fn github_item(
    id: &str,
    title: &str,
    body: &str,
    at: Placement,
    labels: Value,
    slot: Value,
) -> Value {
    let body = if slot.as_object().is_some_and(serde_json::Map::is_empty) {
        body.to_owned()
    } else {
        format!("{body}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->")
    };
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,"body":body,
           "state":at.state.0,"reason":at.state.1,
           "parent":at.parent.map_or(Value::Null, |id| json!(id)),
           "repo":"nickderobertis/onetaskgraph","status":at.status,"origin":"",
           "labels":labels})
}

/// Where one fixture item sits on the board.
///
/// The three facts GitHub keeps about an issue's *position* rather than its content, in
/// one value: which `Status` option the board gives it, whether the issue is open or
/// closed and why, and which issue it is a sub-issue of.
struct Placement<'a> {
    /// The name of the board `Status` option on this item.
    status: &'a str,
    /// `IssueState`, and the `IssueStateReason` behind a closed one.
    state: (&'a str, Option<&'a str>),
    /// The issue this one is filed under, which is what project membership is here.
    parent: Option<&'a str>,
}

fn github_dataset(recorded: Option<&Value>) -> Vec<Value> {
    let marked = |extra: Value| {
        let mut slot = json!({"onetaskgraph.item_kind":"project"});
        for (key, value) in extra.as_object().expect("an object") {
            slot[key] = value.clone();
        }
        slot
    };
    vec![
        github_item(
            "T-1",
            "Alpha engine",
            "the engine core",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: Some("P-1"),
            },
            json!([["L-1", "bug"], ["L-3", "core"]]),
            json!({"onepipeline.turn_budget":12,"caller.flags":[true,null],
                   "onetaskgraph.depends_on":recorded.cloned().unwrap_or_else(||
                       Value::Array(recorded_far_ends("task_dependencies", &json!("T-1"))))}),
        ),
        github_item(
            "T-2",
            "Beta",
            "alpha in the body",
            Placement {
                status: "Shipped",
                state: ("CLOSED", Some("COMPLETED")),
                parent: Some("P-1"),
            },
            json!([["L-2", "chore"]]),
            json!({}),
        ),
        github_item(
            "T-3",
            "Gamma",
            "unrelated",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: None,
            },
            json!([["L-1", "bug"]]),
            json!({}),
        ),
        github_item(
            "T-4",
            "Delta docs",
            "documentation",
            Placement {
                status: "Doing",
                state: ("OPEN", None),
                parent: Some("P-2"),
            },
            json!([["L-3", "core"]]),
            json!({}),
        ),
        github_item(
            "P-1",
            "Engine",
            "the engine",
            Placement {
                status: "Doing",
                state: ("OPEN", None),
                parent: None,
            },
            json!([["L-3", "core"]]),
            marked(json!({"onepipeline.publication":{"mode":"review"},
                          "onetaskgraph.depends_on":recorded_far_ends("project_dependencies", &json!("P-1"))})),
        ),
        github_item(
            "P-2",
            "Docs",
            "alpha docs",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: None,
            },
            json!([]),
            marked(json!({})),
        ),
    ]
}

/// The board's own `blockedBy` graph: which item waits on which.
///
/// The dataset's fourth task edge is `related` rather than `blocks`, and GitHub has no
/// such relation — `blockedBy` is the only native one an issue has — so this board spells
/// it the only way it can. The shared journeys assert the edge's *ends*, which is the
/// property every backend owes; the kind a backend cannot express is not one of them.
fn github_blockers() -> Vec<(String, Vec<String>)> {
    [
        ("T-1", vec!["T-2"]),
        ("T-3", vec!["T-2"]),
        ("T-4", vec!["T-2"]),
        ("P-1", vec!["P-2"]),
    ]
    .into_iter()
    .map(|(id, blockers)| {
        (
            id.to_owned(),
            blockers.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect()
}

/// The board this fixture keeps, and everything a request may change on it.
struct GitHubBoard {
    items: Vec<Value>,
    /// Issues `createIssue` made which `addProjectV2ItemById` has not filed yet.
    pending: Vec<Value>,
    blocked_by: Vec<(String, Vec<String>)>,
    created: usize,
    /// The board's **own** title, description and readme — a person's, not this
    /// product's. `updateProjectV2` is answered here rather than refused so that a
    /// journey asserting these are byte-identical after a copy fails when something
    /// writes them, instead of passing because nothing could have.
    own: Value,
}

/// A read-only handle on one fixture board's own fields.
pub struct GitHubBoardFields {
    endpoint: String,
}

impl GitHubBoardFields {
    /// The board's own `title`, `shortDescription` and `readme`, asked of the GitHub
    /// endpoint the way any client of it asks.
    ///
    /// It crosses the same HTTP boundary the product crosses rather than reading the
    /// fixture's own memory, so a journey asserting the board is untouched is asserting
    /// what a person opening that board would see.
    #[must_use]
    pub fn own(&self) -> Value {
        graphql_over_http(
            &self.endpoint,
            "query($id:ID!){node(id:$id){... on ProjectV2{title shortDescription readme}}}",
            &json!({"id":"PVT-board"}),
        )["node"]
            .clone()
    }
}

/// One GraphQL request to a fixture endpoint, sent as the product itself sends one.
fn graphql_over_http(endpoint: &str, query: &str, variables: &Value) -> Value {
    let address = endpoint
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .expect("a fixture endpoint spelled http://host:port/graphql");
    let body = json!({"query":query,"variables":variables}).to_string();
    let mut stream = TcpStream::connect(address).expect("fixture connection");
    stream
        .write_all(
            format!(
                "POST /graphql HTTP/1.1\r\nHost: {address}\r\nauthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .expect("fixture request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("fixture response");
    let at = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP header terminator")
        + 4;
    let answered: Value = serde_json::from_slice(&response[at..]).expect("fixture response JSON");
    answered["data"].clone()
}

impl GitHubBoard {
    fn options() -> Value {
        json!([{"id":"OPT-backlog","name":"Backlog"},{"id":"OPT-todo","name":"Todo"},
               {"id":"OPT-doing","name":"Doing"},{"id":"OPT-shipped","name":"Shipped"}])
    }

    fn fields() -> Value {
        json!({"nodes":[
            {"__typename":"ProjectV2SingleSelectField","id":"FIELD-status","name":"Status",
             "options":Self::options()},
            {"__typename":"ProjectV2Field","id":"FIELD-origin","name":"onetaskgraph.origin"}
        ],"pageInfo":{"hasNextPage":false}})
    }

    fn subs(&self, id: &str) -> usize {
        self.items
            .iter()
            .filter(|item| item["parent"] == json!(id))
            .count()
    }

    fn find(&mut self, id: &Value) -> &mut Value {
        self.items
            .iter_mut()
            .find(|item| item["id"] == *id)
            .expect("the fixture holds the item being written")
    }

    fn content(&self, item: &Value) -> Value {
        json!({"__typename":"Issue","id":item["id"],"title":item["title"],"body":item["body"],
               "url":format!("https://example.invalid/{}", item["id"].as_str().unwrap()),
               "createdAt":null,"updatedAt":null,"state":item["state"],
               "stateReason":item["reason"],
               "repository":item["repo"].as_str().map(|repo| json!({"nameWithOwner":repo})),
               "parent":item["parent"].as_str().map(|id| json!({"id":id})),
               "subIssuesSummary":{"total":self.subs(item["id"].as_str().unwrap())},
               "labels":{"nodes":item["labels"].as_array().unwrap().iter()
                   .map(|pair| json!({"id":pair[0],"name":pair[1],"color":null}))
                   .collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false}}})
    }

    fn rendered(&self, item: &Value) -> Value {
        let mut values = vec![
            json!({"name":item["status"],"field":{"id":"FIELD-status","name":"Status",
                   "options":Self::options()}}),
            json!({"text":item["origin"],"field":{"id":"FIELD-origin","name":"onetaskgraph.origin"}}),
        ];
        if let Some(labels) = item.get("field_labels") {
            values.push(json!({"labels":{"nodes":labels,"pageInfo":{"hasNextPage":false}}}));
        }
        json!({"id":item["item"],
               "fieldValues":{"nodes":values,"pageInfo":{"hasNextPage":false}},
               "content":self.content(item)})
    }

    /// Every far end one issue is related to, in the direction asked for.
    fn related(&self, id: &str, blocking: bool) -> Value {
        let ids: Vec<String> = if blocking {
            self.blocked_by
                .iter()
                .filter(|(_, blockers)| blockers.iter().any(|blocker| blocker == id))
                .map(|(near, _)| near.clone())
                .collect()
        } else {
            self.blocked_by
                .iter()
                .find(|(near, _)| near == id)
                .map(|(_, blockers)| blockers.clone())
                .unwrap_or_default()
        };
        Value::Array(
            ids.into_iter()
                .map(|id| {
                    let far = self.items.iter().find(|item| item["id"] == json!(id));
                    json!({"id":id,
                           "body":far.map(|item| item["body"].clone()),
                           "parent":far.and_then(|item| item["parent"].as_str())
                               .map(|parent| json!({"id":parent})),
                           "subIssuesSummary":{"total":self.subs(&id)}})
                })
                .collect(),
        )
    }
}

fn github_projects_server(sandbox: &Sandbox, recorded: Option<Value>) -> Value {
    github_projects_board(sandbox, recorded, None).0
}

/// The same board, with a handle on the fields this source must never write.
pub fn github_projects_with_board(sandbox: &Sandbox) -> (Value, GitHubBoardFields) {
    github_projects_board(sandbox, None, None)
}

/// The same board again, failing the first attempt to file a created issue on it.
///
/// Creating an item there is several calls — `createIssue`, then
/// `addProjectV2ItemById`, then the board's own fields — so GitHub can fail part way
/// through, and what the product does with an issue that exists but is on no board is a
/// journey rather than a reading of the code. The failure is spent once, so the same
/// board answers the retry.
pub fn github_projects_failing_to_file_once(sandbox: &Sandbox) -> Value {
    github_projects_board(sandbox, None, Some("addProjectV2ItemById(input:$input)")).0
}

/// The same board, failing the first field write onto an item it has already filed.
///
/// The later half of that sequence: the issue exists and is on the board, and the copy
/// origin and status that make it findable and readable are what did not land.
pub fn github_projects_failing_a_field_write_once(sandbox: &Sandbox) -> Value {
    github_projects_board(
        sandbox,
        None,
        Some("updateProjectV2ItemFieldValue(input:$input)"),
    )
    .0
}

fn github_projects_board(
    sandbox: &Sandbox,
    recorded: Option<Value>,
    fail_first: Option<&'static str>,
) -> (Value, GitHubBoardFields) {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    let board = Arc::new(Mutex::new(GitHubBoard {
        items: github_dataset(recorded.as_ref()),
        pending: Vec::new(),
        blocked_by: github_blockers(),
        created: 0,
        own: json!({"title":"Fixture board",
                    "shortDescription":"the board a person set up",
                    "readme":"# Fixture board\n\nA person wrote this."}),
    }));
    let mut owed_failure = fail_first;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("GitHub fixture connection");
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("GraphQL query string");
            graphql_parser::parse_query::<String>(query).expect("valid GraphQL document");
            let variables = request["variables"]
                .as_object()
                .expect("GraphQL variables object");
            let variables = Value::Object(variables.clone());
            let body = if owed_failure.is_some_and(|operation| query.contains(operation)) {
                owed_failure = None;
                json!({"data":Value::Null,
                       "errors":[{"message":"Something went wrong while executing your query"}]})
                .to_string()
            } else {
                json!({ "data": github_answer(&board, query, &variables) }).to_string()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("GitHub fixture response");
        }
    });
    (
        json!({
            "owner": "fixture-owner",
            "project_number": 7,
            "repository": "nickderobertis/onetaskgraph",
            "token_env": "GITHUB_PROJECTS_FIXTURE_TOKEN",
            "endpoint": endpoint.clone(),
            // `done` and `cancelled` keep their shipped defaults, which close the issue:
            // GitHub derives a project's `Sub-issues progress` from closed sub-issues, so a
            // plan whose finished tasks were only moved to a column reads 0% complete forever.
            "status_mapping": {"todo":"Todo","in-progress":"Doing"}
        }),
        GitHubBoardFields { endpoint },
    )
}

fn github_answer(board: &Arc<Mutex<GitHubBoard>>, query: &str, variables: &Value) -> Value {
    let mut board = board.lock().unwrap();
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if query.contains("repository(owner:$owner,name:$name)") {
        assert_eq!(variables["owner"], "nickderobertis");
        return json!({"repository":{"id":"REPO-1","nameWithOwner":"nickderobertis/onetaskgraph"}});
    }
    if query.contains("createIssue(input:$input)") {
        assert_eq!(input["repositoryId"], "REPO-1");
        assert!(input["title"].as_str().is_some_and(|t| !t.is_empty()));
        assert!(input["body"].is_string() || input["body"].is_null());
        board.created += 1;
        let id = format!("ISSUE-{}", board.created);
        let created = json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
            "title":input["title"],"body":input["body"],"state":"OPEN","reason":null,
            "parent":Value::Null,"repo":"nickderobertis/onetaskgraph","status":"Todo",
            "origin":"","labels":[]});
        board.pending.push(created);
        return json!({"createIssue":{"issue":{"id":id}}});
    }
    if query.contains("addProjectV2ItemById(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        let content = input["contentId"].clone();
        let at = board
            .pending
            .iter()
            .position(|item| item["id"] == content)
            .expect("the issue was created before it was filed");
        let item = board.pending.remove(at);
        let id = item["item"].clone();
        board.items.push(item);
        return json!({"addProjectV2ItemById":{"item":{"id":id}}});
    }
    if query.contains("updateIssue(input:$input)") {
        let held = board.find(&input["id"]);
        if let Some(title) = input["title"].as_str() {
            held["title"] = json!(title);
        }
        if input.get("body").is_some() && input["title"].is_string() {
            held["body"] = input["body"].clone();
        }
        let state = input["stateInput"].clone();
        if !state.is_null() {
            held["state"] = state["value"].clone();
            held["reason"] = state["stateReason"].clone();
        }
        return json!({"updateIssue":{"issue":{"id":input["id"]}}});
    }
    if query.contains("updateProjectV2ItemFieldValue(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        assert!(
            input["value"]
                .as_object()
                .is_some_and(|value| value.len() == 1)
        );
        let item_id = input["itemId"].clone();
        let option = input["value"]["singleSelectOptionId"].as_str().map(|id| {
            GitHubBoard::options()
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["id"] == id)
                .expect("an option this board has")["name"]
                .clone()
        });
        let text = input["value"]["text"].clone();
        let held = board
            .items
            .iter_mut()
            .find(|item| item["item"] == item_id)
            .expect("a field update names a board item");
        if let Some(option) = option {
            held["status"] = option;
        }
        if text.is_string() {
            held["origin"] = text;
        }
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id}}});
    }
    if query.contains("addSubIssue(input:$input)") || query.contains("removeSubIssue(input:$input)")
    {
        let adding = query.contains("addSubIssue(input:$input)");
        let parent = input["issueId"].clone();
        let child = input["subIssueId"].clone();
        board.find(&child)["parent"] = if adding { parent.clone() } else { Value::Null };
        let root = if adding {
            "addSubIssue"
        } else {
            "removeSubIssue"
        };
        return json!({root:{"issue":{"id":parent},"subIssue":{"id":child}}});
    }
    if query.contains("addBlockedBy(input:$input)")
        || query.contains("removeBlockedBy(input:$input)")
    {
        let adding = query.contains("addBlockedBy(input:$input)");
        let issue = input["issueId"].as_str().expect("an issue id").to_owned();
        let blocker = input["blockingIssueId"]
            .as_str()
            .expect("a blocking issue id")
            .to_owned();
        let entry = match board.blocked_by.iter().position(|(near, _)| *near == issue) {
            Some(at) => &mut board.blocked_by[at].1,
            None => {
                board.blocked_by.push((issue.clone(), Vec::new()));
                &mut board.blocked_by.last_mut().unwrap().1
            }
        };
        if adding {
            entry.push(blocker.clone());
        } else {
            entry.retain(|held| held != &blocker);
        }
        let root = if adding {
            "addBlockedBy"
        } else {
            "removeBlockedBy"
        };
        return json!({root:{"issue":{"id":issue},"blockingIssue":{"id":blocker}}});
    }
    if query.contains("updateProjectV2(input:$input)") {
        for field in ["title", "shortDescription", "readme"] {
            if let Some(value) = input.get(field) {
                board.own[field] = value.clone();
            }
        }
        return json!({"updateProjectV2":{"projectV2":{"id":"PVT-board"}}});
    }
    if query.contains("... on ProjectV2{title shortDescription readme}") {
        assert_eq!(variables["id"], "PVT-board");
        return json!({ "node": board.own.clone() });
    }
    if query.contains("node(id:$id)") {
        let id = variables["id"].as_str().expect("dependency id").to_owned();
        let first = variables["first"]
            .as_u64()
            .expect("dependency first must be an unsigned integer");
        assert!(
            (1..=100).contains(&first),
            "dependency first is out of range"
        );
        assert!(
            variables["after"].is_null() || variables["after"].is_string(),
            "dependency after must be null or a string"
        );
        if !board.items.iter().any(|item| item["id"] == json!(id)) {
            return json!({ "node": null });
        }
        return json!({"node":{"__typename":"Issue",
            "blockedBy":{"nodes":board.related(&id, false),
                         "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":board.related(&id, true),
                        "pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    }
    assert!(
        query.contains("owner:repositoryOwner"),
        "fixture received an unknown GraphQL operation"
    );
    assert_eq!(variables["owner"], "fixture-owner");
    assert_eq!(variables["number"], 7);
    assert_eq!(variables["nestedFirst"], 50);
    assert_eq!(variables["duplicates"], json!(true));
    let offset = match &variables["after"] {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
        other => panic!("GraphQL after must be null or a numeric string: {other}"),
    };
    let first = usize::try_from(
        variables["first"]
            .as_u64()
            .expect("GraphQL first must be an unsigned integer"),
    )
    .expect("GraphQL first fits usize");
    assert!(first > 0, "GraphQL first must be positive");
    assert!(
        offset <= board.items.len(),
        "GraphQL after cursor is out of range"
    );
    let end = (offset + first).min(board.items.len());
    let nodes = board.items[offset..end]
        .iter()
        .map(|item| board.rendered(item))
        .collect::<Vec<_>>();
    let title = board.own["title"].clone();
    json!({"owner":{"projectV2":{"id":"PVT-board","title":title,
        "fields":GitHubBoard::fields(),
        "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < board.items.len(),
                                           "endCursor":end.to_string()}}}}})
}

fn read_http_json(stream: &mut impl Read) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("fixture request");
        assert!(count > 0, "fixture request ended before its HTTP headers");
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP header terminator")
        + 4;
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(headers.contains("authorization: Bearer test-token"));
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .expect("Content-Length");
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("fixture request body");
        assert!(count > 0, "fixture request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).expect("request JSON")
}

/// A socket-level Linear GraphQL fixture used by the shared binary journeys.
pub fn linear_block(sandbox: &Sandbox) -> Value {
    linear_server(sandbox, None)
}

/// The same workspace, with the item a dependency read asks about recording `recorded`
/// under the reserved dependency key.
///
/// The counterpart of [`github_projects_recording`], and it exists for the same reason:
/// the shared row is the one every other journey reads, so a workspace holding a key it
/// must not cannot be that row.
pub fn linear_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    linear_server(sandbox, Some(recorded))
}

fn linear_server(sandbox: &Sandbox, recorded: Option<Value>) -> Value {
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(dataset()));
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > 8_192 {
                    break;
                }
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.parse::<usize>().ok());
                    if length.is_some_and(|length| bytes.len() >= split + 4 + length) {
                        break;
                    }
                }
            }
            if bytes.len() > 8_192 {
                let text = r#"{"errors":[{"message":"fixture request too large"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 413 Content Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let split = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|n| n + 4)
                .unwrap_or(bytes.len());
            let request_head = String::from_utf8_lossy(&bytes[..split]);
            let declared_length = request_head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse::<usize>().ok());
            if declared_length.is_none_or(|length| bytes.len() != split + length) {
                let text = r#"{"errors":[{"message":"invalid content length"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let request_line = request_head
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r');
            if request_line != "POST /graphql HTTP/1.1" {
                let text = r#"{"errors":[{"message":"expected POST /graphql"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let Ok(request) = serde_json::from_slice::<Value>(&bytes[split..]) else {
                let text = r#"{"errors":[{"message":"invalid fixture request"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            };
            if request.get("query").and_then(Value::as_str).is_none() {
                let text = r#"{"errors":[{"message":"missing GraphQL query"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let (status, response) =
                match linear_response(&request, recorded.as_ref(), &mut state.lock().unwrap()) {
                    Ok(body) => ("200 OK", json!({"data":body})),
                    Err(message) => ("400 Bad Request", json!({"errors":[{"message":message}]})),
                };
            let text = serde_json::to_string(&response).unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    json!({"endpoint":endpoint,"team":"FIX"})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinearRequest {
    query: String,
    variables: serde_json::Map<String, Value>,
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] These typed fixture-boundary variables mirror the accepted 2026-08-24 Linear documents; the authoritative variable/nullability contract is available only from Linear's authenticated unversioned explorer, while focused TCP tests prove malformed local requests are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoVariables {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemVariables {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
    filter: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationVariables {
    id: String,
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

fn validate_linear_variables(operation: &str, variables: &Value) -> Result<(), &'static str> {
    use onetaskgraph_linear::graphql;
    let valid = match operation {
        graphql::VIEWER => serde_json::from_value::<NoVariables>(variables.clone()).is_ok(),
        graphql::ISSUE | graphql::PROJECT => {
            serde_json::from_value::<ItemVariables>(variables.clone())
                .is_ok_and(|variables| !variables.id.is_empty())
        }
        graphql::LABELS => serde_json::from_value::<PageVariables>(variables.clone())
            .is_ok_and(|variables| variables.first > 0 && variables.after.as_deref() != Some("")),
        graphql::ISSUES | graphql::PROJECTS => {
            serde_json::from_value::<QueryVariables>(variables.clone()).is_ok_and(|variables| {
                variables.first > 0
                    && variables.after.as_deref() != Some("")
                    && valid_linear_filter(&Value::Object(variables.filter))
            })
        }
        graphql::ISSUE_RELATIONS | graphql::PROJECT_RELATIONS => {
            serde_json::from_value::<RelationVariables>(variables.clone()).is_ok_and(|variables| {
                !variables.id.is_empty()
                    && variables.first > 0
                    && variables.after.as_deref() != Some("")
            })
        }
        graphql::TEAM => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("key").is_some_and(|value| !value.is_empty())
                })
        }
        graphql::ISSUE_STATE => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 2
                        && ["name", "team"]
                            .iter()
                            .all(|key| values.get(*key).is_some_and(|value| !value.is_empty()))
                })
        }
        graphql::PROJECT_STATUS | graphql::ISSUE_LABEL | graphql::PROJECT_LABEL => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("name").is_some_and(|value| !value.is_empty())
                })
        }
        graphql::ISSUE_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamId", "title", "stateId", "labelIds"],
                    &["description", "projectId"],
                )
        }
        graphql::PROJECT_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamIds", "name", "statusId", "labelIds"],
                    &["description"],
                )
        }
        graphql::ISSUE_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["issueId", "relatedIssueId", "type"],
                    &[],
                )
        }
        graphql::PROJECT_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["projectId", "relatedProjectId", "type"],
                    &[],
                )
        }
        graphql::ISSUE_RELATION_DELETE | graphql::PROJECT_RELATION_DELETE => {
            exact_linear_variable_keys(variables, &["id"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
        }
        graphql::ISSUE_UPDATE | graphql::PROJECT_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(
                    variables.get("input"),
                    if operation == graphql::ISSUE_UPDATE {
                        &["title", "stateId", "labelIds"]
                    } else {
                        &["name", "statusId", "labelIds"]
                    },
                    if operation == graphql::ISSUE_UPDATE {
                        &["description", "projectId"]
                    } else {
                        &["description"]
                    },
                )
        }
        _ => false,
    };
    valid.then_some(()).ok_or("invalid operation variables")
}

fn exact_linear_variable_keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == expected.len() && expected.iter().all(|key| fields.contains_key(*key))
    })
}

fn valid_linear_write_input(value: Option<&Value>, required: &[&str], optional: &[&str]) -> bool {
    let Some(fields) = value.and_then(Value::as_object) else {
        return false;
    };
    if fields
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        || required.iter().any(|key| !fields.contains_key(*key))
    {
        return false;
    }
    fields.iter().all(|(key, value)| match key.as_str() {
        "labelIds" | "teamIds" => value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| value.as_str().is_some_and(|id| !id.is_empty()))
        }),
        "description" => value.is_null() || value.is_string(),
        "projectId" => value.is_null() || value.as_str().is_some_and(|id| !id.is_empty()),
        _ => value.as_str().is_some_and(|text| !text.is_empty()),
    })
}

fn valid_linear_filter(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().all(|(key, value)| match key.as_str() {
            "and" => value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|value| value.is_object() && valid_linear_filter(value))
            }),
            "team" | "key" | "labels" | "some" | "name" | "every" | "state" | "type"
            | "project" | "id" => value.is_object() && valid_linear_filter(value),
            "eqIgnoreCase" | "neqIgnoreCase" | "eq" => value.is_string(),
            "inIgnoreCase" | "in" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "null" => value.is_boolean(),
            _ => false,
        }),
        _ => false,
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

fn linear_response(
    request: &Value,
    recorded: Option<&Value>,
    data: &mut Value,
) -> Result<Value, &'static str> {
    let request: LinearRequest =
        serde_json::from_value(request.clone()).map_err(|_| "invalid GraphQL request")?;
    use onetaskgraph_linear::graphql;
    let operation = request.query.as_str();
    if ![
        graphql::VIEWER,
        graphql::ISSUE,
        graphql::PROJECT,
        graphql::ISSUES,
        graphql::PROJECTS,
        graphql::LABELS,
        graphql::ISSUE_RELATIONS,
        graphql::PROJECT_RELATIONS,
        graphql::TEAM,
        graphql::ISSUE_STATE,
        graphql::PROJECT_STATUS,
        graphql::ISSUE_LABEL,
        graphql::PROJECT_LABEL,
        graphql::ISSUE_CREATE,
        graphql::ISSUE_UPDATE,
        graphql::PROJECT_CREATE,
        graphql::PROJECT_UPDATE,
        graphql::ISSUE_RELATION_CREATE,
        graphql::PROJECT_RELATION_CREATE,
        graphql::ISSUE_RELATION_DELETE,
        graphql::PROJECT_RELATION_DELETE,
    ]
    .contains(&operation)
    {
        return Err("unknown GraphQL operation");
    }
    let vars = Value::Object(request.variables);
    validate_linear_variables(operation, &vars)?;
    if operation == graphql::TEAM {
        return Ok(json!({"teams":{"nodes":[{"id":"TEAM-1"}]}}));
    }
    if operation == graphql::ISSUE_STATE {
        return Ok(json!({"workflowStates":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::PROJECT_STATUS {
        return Ok(json!({"projectStatuses":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::ISSUE_LABEL {
        return Ok(json!({"issueLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::PROJECT_LABEL {
        return Ok(json!({"projectLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    if matches!(operation, graphql::ISSUE_CREATE | graphql::ISSUE_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::ISSUE_CREATE, false);
    }
    if matches!(operation, graphql::PROJECT_CREATE | graphql::PROJECT_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::PROJECT_CREATE, true);
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_CREATE | graphql::PROJECT_RELATION_CREATE
    ) {
        return linear_write_relation(data, &vars, operation == graphql::PROJECT_RELATION_CREATE);
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_DELETE | graphql::PROJECT_RELATION_DELETE
    ) {
        let project = operation == graphql::PROJECT_RELATION_DELETE;
        let index = vars["id"]
            .as_str()
            .and_then(|id| id.rsplit(':').next())
            .and_then(|id| id.parse::<usize>().ok())
            .ok_or("invalid relation fixture id")?;
        let edges = data[if project {
            "project_dependencies"
        } else {
            "task_dependencies"
        }]
        .as_array_mut()
        .ok_or("fixture edges are not an array")?;
        if index < edges.len() {
            edges.remove(index);
        }
        return Ok(if project {
            json!({"projectRelationDelete":{"success":true}})
        } else {
            json!({"issueRelationDelete":{"success":true}})
        });
    }
    if operation == graphql::LABELS {
        return Ok(
            json!({"issueLabels":linear_connection(data["labels"].as_array().unwrap().iter().map(linear_label).collect(),&vars)}),
        );
    }
    if operation == graphql::ISSUES {
        let mut rows: Vec<Value> = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_matches_fixture_subset(v, &vars))
            .map(|v| linear_task(v, data))
            .collect();
        return Ok(json!({"issues":linear_connection(std::mem::take(&mut rows),&vars)}));
    }
    if operation == graphql::PROJECTS {
        let rows = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_matches_fixture_subset(v, &vars))
            .map(|v| linear_project(v, data))
            .collect();
        return Ok(json!({"projects":linear_connection(rows,&vars)}));
    }
    if matches!(operation, graphql::ISSUE | graphql::ISSUE_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::ISSUE_RELATIONS {
            return Ok(
                json!({"issue":linear_relations(data,"task_dependencies",id,"Issue",recorded)}),
            );
        }
        return Ok(json!({"issue":item.map(|v|linear_task(v,data))}));
    }
    if matches!(operation, graphql::PROJECT | graphql::PROJECT_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::PROJECT_RELATIONS {
            return Ok(
                json!({"project":linear_relations(data,"project_dependencies",id,"Project",recorded)}),
            );
        }
        return Ok(json!({"project":item.map(|v|linear_project(v,data))}));
    }
    Ok(json!({"viewer":{"id":"fixture-user"}}))
}

fn linear_write_item(
    data: &mut Value,
    vars: &Value,
    create: bool,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("write input must be an object")?;
    let collection = if project { "projects" } else { "tasks" };
    let rows = data[collection]
        .as_array_mut()
        .ok_or("fixture collection is not an array")?;
    let id = if create {
        format!("{}-W{}", if project { "P" } else { "T" }, rows.len() + 1)
    } else {
        vars["id"]
            .as_str()
            .ok_or("update id must be a string")?
            .to_owned()
    };
    let existing = rows.iter().position(|row| row["id"] == id);
    if !create && existing.is_none() {
        return Err("update target does not exist");
    }
    let title_key = if project { "name" } else { "title" };
    let status_key = if project { "statusId" } else { "stateId" };
    let labels = input
        .get("labelIds")
        .and_then(Value::as_array)
        .ok_or("labelIds must be an array")?
        .iter()
        .map(|id| json!({"id":id,"name":id}))
        .collect::<Vec<_>>();
    let mut row = json!({
        "id": id,
        "title": input.get(title_key).and_then(Value::as_str).ok_or("title must be a string")?,
        "content": "",
        "status": {"name":input.get(status_key).and_then(Value::as_str).ok_or("status id must be a string")?,"category":"todo"},
        "labels": labels,
        "_linear_description": input.get("description").cloned().unwrap_or(Value::Null),
    });
    if !project && let Some(project_id) = input.get("projectId").filter(|v| !v.is_null()) {
        row["project"] = project_id.clone();
    }
    if let Some(index) = existing {
        rows[index] = row;
    } else {
        rows.push(row);
    }
    let payload = json!({"id":id});
    Ok(if project {
        if create {
            json!({"projectCreate":{"success":true,"project":payload}})
        } else {
            json!({"projectUpdate":{"success":true,"project":payload}})
        }
    } else if create {
        json!({"issueCreate":{"success":true,"issue":payload}})
    } else {
        json!({"issueUpdate":{"success":true,"issue":payload}})
    })
}

fn linear_write_relation(
    data: &mut Value,
    vars: &Value,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("relation input must be an object")?;
    let near_key = if project { "projectId" } else { "issueId" };
    let far_key = if project {
        "relatedProjectId"
    } else {
        "relatedIssueId"
    };
    let kind = input
        .get("type")
        .and_then(Value::as_str)
        .ok_or("relation type must be a string")?;
    if !matches!(kind, "blocks" | "related") {
        return Err("undocumented relation type");
    }
    let edge = json!({"from":input.get(near_key).ok_or("missing near id")?,"to":input.get(far_key).ok_or("missing far id")?,"kind":kind});
    data[if project {
        "project_dependencies"
    } else {
        "task_dependencies"
    }]
    .as_array_mut()
    .ok_or("fixture edges are not an array")?
    .push(edge);
    Ok(if project {
        json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"PR-W"}}})
    } else {
        json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"IR-W"}}})
    })
}

#[test]
// llmlint: ignore[tests_mirror_real_usage] This is a failure test for the fixture server's own untrusted HTTP boundary, which product CLI requests cannot malformedly exercise because the Linear client always emits valid typed requests; it intentionally sends raw TCP requests through the real socket rather than calling response logic directly.
fn linear_fixture_rejects_invalid_variables_and_unknown_operations() {
    let sandbox = Sandbox::new();
    let config = linear_block(&sandbox);
    let endpoint = config["endpoint"].as_str().unwrap();
    let address = endpoint
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/graphql")
        .unwrap();
    for body in [
        json!({"query":onetaskgraph_linear::graphql::VIEWER}),
        json!({"query":onetaskgraph_linear::graphql::VIEWER,"variables":[]}),
        json!({"query":"query { invented { id } }","variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":7}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":"i1","extra":true}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":0,"after":null,"filter":{}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":[]}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"invented":true}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"state":{"type":{"in":7}}}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"and":"invalid"}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"project":{"null":"true"}}}}),
    ] {
        let body = serde_json::to_string(&body).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        write!(
            stream,
            "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
    }
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(stream, "GET /graphql HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the invalid-method fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: 9\r\n\r\n{{}}"
    )
    .unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the short-body fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let oversized = "x".repeat(8_193);
    let _ = write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n{oversized}",
        oversized.len()
    );
    // The fixture can reject and close this oversized request before the client reaches
    // shutdown, so a half-close here would race with the expected server response.
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 413 Content Too Large"));
}
/// The far ends `id` records under the reserved key, for a source with no native way to
/// name one: every qualified endpoint the dataset gives that item at `key`.
fn recorded_far_ends(key: &str, id: &Value) -> Vec<Value> {
    dataset()[key]
        .as_array()
        .expect("the dataset lists edges")
        .iter()
        .filter(|edge| edge["from"].get("id") == Some(id))
        .map(|edge| edge["to"].clone())
        .collect()
}
fn linear_label(v: &Value) -> Value {
    json!({"id":v["id"],"name":v["name"],"color":null})
}
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This fixture-only mapping and matcher implement the finite shared journey dataset against the accepted 2026-08-24 contract; production parsing and real CLI row assertions independently verify the observable behavior without requiring live credentials.
fn linear_state(v: &Value) -> Value {
    let category = v["category"].as_str().unwrap_or("");
    json!({"name":v["name"],"type":match category{"todo"=>"unstarted","in-progress"=>"started","done"=>"completed","cancelled"=>"canceled",_=>"backlog"}})
}
fn linear_task(v: &Value, data: &Value) -> Value {
    json!({"id":v["id"],"title":v["title"],"description":linear_description(v,"task_dependencies",data),"state":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"project":v.get("project").map(|id|json!({"id":id})),"url":v.get("url"),"createdAt":null,"updatedAt":null})
}
fn linear_project(v: &Value, data: &Value) -> Value {
    json!({"id":v["id"],"name":v["title"],"description":linear_description(v,"project_dependencies",data),"status":linear_state(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"url":v.get("url"),"createdAt":null,"updatedAt":null})
}
fn linear_description(v: &Value, edges: &str, data: &Value) -> String {
    if let Some(description) = v.get("_linear_description").and_then(Value::as_str) {
        return description.to_owned();
    }
    let mut metadata = v
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(repositories) = v.get("repositories") {
        metadata.insert("onetaskgraph.repositories".into(), repositories.clone());
    }
    // No Linear relation can name an item of another source, so this is the one slot a
    // far end like that can be in.
    let far = data[edges]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["from"].get("id") == Some(&v["id"])
                && edge["to"]["id"].as_str().is_some_and(|id| id.contains(':'))
        })
        .map(|edge| edge["to"].clone())
        .collect::<Vec<_>>();
    if !far.is_empty() {
        metadata.insert("onetaskgraph.depends_on".into(), Value::Array(far));
    }
    let content = v.get("content").and_then(Value::as_str).unwrap_or_default();
    if metadata.is_empty() {
        content.into()
    } else {
        linear_metadata_slot(content, &Value::Object(metadata))
    }
}

/// The one slot a Linear item keeps caller-defined metadata in: an HTML comment appended
/// to the description, which is what the source reads and what a person never sees.
fn linear_metadata_slot(content: &str, metadata: &Value) -> String {
    format!(
        "{content}\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        serde_json::to_string(metadata).unwrap()
    )
}
fn linear_connection(rows: Vec<Value>, vars: &Value) -> Value {
    let start = vars["after"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit = vars["first"].as_u64().unwrap_or(50) as usize;
    let nodes = rows
        .iter()
        .skip(start)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let end = start + nodes.len();
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":end<rows.len(),"endCursor":if end<rows.len(){Some(end.to_string())}else{None}}})
}
fn linear_matches_fixture_subset(v: &Value, vars: &Value) -> bool {
    let text = vars["filter"].to_string().to_ascii_lowercase();
    let labels = v["labels"].as_array().unwrap();
    for name in ["bug", "chore", "core"] {
        if text.contains(&format!("\"{name}\"")) {
            let present = labels.iter().any(|l| l["name"].as_str() == Some(name));
            let excluded = text.contains(&format!("neqignorecase\":\"{name}"));
            if (excluded && present) || (!excluded && !present) {
                return false;
            }
        }
    }
    let mut allowed = Vec::new();
    for (linear, category) in [
        ("completed", "done"),
        ("unstarted", "todo"),
        ("\"started\"", "in-progress"),
        ("backlog", "backlog"),
        ("canceled", "cancelled"),
    ] {
        if text.contains(linear) {
            allowed.push(category);
        }
    }
    if !allowed.is_empty() && !allowed.contains(&v["status"]["category"].as_str().unwrap_or("")) {
        return false;
    }
    if text.contains("\"null\":true") && v.get("project").is_some() {
        return false;
    }
    for id in ["p-1", "p-2"] {
        if text.contains(id)
            && v.get("project")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(id)
        {
            return false;
        }
    }
    true
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
fn linear_relations(
    data: &Value,
    key: &str,
    id: &str,
    suffix: &str,
    recorded: Option<&Value>,
) -> Value {
    let edges = data[key].as_array().unwrap();
    // A Linear relation names a Linear item, so only the edges whose ends are both plain
    // native ids are here. The rest are in the item's own description slot, which this
    // operation selects for exactly that reason.
    let forward = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["from"] == id && e["to"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":e["kind"],(format!("related{suffix}")):{"id":e["to"]}}))
        .collect::<Vec<_>>();
    let inverse = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["to"] == id && e["from"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":e["kind"],(suffix.to_ascii_lowercase()):{"id":e["from"]}}))
        .collect::<Vec<_>>();
    let items = if suffix == "Issue" {
        "tasks"
    } else {
        "projects"
    };
    let item = data[items]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id);
    // The description slot is where this source reads a recorded far end from, so a
    // workspace built to hold one puts it here and leaves every other operation alone.
    let description = match recorded {
        Some(recorded) => Some(linear_metadata_slot(
            "",
            &json!({"onetaskgraph.depends_on": recorded}),
        )),
        None => item.map(|item| linear_description(item, key, data)),
    };
    json!({"description":description,"relations":{"nodes":forward,"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":inverse,"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

fn local_md_block(sandbox: &Sandbox) -> Value {
    let root = sandbox.subdirectory("local-md");
    for (kind, id, front, body) in [
        (
            "tasks",
            "T-1",
            "title: Alpha engine\nstatus: Todo\nlabels: [{id: L-1, name: bug}, {id: L-3, name: core}]\nproject: P-1\nurl: https://example.invalid/T-1\nmetadata: {onepipeline.turn_budget: 12, caller.flags: [true, null]}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [T-2, {id: \"elsewhere:P-9\", item: project}]",
            "the engine core",
        ),
        (
            "tasks",
            "T-2",
            "title: Beta\nstatus: Shipped\nlabels: [{id: L-2, name: chore}]\nproject: P-1",
            "alpha in the body",
        ),
        (
            "tasks",
            "T-3",
            "title: Gamma\nstatus: Todo\nlabels: [{id: L-1, name: bug}]\ndepends_on: [T-2]",
            "unrelated",
        ),
        (
            "tasks",
            "T-4",
            "title: Delta docs\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nproject: P-2\ndepends_on:\n  - id: T-2\n    kind: related",
            "documentation",
        ),
        (
            "projects",
            "P-1",
            "title: Engine\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nurl: https://example.invalid/P-1\nmetadata: {onepipeline.publication: {mode: review}}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [P-2, {id: \"elsewhere:T-9\", item: task}]",
            "the engine",
        ),
        ("projects", "P-2", "title: Docs\nstatus: Todo", "alpha docs"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
        std::fs::write(path, format!("---\n{front}\n---\n{body}\n")).expect("Markdown fixture");
    }
    json!({ "root": root, "status_mapping": {"todo":"todo", "doing":"in-progress", "shipped":"done"} })
}

/// The `in-memory` row that applies every predicate itself.
fn native_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = json!({"max_page_size": 50});
    block
}

/// The row that runs the same dataset in a second process, over the stdio protocol.
///
/// This is journey 19 — every journey again, through a subprocess-wrapped source — and it
/// is a row rather than a suite of its own for the reason the whole table exists: a
/// transport proven by tests written for it is proven against its author's expectations,
/// and this one has to answer the same assertions every in-process source answers. It
/// declares everything native because the source behind the pipe does, which is the claim
/// worth making: what a source can do must not change because it is a process away.
fn hosted_block(_sandbox: &Sandbox) -> Value {
    let mut settings = dataset();
    settings["capabilities"] = json!({"max_page_size": 50});
    json!({
        "command": env!("CARGO_BIN_EXE_onetaskgraph"),
        "args": ["plugin-serve", "in-memory"],
        "settings": settings,
    })
}

/// The `in-memory` row that applies none of them, and pages two rows at a time.
///
/// A small page ceiling on purpose: compensation has to walk more than one page to find
/// the rows a filter keeps, and a ceiling of two is what makes a journey notice when it
/// stops doing so.
fn compensated_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = json!({
        "filter_by_label": "unsupported",
        "filter_by_status": "unsupported",
        "search_title": "unsupported",
        "search_content": "unsupported",
        "orphan_tasks": "unsupported",
        "task_dependencies": "forward-only",
        "project_dependencies": "forward-only",
        "max_page_size": 2
    });
    block
}

/// The work every row serves: four tasks, three of which are in a project, three labels,
/// two projects, and a dependency graph with a reverse answer worth checking.
///
/// Exactly one of the two projects carries a label, and the two sit in different status
/// categories, so every project filter has something to keep and something to drop —
/// a filter both projects satisfied would pass against a source that ignored it.
///
/// The dependency edges are listed in the order their `from` items are, which is what
/// makes the engine's emulated reverse scan — item by item, each item's forward edges in
/// order — produce the *same sequence* a source answering natively does, rather than the
/// same set in another order. A fixture that shuffled them would make the two answers
/// compare unequal for a reason that has nothing to do with the engine.
///
/// Two of the edges leave this source altogether — one from a task to a project, one from
/// a project to a task, both in a source called `elsewhere` that is not configured at all.
/// They are here rather than in a journey of their own because *where* such an edge is
/// held is each source's own business: a native relation that can name the far end, and
/// the reserved key on the near item where none can. Every row below encodes these two in
/// its own way, and one journey asserts that all of them report the same edge.
pub fn dataset() -> Value {
    json!({
        "tasks": [
            {"id": "T-1", "title": "Alpha engine", "content": "the engine core",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}, {"id": "L-3", "name": "core"}],
            "project": "P-1", "url": "https://example.invalid/T-1",
            "metadata": {"onepipeline.turn_budget": 12, "caller.flags": [true, null]},
            "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "T-2", "title": "Beta", "content": "alpha in the body",
             "status": {"category": "done", "name": "Shipped"},
             "labels": [{"id": "L-2", "name": "chore"}], "project": "P-1"},
            {"id": "T-3", "title": "Gamma", "content": "unrelated",
             "status": {"category": "todo", "name": "Todo"},
             "labels": [{"id": "L-1", "name": "bug"}]},
            {"id": "T-4", "title": "Delta docs", "content": "documentation",
             "status": {"category": "in-progress", "name": "Doing"},
             "labels": [{"id": "L-3", "name": "core"}], "project": "P-2"}
        ],
        "projects": [
            {"id": "P-1", "title": "Engine", "content": "the engine",
             "status": {"category": "in-progress", "name": "Doing"},
             "labels": [{"id": "L-3", "name": "core"}],
             "url": "https://example.invalid/P-1",
             "metadata": {"onepipeline.publication": {"mode": "review"}},
             "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "P-2", "title": "Docs", "content": "alpha docs",
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ],
        "labels": [
            {"id": "L-1", "name": "bug"},
            {"id": "L-2", "name": "chore"},
            {"id": "L-3", "name": "core"}
        ],
        "task_dependencies": [
            {"from": "T-1", "to": "T-2", "kind": "blocks"},
            {"from": {"id": "T-1", "kind": "task"},
             "to": {"id": "elsewhere:P-9", "kind": "project"}, "kind": "blocks"},
            {"from": "T-3", "to": "T-2", "kind": "blocks"},
            {"from": "T-4", "to": "T-2", "kind": "related"}
        ],
        "project_dependencies": [
            {"from": "P-1", "to": "P-2", "kind": "blocks"},
            {"from": {"id": "P-1", "kind": "project"},
             "to": {"id": "elsewhere:T-9", "kind": "task"}, "kind": "blocks"}
        ]
    })
}
