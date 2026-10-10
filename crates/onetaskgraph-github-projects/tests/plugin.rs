//! The source's own suite, driven over a real loopback socket against a board fixture.
//!
//! Nothing here mocks the layer under test: every test builds the plugin through
//! `SourcePlugin::build`, and every request it makes is a real HTTP POST carrying a real
//! GraphQL document, answered by a fixture that keeps board state the way GitHub does.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind,
    DependencySupport, Direction, Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter,
    Location, MetadataKey, MetadataMatch, NativeId, NewComment, PageRequest, Priority, Project,
    ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskRef, TaskSource, TaskUpdate, TextFields,
    TextQuery, UpdatedField, WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

// The credentialed lane's own halves, driven here without a credential: `journey` is the
// whole session this board is asked to cost, and it reaches `lane` for the decisions that
// bound where a run may write. Most of what each holds is for the target that runs it
// against GitHub, so the parts this one does not reach are not dead code — they are the
// other drive's.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

// The half of this board that answers the journey's own calls rather than board state — the
// probe, the introspection and the allowance read. It lives beside this file because
// `tests/reconciliation_gate.rs` answers those same calls with the same code, against a
// board configured to report a price this workspace does not compute.
#[allow(dead_code)]
mod board;

use board::{FIXTURE_BUDGET_LIMIT, Pricing, ample_allowance};

// `status_mapping` scoped by item kind, over this file's board: a module of this target
// rather than a target of its own because it is this board it drives.
mod status_by_kind;

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, var: &str) -> Option<SecretString> {
        (var == "GH_PROJECTS_TOKEN").then(|| "test-token".into())
    }
}

#[tokio::test]
async fn guarded_status_options_preserve_the_complete_existing_options_and_assignments() {
    let fixture = board(vec![Item::issue("I_task", "task").status("Todo")]);
    fixture.status_options_missing_queued();
    let report = status_options_source(&fixture)
        .status_options(StatusOptionsMode::Apply)
        .await
        .expect("the guarded addition is verified");
    assert_eq!(report.outcome, StatusOptionsOutcome::Applied);
    assert_eq!(report.missing, ["Queued"]);
    assert!(report.existing.iter().any(|option| {
        serde_json::to_value(option).unwrap()
            == json!({"id":"OPT_todo","name":"Todo","color":"BLUE","description":"ready"})
    }));
    let update = fixture
        .seen()
        .into_iter()
        .find(|entry| entry[0] == "updateProjectV2Field")
        .expect("the fixture received the guarded mutation");
    assert!(
        update[1]["singleSelectOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|option| option["id"] == "OPT_shipped"
                && option["color"] == "PURPLE"
                && option["description"] == "custom")
    );
}

#[tokio::test]
async fn guarded_status_options_count_a_terminal_category_s_option_as_configured() {
    // A terminal write selects its mapped option before it closes, and refuses when the
    // board lacks it — so the guarded plan names that option exactly as it names an open
    // category's, and a terminal category an operator disabled is not asked for.
    let fixture = board(vec![Item::issue("I_task", "task").status("Todo")]);
    fixture.status_options_missing_terminals();
    let report = status_options_source(&fixture)
        .status_options(StatusOptionsMode::Plan)
        .await
        .expect("a plan reads without writing");
    assert_eq!(report.outcome, StatusOptionsOutcome::Planned);
    assert_eq!(report.missing, ["Done", "Cancelled"]);
    assert!(fixture.seen().is_empty(), "a plan sends no mutation");

    let fixture = board(vec![Item::issue("I_task", "task").status("Todo")]);
    fixture.status_options_missing_terminals();
    let config: GitHubProjectsConfig = serde_json::from_value(fixture_config(
        &fixture.endpoint,
        &json!({"status_mapping": {"done": "Shipped", "cancelled": null}}),
    ))
    .unwrap();
    let source =
        GitHubProjectsSource::new(&SourceName::new("work").unwrap(), config, &Secrets).unwrap();
    let report = source
        .status_options(StatusOptionsMode::Plan)
        .await
        .expect("a plan reads without writing");
    assert_eq!(report.missing, ["Shipped"]);
}

#[tokio::test]
async fn guarded_status_options_refuse_existing_option_metadata_drift_with_recovery_data() {
    let fixture = board(vec![Item::issue("I_task", "task").status("Todo")]);
    fixture.status_options_missing_queued();
    fixture.drift_guarded_status_description();
    let error = status_options_source(&fixture)
        .status_options(StatusOptionsMode::Apply)
        .await
        .expect_err("changed option metadata is drift");
    let complaint = error.to_string();
    assert!(complaint.contains("color or description"), "{complaint}");
    assert!(complaint.contains("I_task"), "{complaint}");
    assert!(complaint.contains("OPT_todo"), "{complaint}");
}

#[tokio::test]
async fn guarded_status_options_refuse_blank_external_snapshot_ids() {
    for (target, expected) in [
        ("board", "board id"),
        ("field", "Status field id"),
        ("item", "item id"),
        ("cursor", "pagination cursor"),
    ] {
        let fixture = board(vec![Item::issue("I_task", "task").status("Todo")]);
        fixture.blank_status_snapshot_id(target);
        let error = status_options_source(&fixture)
            .status_options(StatusOptionsMode::Plan)
            .await
            .expect_err("a blank external identifier is refused");
        let complaint = error.to_string();
        assert!(
            complaint.contains(if target == "cursor" {
                "blank string field endCursor"
            } else {
                "blank string field id"
            }),
            "{expected}: {complaint}"
        );
        assert!(fixture.seen().is_empty(), "{expected}: nothing is written");
    }
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

fn resume(cursor: &str, limit: u32) -> PageRequest {
    PageRequest {
        cursor: Some(Cursor(cursor.to_owned())),
        limit,
    }
}

/// One issue or draft as the fixture holds it.
#[derive(Clone)]
struct Item {
    item_id: String,
    content_id: String,
    typename: &'static str,
    title: String,
    body: Option<String>,
    state: &'static str,
    state_reason: Option<String>,
    /// This issue's own number on its repository, which is the short handle GitHub shows
    /// people and this source reports as a task's `key`.
    ///
    /// Every fixture issue wears the same one unless a case says otherwise, because only
    /// the cases about the handle care which number it is — and those set their own, so
    /// two issues they compare cannot accidentally agree. A draft carries it and never
    /// sends it: `DraftIssue` declares no `number`, which is what having no handle looks
    /// like on the wire.
    number: u64,
    parent: Option<String>,
    sub_issues: u64,
    /// `owner/name` of the repository this issue is in — the one the `createIssue` that
    /// made it named by node id, or the board's own for an issue a case seeded.
    repository: Option<String>,
    labels: Vec<(&'static str, &'static str)>,
    status: Option<String>,
    origin: Option<String>,
    /// Other boards this issue sits on, ahead of this one in `Issue.projectItems`.
    ///
    /// GitHub answers that connection a page at a time and this board is not obliged to be
    /// on the first one, which is the shape the recovery read exists for: with enough of
    /// these ahead of it, the entry naming this board sits past the page a read carries.
    other_boards: Vec<u64>,
    /// Whether this issue's memberships name this board at all.
    ///
    /// An issue GitHub happily resolves that is on other boards and not on this one — which
    /// is a different answer from an issue whose entry is merely unreached, and the one
    /// case a walk to exhaustion has to be able to tell apart from it.
    on_this_board: bool,
    /// The board's node id as this issue's own entry for the board under test names it.
    ///
    /// The board's real id unless a case has made the entry name something no write could
    /// address, which is what an update reading its board off the item has to refuse.
    board_entry_id: &'static str,
    /// Whether the read of this issue by its own id carries, ahead of this board's field
    /// definitions, those of another owner's board that shares this board's project number.
    ///
    /// A project number is unique only within its owner, so a number alone does not say
    /// which entry of `boards` holds this board's fields; the board item's own project id
    /// does.
    fields_of_a_namesake_ahead: bool,
    /// Whether GitHub's own enumerations of the board — `ProjectV2.items` and the
    /// board-scoped issue search — list this item yet.
    ///
    /// An item added to a board can appear in its own membership before it appears in
    /// `ProjectV2.items`. This flag models that state in the fixture board.
    listed: bool,
    /// Whether this item holds a value of the board's origin text field.
    ///
    /// GitHub answers `fieldValues` with the values an item holds and nothing for a field
    /// it holds none of, so an item nobody ever copied carries no definition of that field.
    origin_value: bool,
    /// When GitHub last saw this issue change, as `Issue.updatedAt` carries it — `None` for
    /// the `null` every case that does not care about it is answered with.
    ///
    /// GitHub moves it when a comment on the issue is added **or edited**, which is what its
    /// issue search's `updated:` qualifier is read against; see [`Item::updated`].
    updated_at: Option<String>,
    /// A label set this board answers one path with, instead of the one above.
    ///
    /// Nothing GitHub does. It is how the four-way equivalence check is watched failing:
    /// a check that agrees with itself over every tree is not evidence that it would
    /// catch a path serving another path's answer.
    path_labels: BTreeMap<&'static str, Vec<(&'static str, &'static str)>>,
    /// What this item holds in each of the board's own text fields other than the origin's,
    /// by field id — a field holding nothing is absent, as GitHub answers it.
    texts: BTreeMap<String, String>,
}

/// What the document that just arrived asked for, as far as rendering an item needs it.
///
/// Read off the document rather than assumed, so this board answers what was selected
/// rather than what a test wished for.
#[derive(Clone, Copy)]
struct Asked<'a> {
    /// Which of this source's reads this is, by the name `operation_name` gives it.
    path: &'a str,
    /// How many board memberships this document asked for, off `$boardItems`.
    ///
    /// Read off the request rather than from the source's own constant, so a page of
    /// memberships here is the page that was really asked for.
    board_items: usize,
    /// A cursor this board answers every membership page with, when a case has wedged it.
    ///
    /// `None` is the ordinary board. See [`membership_page`].
    stuck_cursor: Option<&'static str>,
}

impl Item {
    fn issue(id: &str, title: &str) -> Self {
        Self {
            item_id: format!("PVTI_{id}"),
            content_id: id.to_owned(),
            typename: "Issue",
            title: title.to_owned(),
            body: None,
            state: "OPEN",
            state_reason: None,
            number: 1043,
            parent: None,
            sub_issues: 0,
            repository: Some("acme/work".to_owned()),
            labels: vec![],
            status: None,
            origin: None,
            other_boards: Vec::new(),
            on_this_board: true,
            board_entry_id: "PVT_board",
            fields_of_a_namesake_ahead: false,
            listed: true,
            origin_value: true,
            updated_at: None,
            path_labels: BTreeMap::new(),
            texts: BTreeMap::new(),
        }
    }
    /// Hold `text` in the text field `field_id`. See [`Item::texts`].
    fn holding_text(mut self, field_id: &str, text: &str) -> Self {
        self.texts.insert(field_id.to_owned(), text.to_owned());
        self
    }
    fn draft(id: &str, title: &str) -> Self {
        Self {
            typename: "DraftIssue",
            repository: None,
            ..Self::issue(id, title)
        }
    }
    fn pull_request(id: &str) -> Self {
        Self {
            typename: "PullRequest",
            ..Self::issue(id, "a change")
        }
    }
    fn body(mut self, body: &str) -> Self {
        self.body = Some(body.to_owned());
        self
    }
    fn status(mut self, status: &str) -> Self {
        self.status = Some(status.to_owned());
        self
    }
    fn parent(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_owned());
        self
    }
    /// Stamp this issue as last updated at `at`, as GitHub would after a comment on it was
    /// added or edited then.
    fn updated(mut self, at: &str) -> Self {
        self.updated_at = Some(at.to_owned());
        self
    }
    fn number(mut self, number: u64) -> Self {
        self.number = number;
        self
    }
    fn sub_issues(mut self, total: u64) -> Self {
        self.sub_issues = total;
        self
    }
    fn in_repository(mut self, repository: &str) -> Self {
        self.repository = Some(repository.to_owned());
        self
    }
    fn closed(mut self, reason: Option<&str>) -> Self {
        self.state = "CLOSED";
        self.state_reason = reason.map(str::to_owned);
        self
    }
    fn labelled(mut self, labels: &[(&'static str, &'static str)]) -> Self {
        self.labels = labels.to_vec();
        self
    }
    /// Put this issue on `boards` as well, ahead of its entry for the board under test.
    ///
    /// Enough of them and the entry for this board sits past the page `board_issue!`
    /// carries, which is the case the recovery read is for. See [`Item::other_boards`].
    fn also_on(mut self, boards: &[u64]) -> Self {
        self.other_boards = boards.to_vec();
        self
    }
    /// Take this issue off the board under test, leaving it on `boards` alone.
    fn only_on(mut self, boards: &[u64]) -> Self {
        self.other_boards = boards.to_vec();
        self.on_this_board = false;
        self
    }
    /// Make this issue's entry for the board under test name `id` as the board's node id.
    fn board_entry_names(mut self, id: &'static str) -> Self {
        self.board_entry_id = id;
        self
    }
    /// Carry another owner's board numbered as this one is, with fields of its own, ahead of
    /// this board in the read of this issue by its own id. See
    /// [`Item::fields_of_a_namesake_ahead`].
    fn namesake_board_ahead(mut self) -> Self {
        self.fields_of_a_namesake_ahead = true;
        self
    }
    /// Leave this item out of every listing of the board while every read of it by id still
    /// places it on the board. See [`Item::listed`].
    fn unlisted(mut self) -> Self {
        self.listed = false;
        self
    }
    /// Put `origin` in this item's origin field — and nowhere else, which is how the release
    /// before this one wrote a copy's origin.
    fn carrying(mut self, origin: &str) -> Self {
        self.origin = Some(origin.to_owned());
        self
    }
    /// Give this item no value of the origin field. See [`Item::origin_value`].
    fn holding_no_origin_value(mut self) -> Self {
        self.origin_value = false;
        self
    }
    /// Answer `path` with a label set of its own. See [`Item::path_labels`].
    fn labels_on(mut self, path: &'static str, labels: &[(&'static str, &'static str)]) -> Self {
        self.path_labels.insert(path, labels.to_vec());
        self
    }
    /// The labels this board answers `path` with, which is this item's own set unless a
    /// case has deliberately made that one path disagree.
    fn labels_seen_on(&self, path: &str) -> &[(&'static str, &'static str)] {
        self.path_labels
            .get(path)
            .map_or(self.labels.as_slice(), Vec::as_slice)
    }
    /// One label connection, as every one of them comes back.
    fn label_nodes(&self, path: &str) -> Value {
        json!({"nodes":self.labels_seen_on(path).iter()
                   .map(|(id,name)| json!({"id":id,"name":name,"color":null}))
                   .collect::<Vec<_>>(),
               "pageInfo":{"hasNextPage":false}})
    }

    fn field_values(&self, options: &Value) -> Value {
        let mut nodes = self
            .texts
            .iter()
            .map(|(id, text)| {
                json!({"text":text,"field":{"id":id,"name":TEXT_FIELD_NAMES
                    .iter()
                    .find(|(known, _)| known == id)
                    .map_or("unnamed", |(_, name)| *name)}})
            })
            .collect::<Vec<_>>();
        if let Some(status) = &self.status {
            nodes.push(
                json!({"name":status,"field":{"id":"FIELD_status","name":"Status","options":options}}),
            );
        }
        if self.origin_value || self.origin.is_some() {
            nodes.push(
                json!({"text":self.origin.clone().unwrap_or_default(),"field":{"id":"FIELD_origin","name":"onetaskgraph.origin"}}),
            );
        }
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }

    /// Every board this item sits on, in the order `Issue.projectItems` reports them.
    ///
    /// The entry for the board this suite configures — project number 7 — carries the same
    /// board item id and the same field values a `ProjectV2.items` read gives it, reached
    /// from the issue instead of from the board. Anything in [`Item::other_boards`] sits
    /// ahead of it, which is what pushes it past a page.
    fn memberships(&self, options: &Value) -> Vec<Value> {
        let mut nodes = self
            .other_boards
            .iter()
            .map(|number| {
                json!({"id":format!("PVTI_{number}_{}", self.content_id),
                       "project":{"id":format!("PVT_{number}"),"number":number},
                       "fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}}})
            })
            .collect::<Vec<_>>();
        if self.on_this_board {
            nodes.push(
                json!({"id":self.item_id,"project":{"id":self.board_entry_id,"number":7},
                       "fieldValues":self.field_values(options)}),
            );
        }
        nodes
    }

    /// The board half of this item, as one page of `Issue.projectItems` carries it.
    fn project_items(&self, options: &Value, asked: Asked) -> Value {
        membership_page(&self.memberships(options), 0, asked.board_items, asked)
    }

    /// This item as a search, a node read or a sub-issue read returns it.
    fn as_issue(&self, options: &Value, asked: Asked) -> Value {
        let mut issue = self.content(asked);
        issue["projectItems"] = self.project_items(options, asked);
        issue
    }

    fn content(&self, asked: Asked) -> Value {
        match self.typename {
            "PullRequest" => json!({"__typename":"PullRequest","id":self.content_id}),
            "DraftIssue" => json!({"__typename":"DraftIssue","id":self.content_id,
                "title":self.title,"body":self.body,"createdAt":null,"updatedAt":null}),
            _ => json!({"__typename":"Issue","id":self.content_id,"number":self.number,
                "title":self.title,
                "body":self.body.clone().unwrap_or_default(),
                "url":format!("https://github.example/{}", self.content_id),
                "createdAt":null,"updatedAt":self.updated_at,"state":self.state,
                "stateReason":self.state_reason,
                "repository":self.repository.as_ref().map(|r| json!({"nameWithOwner":r})),
                "parent":self.parent.as_ref().map(|id| json!({"id":id})),
                "subIssuesSummary":{"total":self.sub_issues},
                "labels":self.label_nodes(asked.path)}),
        }
    }
}

/// One page of a membership connection, in the order and shape GitHub answers it.
///
/// Shared by the page a read of an issue carries and by the pages the recovery read walks,
/// because the source has to be able to resume the second from the first's own cursor: an
/// offset here is a row of the same list either way.
///
/// [`Asked::stuck_cursor`] is what makes a walk misbehave on purpose — the page reports
/// another page and answers with that same cursor however far the walk has got, which is
/// what a source with no pagination guard would spin on for ever.
fn membership_page(all: &[Value], offset: usize, first: usize, asked: Asked) -> Value {
    let end = (offset + first).min(all.len());
    let nodes = &all[offset.min(end)..end];
    match asked.stuck_cursor {
        Some(cursor) => json!({"nodes":nodes,"pageInfo":{"hasNextPage":true,
                                                         "endCursor":cursor}}),
        None => json!({"nodes":nodes,
                       "pageInfo":{"hasNextPage":end < all.len(),"endCursor":end.to_string()}}),
    }
}

/// The login this board signs a comment added through it with.
///
/// The account the token belongs to, which is who GitHub records as the author of every
/// comment — so a comment that comes back naming anybody else was not signed by this board.
const COMMENTER: &str = "onetaskgraph-bot";

/// One issue comment as the fixture holds it, in the order it was written.
#[derive(Clone, Debug)]
struct HeldComment {
    id: String,
    issue: String,
    /// `None` is an account GitHub no longer has, which it answers `author: null` for.
    author: Option<String>,
    body: String,
    created_at: String,
    updated_at: String,
}

impl HeldComment {
    /// This comment as every comment document selects it.
    fn as_node(&self) -> Value {
        json!({"id":self.id,"author":self.author.as_ref().map(|login| json!({"login":login})),
               "createdAt":self.created_at,"updatedAt":self.updated_at,"body":self.body,
               "url":format!("https://github.example/{}#{}", self.issue, self.id)})
    }
}

/// Everything the fixture remembers between requests.
struct State {
    items: Vec<Item>,
    /// Every comment on every issue this board holds, oldest first.
    comments: Vec<HeldComment>,
    /// How many comment timestamps this board has handed out, which is what makes each one
    /// later than the last — and a comment's id, so no two are ever alike.
    comment_ticks: u64,
    /// Issues created but not yet added to the board.
    pending: Vec<Item>,
    options: Vec<(&'static str, &'static str)>,
    /// Complete option records used by the guarded Status-option fixture cases.
    guarded_status_options: Option<Vec<Value>>,
    /// Whether the first post-update read changes existing option metadata.
    drift_guarded_status_description: bool,
    /// Whether `createIssue` answers with a `number` that is not an unsigned integer.
    ///
    /// The other half of the tolerated-absence rule: a member that came back *missing* is
    /// not worth failing a landed write over, and one that came back unreadable is a
    /// response this source cannot read at all. Only a fixture reaches it, because GitHub
    /// answers with an `Int!`.
    creation_number_is_unreadable: bool,
    /// Whether `createIssue` answers with the new issue's `number`.
    ///
    /// GitHub declares `Issue.number` non-null and always answers with one, which is the
    /// default here. A board that leaves it out models the response this source is
    /// deliberately lenient about: a landed write is not worth failing over a member that
    /// came back missing, so the item reports no handle until a read of the board catches
    /// up. Nothing else can produce that state, and it is not one a board is ever *seen*
    /// in — which is exactly why it needs a fixture to reach it.
    creation_reports_number: bool,
    /// Which identifier the next guarded snapshot returns blank, for boundary validation.
    blank_status_snapshot_id: Option<&'static str>,
    origin_field: bool,
    status_field: bool,
    /// The board's own text fields beyond the origin's — and a same-named field of another
    /// type a case puts there — as the board's field list answers each.
    extra_fields: Vec<Value>,
    /// One alias of the next batched field write this board answers with this value instead
    /// of what it did — a response the source must refuse to read as landed.
    answers_alias_with: Option<(&'static str, Value)>,
    blocked_by: BTreeMap<String, Vec<String>>,
    /// Mutations this board answers with a GraphQL error rather than performing. GitHub
    /// fails one call of the several a write is, and what the source does about the calls
    /// that already landed is only readable if one of them can be made to fail.
    refuses: BTreeSet<String>,
    /// Mutations this board begins refusing after this many matching calls have landed.
    refuse_after: BTreeMap<String, usize>,
    /// How many of the most recently filed items this board's own reads do not show yet.
    /// GitHub's `projectV2.items` is eventually consistent, so an item a run just created
    /// is answered out of the source's own record of what it created until the board
    /// catches up — and what that record holds is only observable while it is behind.
    lagging_reads: usize,
    /// Whether this board's issue search orders one answer differently at every page size,
    /// as GitHub's does: the credentialed lane watched a search answer three issues in one
    /// order at a page of twenty and in another when asked a row at a time.
    search_order_varies_with_first: bool,
    /// How many items this board's own `ProjectV2.items` connection lists, once that
    /// connection has been left behind — `None` while it lists everything this board holds.
    ///
    /// GitHub's lag, which that connection has and no other view of a board does; the crate
    /// documentation at `GitHubProjectsSource::board` is where it is written down. So this
    /// withholds from that one connection and from nothing else.
    ///
    /// It is deliberately permanent rather than timed: a lag that expires would let a wait
    /// outlast it, and a wait that can outlast the defect proves nothing about the source.
    items_connection_behind_from: Option<usize>,
    /// A cursor this board answers every membership page with, instead of one that
    /// advances. See [`Asked::stuck_cursor`].
    stuck_membership_cursor: Option<&'static str>,
    seen: Vec<Value>,
    /// Every GraphQL document this board received, in order.
    ///
    /// The operation counts beside it say how *many* requests a read made; this says what
    /// each one asked for, which is the only place a test can see that a read scoped to one
    /// project never asked the board for its items.
    documents: Vec<String>,
    bindings: Vec<(String, Value)>,
    /// The search string of every board-scoped search this board answered, in order.
    searches: Vec<String>,
    /// The field filter of every origin lookup this board answered, in order.
    origin_filters: Vec<String>,
    /// Every non-null variable a request declared and left unbound, as `$name: Type`, in
    /// arrival order. GitHub refuses such a request before it runs, and so does this board;
    /// this is the record a test reads to say which request it was.
    unbound_variables: Vec<String>,
    /// Items this board's indexes — its issue search and its item connection's field filter —
    /// still answer as they were before a write, by content id.
    ///
    /// GitHub's indexes lag a write, so a search can still name an item under what it held a
    /// moment ago, and answer with that. A read of the item by its own id is current.
    indexed_as: BTreeMap<String, Item>,
    /// The issue named by every read of an issue's comments this board answered, in order.
    comment_reads: Vec<String>,
    /// The repository's own labels, as `(name, node id)`.
    ///
    /// A repository label is created and deleted over REST and attached over GraphQL, so
    /// the two halves of this fixture share one registry: a name the REST side created is
    /// the name the GraphQL side attaches, exactly as it is on GitHub.
    labels: Vec<(String, String)>,
    next: usize,
    /// GitHub's two rate limiters, as far as a board fixture can spell them.
    limits: Limits,
}

/// One canned HTTP refusal, in the exact shape GitHub answers a rate limit with.
#[derive(Clone)]
struct Refusal {
    status: &'static str,
    headers: String,
    body: String,
}

impl Refusal {
    /// A secondary rate limit under a forbidden status, which is how GitHub answers it far
    /// more often than with too-many-requests.
    fn secondary_forbidden() -> Self {
        Self {
            status: "403 Forbidden",
            headers: String::new(),
            body: json!({"message":"You have exceeded a secondary rate limit and have been \
                                   temporarily blocked from content creation. Please retry \
                                   your request again later.",
                         "documentation_url":"https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api"})
                .to_string(),
        }
    }
    /// A transient unavailability, which is what GitHub answers when it is briefly unwell.
    ///
    /// Deliberately not a rate limit: `Limiter::classify` recognises none of it, so the
    /// source reports it rather than waiting it out, and one scripted entry is therefore one
    /// failed request rather than the first of a retried schedule.
    fn unavailable() -> Self {
        Self {
            status: "503 Service Unavailable",
            headers: String::new(),
            body: json!({"message": "unavailable"}).to_string(),
        }
    }
    /// The same refusal, asking for a wait of `seconds`.
    fn after(mut self, seconds: u64) -> Self {
        self.headers = format!("retry-after: {seconds}\r\n");
        self
    }
}

/// What this board refuses for going too fast, and what it counts while it does.
#[derive(Default)]
struct Limits {
    /// Canned refusals to answer the next requests with, oldest first.
    scripted: Vec<Refusal>,
    /// Canned refusals to answer the next requests *carrying one operation* with.
    scripted_for: BTreeMap<String, Vec<Refusal>>,
    /// Shortest interval this board accepts between two mutations. Anything faster is
    /// refused the way GitHub refuses a secondary rate limit.
    min_mutation_interval: Option<Duration>,
    /// Refuse every mutation with a secondary rate limit, however slowly it arrives.
    refuse_every_mutation: bool,
    /// Operations this board answers normally, with the budget reported as spent — which
    /// is what GitHub sends on the last request a budget allows.
    spends_the_budget: BTreeSet<String>,
    /// How long this board sits on a mutation's answer before sending it, which is how a
    /// fixture on loopback stands in for the transit a real request spends between leaving
    /// the source and arriving here.
    mutation_response_delay: Option<Duration>,
    /// When the last mutation arrived, for the interval above.
    last_mutation: Option<Instant>,
    /// How many mutations this board refused for arriving too fast.
    too_fast: u32,
    /// What the *account* has spent of [`FIXTURE_BUDGET_LIMIT`], reported in every answer's
    /// own `x-ratelimit-used` and `x-ratelimit-remaining` the way GitHub reports it.
    budget_used: u64,
    /// What something *else* spends against the same budget between two of this session's
    /// requests.
    ///
    /// The account these figures describe is shared and rate-limited, so its remaining
    /// allowance really does fall by more than one session's own calls account for. Setting
    /// this is what lets a test prove the session's reported spend is unmoved by the
    /// difference — which is the whole distinction between measuring a session and
    /// differencing a counter somebody else is also spending.
    other_traffic_per_request: u64,
    /// When each request arrived, which operation it carried, and whether that operation
    /// creates content.
    arrivals: Vec<(Instant, String, bool)>,
}

impl Limits {
    /// The refusal this request earns, or `None` to let the board answer it.
    fn refusal(&mut self, operation: &str, mutation: bool) -> Option<Refusal> {
        let now = Instant::now();
        self.arrivals.push((now, operation.to_owned(), mutation));
        if let Some(scripted) = self
            .scripted_for
            .get_mut(operation)
            .filter(|scripted| !scripted.is_empty())
        {
            return Some(scripted.remove(0));
        }
        if !self.scripted.is_empty() {
            return Some(self.scripted.remove(0));
        }
        if !mutation {
            return None;
        }
        if self.refuse_every_mutation {
            return Some(Refusal::secondary_forbidden());
        }
        let too_fast = self.min_mutation_interval.is_some_and(|interval| {
            self.last_mutation
                .is_some_and(|last| now.duration_since(last) < interval)
        });
        self.last_mutation = Some(now);
        if too_fast {
            self.too_fast += 1;
            return Some(Refusal::secondary_forbidden());
        }
        None
    }
}

impl State {
    /// `item` as this board's indexes answer it: as it was when its index was last current.
    fn indexed(&self, item: &Item) -> Item {
        self.indexed_as
            .get(&item.content_id)
            .cloned()
            .unwrap_or_else(|| item.clone())
    }
    fn options(&self) -> Value {
        Value::Array(
            self.options
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect(),
        )
    }
    fn fields(&self) -> Value {
        let mut nodes = Vec::new();
        if self.status_field {
            nodes.push(
                json!({"__typename":"ProjectV2SingleSelectField","id":"FIELD_status",
                              "name":"Status","options":self.options()}),
            );
        }
        if self.origin_field {
            nodes.push(
                json!({"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin","dataType":"TEXT"}),
            );
        }
        nodes.extend(self.extra_fields.iter().cloned());
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }
    /// The next moment this board stamps a comment with, a second after the one before.
    fn tick(&mut self) -> String {
        self.comment_ticks += 1;
        format!(
            "2026-09-01T00:{:02}:{:02}Z",
            self.comment_ticks / 60,
            self.comment_ticks % 60
        )
    }
    /// Put a comment on `issue`, as `author` wrote it.
    fn comment(&mut self, issue: &str, author: Option<&str>, body: &str) -> HeldComment {
        let at = self.tick();
        let held = HeldComment {
            id: format!("IC_{}", self.comment_ticks),
            issue: issue.to_owned(),
            author: author.map(str::to_owned),
            body: body.to_owned(),
            created_at: at.clone(),
            updated_at: at,
        };
        self.comments.push(held.clone());
        held
    }
    fn find(&mut self, content_id: &Value) -> &mut Item {
        let wanted = content_id.as_str().expect("a content id");
        self.items
            .iter_mut()
            .find(|item| item.content_id == wanted)
            .expect("the fixture holds the item being written")
    }
}

/// A running board fixture.
struct Fixture {
    endpoint: String,
    state: Arc<Mutex<State>>,
}

impl Fixture {
    /// The mutation inputs the source sent, in order, as `[operation, input]` pairs.
    fn seen(&self) -> Vec<Value> {
        self.state.lock().unwrap().seen.clone()
    }
    /// Put a comment on `issue` the way somebody else writing on GitHub would, answering its
    /// id.
    fn commented(&self, issue: &str, author: Option<&str>, body: &str) -> String {
        self.state.lock().unwrap().comment(issue, author, body).id
    }
    /// The comments this board holds on `issue`, oldest first.
    fn comments_on(&self, issue: &str) -> Vec<HeldComment> {
        self.state
            .lock()
            .unwrap()
            .comments
            .iter()
            .filter(|held| held.issue == issue)
            .cloned()
            .collect()
    }
    /// Answer `alias` of the next batched field write that sends it with `value`.
    fn answer_alias_with(&self, alias: &'static str, value: Value) {
        self.state.lock().unwrap().answers_alias_with = Some((alias, value));
    }
    /// Put a field on this board beside the ones it already has, as its field list answers it.
    fn with_field(self, field: Value) -> Self {
        self.state.lock().unwrap().extra_fields.push(field);
        self
    }
    fn item(&self, content_id: &str) -> Item {
        self.state
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item.content_id == content_id)
            .expect("the fixture holds that item")
            .clone()
    }
    /// Whether this board still holds an item, which is a different question from
    /// `item` — one asserts on what it carries, this on whether it is there at all.
    fn holds(&self, content_id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .items
            .iter()
            .any(|item| item.content_id == content_id)
    }
    /// Fail this mutation from here on, the way GitHub fails one call part way through a
    /// write: everything before it has landed, and nothing after it runs.
    fn refuse(&self, operation: &str) {
        self.state
            .lock()
            .unwrap()
            .refuses
            .insert(operation.to_owned());
    }
    /// Let `successful_calls` matching mutations land, then fail the next one.
    fn refuse_after(&self, operation: &str, successful_calls: usize) {
        self.state
            .lock()
            .unwrap()
            .refuse_after
            .insert(operation.to_owned(), successful_calls);
    }
    /// Answer the next requests with these canned HTTP refusals, oldest first.
    fn script(&self, refusals: Vec<Refusal>) {
        self.state.lock().unwrap().limits.scripted = refusals;
    }
    /// Answer the next requests carrying `operation` with these canned refusals, oldest
    /// first. A write is several calls, so refusing one of them by name is the only way to
    /// say *which* the limiter caught.
    fn script_for(&self, operation: &str, refusals: Vec<Refusal>) {
        self.state
            .lock()
            .unwrap()
            .limits
            .scripted_for
            .insert(operation.to_owned(), refusals);
    }
    /// Refuse any mutation arriving less than `interval` after the one before it, the way
    /// GitHub's secondary limiter refuses a burst of content creation.
    fn rate_limit_mutations(&self, interval: Duration) {
        self.state.lock().unwrap().limits.min_mutation_interval = Some(interval);
    }
    /// Sit on every mutation's answer for `delay` before sending it.
    ///
    /// A real request costs time in both directions and this fixture answers instantly, so
    /// nothing here would otherwise separate a source that spaces its departures from one
    /// that spaces from the moment the last request finished. Holding the answer makes that
    /// difference measurable in the arrival gaps this board records.
    fn delay_mutation_responses(&self, delay: Duration) {
        self.state.lock().unwrap().limits.mutation_response_delay = Some(delay);
    }
    /// Refuse every mutation with a secondary rate limit, however slowly it arrives.
    fn refuse_every_mutation(&self) {
        self.state.lock().unwrap().limits.refuse_every_mutation = true;
    }
    /// Answer `operation` normally, reporting the budget spent — which is what GitHub
    /// sends on the last request a budget allows, not only on the ones it then refuses.
    fn spend_the_budget_on(&self, operation: &str) {
        self.state
            .lock()
            .unwrap()
            .limits
            .spends_the_budget
            .insert(operation.to_owned());
    }
    /// How many mutations this board refused for arriving faster than it allows.
    fn too_fast(&self) -> u32 {
        self.state.lock().unwrap().limits.too_fast
    }
    /// Report the account's allowance falling by `extra` more per request than this
    /// session's own calls account for, the way a shared account falls while other work
    /// draws on it.
    fn other_traffic(&self, extra: u64) {
        self.state.lock().unwrap().limits.other_traffic_per_request = extra;
    }
    /// What this board last reported the account had spent.
    fn budget_used(&self) -> u64 {
        self.state.lock().unwrap().limits.budget_used
    }
    /// Answer every page of every membership connection with `cursor`, saying there is
    /// more.
    ///
    /// Nothing GitHub does: it is how a source's pagination guard is watched refusing. An
    /// empty cursor and one that does not advance are the two ways a connection can offer
    /// no progress, and both arrive here as this one knob.
    fn wedge_membership_cursor(&self, cursor: &'static str) {
        self.state.lock().unwrap().stuck_membership_cursor = Some(cursor);
    }
    /// How many times this board was asked for one issue's memberships on their own.
    ///
    /// The recovery read and nothing else, counted from the documents this board really
    /// received — which is what tells a miss that cost a request from one that cost none.
    fn membership_walks(&self) -> usize {
        self.documents()
            .iter()
            .filter(|document| {
                document.contains("on Issue{projectItems(first:$first,after:$after)")
            })
            .count()
    }
    /// Every GraphQL document this board received, in order.
    fn documents(&self) -> Vec<String> {
        self.state.lock().unwrap().documents.clone()
    }
    /// The search string of every board-scoped search this board answered, in order.
    fn searches(&self) -> Vec<String> {
        self.state.lock().unwrap().searches.clone()
    }
    /// The issue named by every read of an issue's comments this board answered, in order.
    fn comment_reads(&self) -> Vec<String> {
        self.state.lock().unwrap().comment_reads.clone()
    }
    /// Put a comment on `issue` written at `created_at` and last edited at `updated_at`, the
    /// way somebody writing on GitHub earlier would have left it.
    fn commented_at(&self, issue: &str, created_at: &str, updated_at: &str) -> String {
        let mut state = self.state.lock().unwrap();
        let held = state.comment(issue, Some("someone"), "a word\n");
        let held = state
            .comments
            .iter_mut()
            .find(|comment| comment.id == held.id)
            .expect("just added");
        held.created_at = created_at.to_owned();
        held.updated_at = updated_at.to_owned();
        held.id.clone()
    }
    /// Which of the documents this board received selected its own item connection.
    ///
    /// The one read whose cost is the whole board, named by the selection that makes it
    /// so rather than by the constant that holds it: a read that stopped asking for
    /// `ProjectV2.items` and started asking for it again under another name would still be
    /// caught here.
    fn board_item_reads(&self) -> Vec<String> {
        self.documents()
            .into_iter()
            .filter(|document| {
                document.contains("projectV2(number:$number)")
                    && document.contains("items(first:$first,after:$after)")
            })
            .collect()
    }
    /// How many requests carried `operation`, refused ones included.
    fn requests(&self, operation: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .limits
            .arrivals
            .iter()
            .filter(|(_, seen, _)| seen == operation)
            .count()
    }
    /// The operation every request this board received carried, in order, refused ones
    /// included.
    fn operations(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .limits
            .arrivals
            .iter()
            .map(|(_, seen, _)| seen.clone())
            .collect()
    }
    /// The gaps between consecutive arrivals of any content-creating mutation.
    fn mutation_gaps(&self) -> Vec<Duration> {
        let state = self.state.lock().unwrap();
        let times = state
            .limits
            .arrivals
            .iter()
            .filter(|(_, _, mutation)| *mutation)
            .map(|(at, _, _)| *at)
            .collect::<Vec<_>>();
        times.windows(2).map(|pair| pair[1] - pair[0]).collect()
    }
    /// Retitle an item without going through the source, the way anything else that
    /// touches this board does — another person, another tool, another process.
    fn retitled_by_something_else(&self, content_id: &str, title: &str) {
        let mut state = self.state.lock().unwrap();
        let item = state
            .items
            .iter_mut()
            .find(|item| item.content_id == content_id)
            .expect("this board holds the item being retitled");
        item.title = title.to_owned();
    }
    /// Hold this board's reads `count` items behind what it really holds, the way GitHub's
    /// eventually-consistent board read holds behind a mutation that has already landed.
    fn read_behind(&self, count: usize) {
        self.state.lock().unwrap().lagging_reads = count;
    }
    /// Order this board's issue search by the page size it is asked at, the way GitHub's
    /// relevance ordering breaks its ties differently per page size; see
    /// [`State::search_order_varies_with_first`].
    fn order_search_by_page_size(&self) {
        self.state.lock().unwrap().search_order_varies_with_first = true;
    }

    /// Leave this board's own `ProjectV2.items` connection behind everything filed from now
    /// on, the way GitHub's is, and leave every other view of this board current.
    ///
    /// See [`State::items_connection_behind_from`] for the measurement this models. It
    /// reaches the source's own whole-board document alone: the lane's artifact lookup in
    /// `journey::artifact_item_ids` walks that connection too, and on GitHub what it looks
    /// for is residue from runs long since projected — a board that hid a run's own
    /// artifacts from that run's own cleanup would model a board nothing can ever sweep,
    /// which is not what was observed and would fail the journey somewhere it is not about.
    /// File `item` on this board the way another process writing to it would, after
    /// anything this board already holds.
    fn filed_by_something_else(&self, item: Item) {
        self.state.lock().unwrap().items.push(item);
    }
    /// Hold this board's indexes on `content_id` as it is now, whatever is written to it
    /// later. See [`State::indexed_as`].
    fn indexes_behind(&self, content_id: &str) {
        let mut state = self.state.lock().unwrap();
        let held = state
            .items
            .iter()
            .find(|item| item.content_id == content_id)
            .expect("this board holds the item")
            .clone();
        state.indexed_as.insert(content_id.to_owned(), held);
    }
    /// Let this board's indexes answer `content_id` as it now is, as GitHub's do once they
    /// catch up with a write.
    fn index_catches_up(&self, content_id: &str) {
        self.state.lock().unwrap().indexed_as.remove(content_id);
    }
    /// The field filter of every origin lookup this board answered, in order.
    fn origin_filters(&self) -> Vec<String> {
        self.state.lock().unwrap().origin_filters.clone()
    }
    /// Every non-null variable a request this board refused had left unbound.
    fn unbound_variables(&self) -> Vec<String> {
        self.state.lock().unwrap().unbound_variables.clone()
    }
    fn items_connection_falls_behind(&self) {
        let mut state = self.state.lock().unwrap();
        state.items_connection_behind_from = Some(state.items.len());
    }
    /// Give this board's `Status` field one more option, the way a person adding a column on
    /// GitHub does. The shipped board has no `Queued` option, which is what lets one case
    /// prove a status needing it is refused and another prove it lands once it is there.
    fn offer_option(&self, id: &'static str, name: &'static str) {
        self.state.lock().unwrap().options.push((id, name));
    }

    /// Give the guarded operation a Status field missing the configured `Queued` option.
    fn status_options_missing_queued(&self) {
        self.state.lock().unwrap().guarded_status_options = Some(vec![
            json!({"id":"OPT_backlog","name":"Backlog","color":"GRAY","description":"later"}),
            json!({"id":"OPT_todo","name":"Todo","color":"BLUE","description":"ready"}),
            json!({"id":"OPT_doing","name":"In Progress","color":"YELLOW","description":"active"}),
            json!({"id":"OPT_done","name":"Done","color":"GREEN","description":""}),
            json!({"id":"OPT_cancelled","name":"Cancelled","color":"RED","description":""}),
            json!({"id":"OPT_shipped","name":"Shipped","color":"PURPLE","description":"custom"}),
        ]);
    }

    /// Give the guarded operation a Status field missing both shipped terminal options,
    /// which a terminal write needs before it will close an issue.
    fn status_options_missing_terminals(&self) {
        self.state.lock().unwrap().guarded_status_options = Some(vec![
            json!({"id":"OPT_backlog","name":"Backlog","color":"GRAY","description":"later"}),
            json!({"id":"OPT_todo","name":"Todo","color":"BLUE","description":"ready"}),
            json!({"id":"OPT_queued","name":"Queued","color":"YELLOW","description":""}),
            json!({"id":"OPT_doing","name":"In Progress","color":"YELLOW","description":"active"}),
        ]);
    }

    /// Make the verification snapshot disagree on existing option metadata.
    fn drift_guarded_status_description(&self) {
        self.state.lock().unwrap().drift_guarded_status_description = true;
    }
    /// Answer every `createIssue` from here on without the issue's `number`.
    fn creation_reports_no_number(&self) {
        self.state.lock().unwrap().creation_reports_number = false;
    }

    /// Answer every `createIssue` from here on with a `number` that is not an integer.
    fn creation_reports_an_unreadable_number(&self) {
        self.state.lock().unwrap().creation_number_is_unreadable = true;
    }

    fn blank_status_snapshot_id(&self, target: &'static str) {
        self.state.lock().unwrap().blank_status_snapshot_id = Some(target);
    }
}

/// The id and name of every text field beyond the origin's a case may put on this board.
const TEXT_FIELD_NAMES: [(&str, &str); 10] = [
    ("FIELD_host", "Host"),
    ("FIELD_team", "Team"),
    ("FIELD_b0", "B0"),
    ("FIELD_b1", "B1"),
    ("FIELD_b2", "B2"),
    ("FIELD_b3", "B3"),
    ("FIELD_b4", "B4"),
    ("FIELD_b5", "B5"),
    ("FIELD_b6", "B6"),
    ("FIELD_b7", "B7"),
];

fn board(items: Vec<Item>) -> Fixture {
    board_with(items, true, true)
}

fn board_with(items: Vec<Item>, status_field: bool, origin_field: bool) -> Fixture {
    let state = Arc::new(Mutex::new(State {
        items,
        comments: Vec::new(),
        comment_ticks: 0,
        pending: Vec::new(),
        options: vec![
            ("OPT_backlog", "Backlog"),
            ("OPT_todo", "Todo"),
            ("OPT_doing", "In Progress"),
            ("OPT_done", "Done"),
            ("OPT_cancelled", "Cancelled"),
            ("OPT_shipped", "Shipped"),
        ],
        guarded_status_options: None,
        drift_guarded_status_description: false,
        creation_number_is_unreadable: false,
        creation_reports_number: true,
        blank_status_snapshot_id: None,
        origin_field,
        status_field,
        extra_fields: Vec::new(),
        answers_alias_with: None,
        blocked_by: BTreeMap::new(),
        refuses: BTreeSet::new(),
        refuse_after: BTreeMap::new(),
        lagging_reads: 0,
        search_order_varies_with_first: false,
        items_connection_behind_from: None,
        stuck_membership_cursor: None,
        seen: Vec::new(),
        documents: Vec::new(),
        bindings: Vec::new(),
        searches: Vec::new(),
        origin_filters: Vec::new(),
        unbound_variables: Vec::new(),
        indexed_as: BTreeMap::new(),
        comment_reads: Vec::new(),
        labels: Vec::new(),
        next: 0,
        limits: Limits::default(),
    }));
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let served = Arc::clone(&state);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("fixture connection");
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("a GraphQL document");
            graphql_parser::parse_query::<String>(query).expect("a valid GraphQL document");
            let variables = &request["variables"];
            served.lock().unwrap().documents.push(query.to_owned());
            // The limiter answers before the board does, exactly as GitHub's does: a
            // refused request never reaches the board and changes nothing on it.
            let limited = served
                .lock()
                .unwrap()
                .limits
                .refusal(operation_name(query), is_mutation(query));
            let spent = served
                .lock()
                .unwrap()
                .limits
                .spends_the_budget
                .contains(operation_name(query));
            // What this answer says about the account's budget. GitHub carries these on
            // every response, so the source's accounting has real headers to read rather
            // than a shape only the live lane could ever fill in — and `used` climbing
            // faster than one per request is how a shared account behaves.
            let used = {
                let mut state = served.lock().unwrap();
                state.limits.budget_used += 1 + state.limits.other_traffic_per_request;
                state.limits.budget_used
            };
            let remaining = if spent {
                0
            } else {
                FIXTURE_BUDGET_LIMIT.saturating_sub(used)
            };
            let (status, headers, body) = match limited {
                Some(refusal) => (refusal.status, refusal.headers, refusal.body),
                None => (
                    "200 OK",
                    format!(
                        "x-ratelimit-limit: {FIXTURE_BUDGET_LIMIT}\r\n\
                         x-ratelimit-used: {used}\r\n\
                         x-ratelimit-remaining: {remaining}\r\n\
                         x-ratelimit-resource: graphql\r\n"
                    ),
                    match unbound_refusal(&served, query, variables)
                        .or_else(|| refused(&served, query, variables))
                    {
                        Some(message) => json!({"errors":[{"message":message}]}).to_string(),
                        None => json!({ "data": answer(&served, query, variables) }).to_string(),
                    },
                ),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            // The stand-in for transit, held after the arrival above is recorded and before
            // the answer goes back — which is where a real round trip spends its time.
            let delay = served.lock().unwrap().limits.mutation_response_delay;
            if let (Some(delay), true) = (delay, is_mutation(query)) {
                thread::sleep(delay);
            }
            stream.write_all(response.as_bytes()).expect("a response");
        }
    });
    Fixture { endpoint, state }
}

/// GitHub's refusal of a request that leaves a non-null variable unbound, recorded, or
/// `None` when every such variable is given.
///
/// GitHub validates the variables before it runs anything, so a request refused here reaches
/// no board state — exactly what the credentialed lane saw of an origin lookup's cost probe
/// sent without its `$filter`, which no board that answered it anyway could have shown.
fn unbound_refusal(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Option<String> {
    let unbound = board::unbound_required_variables(query, variables);
    let (name, ty) = unbound.first()?.clone();
    state
        .lock()
        .unwrap()
        .unbound_variables
        .extend(unbound.iter().map(|(name, ty)| format!("${name}: {ty}")));
    Some(format!(
        "Variable ${name} of type {ty} was provided invalid value"
    ))
}

/// The GraphQL error a refused mutation answers with, or `None` to perform it.
///
/// A refused call is still recorded as seen and still changes nothing: that is what GitHub
/// failing one mutation of a write looks like from here.
fn refused(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Option<String> {
    // A connection asked for more rows than GitHub's own maximum is refused before it runs,
    // which is what `max_page_size` claims to describe and what the journey's page-size
    // probe asks this board to prove one row either side of.
    if query.contains("items(first:$first){nodes{id}}")
        && variables["first"].as_u64()
            > Some(u64::from(onetaskgraph_github_projects::MAX_PAGE_SIZE))
    {
        return Some(format!(
            "Argument 'first' on Field 'items' has an invalid value ({}). Expected type 'Int'.",
            variables["first"]
        ));
    }
    let mut state = state.lock().unwrap();
    // GitHub answers a node read of an id that names nothing with an error rather than with a
    // null node alone, and a comment id is read this way before anything is changed — so
    // that read is answered the way GitHub answers it. A `rateLimit(dryRun: true)` probe of
    // the same document is not executed, so nothing in it is resolved and nothing refuses.
    if query.contains("on IssueComment{id issue{id}}") && board::strip_probe(query).is_none() {
        let id = variables["id"].as_str().expect("a node id");
        if !state.comments.iter().any(|held| held.id == id)
            && !state.items.iter().any(|item| item.content_id == id)
        {
            return Some(format!(
                "Could not resolve to a node with the global id of '{id}'."
            ));
        }
    }
    // Filing an issue the board already holds is refused, which is what a real board
    // answered to the filing that followed a create naming the board in `projectV2Ids`.
    if query.contains("addProjectV2ItemById(input:$input)") {
        let content = variables["input"]["contentId"].as_str();
        if state
            .items
            .iter()
            .any(|item| Some(item.content_id.as_str()) == content)
        {
            return Some("Content already exists in this project".to_owned());
        }
    }
    let operation = operation_name(query);
    let delayed_refusal = state
        .refuse_after
        .get_mut(operation)
        .is_some_and(|remaining| {
            if *remaining == 0 {
                true
            } else {
                *remaining -= 1;
                false
            }
        });
    if !state.refuses.contains(operation) && !delayed_refusal {
        return None;
    }
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state.seen.push(json!([operation, input]));
    }
    Some(format!("{operation} is refused by this board"))
}

/// What this board prefixes a repository's `owner/name` with to make its node id.
///
/// One distinct id per repository, so a `createIssue` says which repository it named:
/// under one shared id every lookup answered alike and a created issue could only be
/// given the board's own repository.
const REPOSITORY_NODE_PREFIX: &str = "REPO_";

fn repository_node_id(slug: &str) -> String {
    format!("{REPOSITORY_NODE_PREFIX}{slug}")
}

fn answer(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Value {
    if query == onetaskgraph_github_projects::graphql::UPDATE_FIELDS {
        let mut result = serde_json::Map::new();
        let writes = onetaskgraph_github_projects::FIELD_WRITE_SLOTS.map(|slot| {
            (
                slot,
                onetaskgraph_github_projects::graphql::UPDATE_FIELD,
                "updateProjectV2ItemFieldValue",
            )
        });
        let clears = onetaskgraph_github_projects::FIELD_CLEAR_SLOTS.map(|slot| {
            (
                slot,
                onetaskgraph_github_projects::graphql::CLEAR_FIELD,
                "clearProjectV2ItemFieldValue",
            )
        });
        for (slot, document, key) in writes.into_iter().chain(clears) {
            let (alias, variable, enabled) =
                (slot.alias, slot.variable, variables[slot.include] == true);
            if enabled {
                let answer = answer(state, document, &json!({"input":variables[variable]}));
                result.insert(alias.to_owned(), answer[key].clone());
            }
        }
        let mut held = state.lock().unwrap();
        if held
            .answers_alias_with
            .as_ref()
            .is_some_and(|(alias, _)| result.contains_key(*alias))
            && let Some((alias, value)) = held.answers_alias_with.take()
        {
            result.insert(alias.to_owned(), value);
        }
        return Value::Object(result);
    }
    let mut state = state.lock().unwrap();
    state
        .bindings
        .push((operation_name(query).to_owned(), variables.clone()));
    // Which read this is, taken from the document itself: every label this board answers
    // with hangs on the content, and which path asked is what lets one case make a single
    // path disagree.
    let asked = Asked {
        path: operation_name(query),
        board_items: variables["boardItems"].as_u64().unwrap_or_default() as usize,
        stuck_cursor: state.stuck_membership_cursor,
    };
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state
            .seen
            .push(json!([operation_name(query), input.clone()]));
    }
    if query.contains("updateProjectV2Field(input:$input)") && board::strip_probe(query).is_none() {
        let supplied = input["singleSelectOptions"]
            .as_array()
            .expect("a complete Status option list");
        assert!(
            supplied
                .iter()
                .filter(|option| option["name"] != "Queued")
                .all(|option| option["id"].is_string()),
            "every existing option keeps its id"
        );
        let mut next = supplied.clone();
        for (index, option) in next.iter_mut().enumerate() {
            if option.get("id").is_none() {
                option["id"] = json!(format!("OPT_added_{index}"));
            }
        }
        if state.drift_guarded_status_description {
            next[0]["description"] = json!("changed elsewhere");
            state.drift_guarded_status_description = false;
        }
        state.guarded_status_options = Some(next.clone());
        return json!({"updateProjectV2Field":{"projectV2Field":{
            "id":"FIELD_status","options":next
        }}});
    }
    if query.contains("optionId field{") && board::strip_probe(query).is_none() {
        let options = state.guarded_status_options.clone().unwrap_or_else(|| {
            state
                .options
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name,"color":"BLUE","description":""}))
                .collect()
        });
        let nodes = state
            .items
            .iter()
            .map(|item| {
                let values = item.status.as_ref().map_or_else(Vec::new, |name| {
                    let id = options
                        .iter()
                        .find(|option| option["name"] == name.as_str())
                        .and_then(|option| option["id"].as_str())
                        .expect("an assigned option")
                        .to_owned();
                    vec![json!({"name":name,"optionId":id,
                        "field":{"id":"FIELD_status","name":"Status"}})]
                });
                let item_id = if state.blank_status_snapshot_id == Some("item") {
                    ""
                } else {
                    item.item_id.as_str()
                };
                json!({"id":item_id,"fieldValues":{"nodes":values,
                    "pageInfo":{"hasNextPage":false}}})
            })
            .collect::<Vec<_>>();
        let board_id = if state.blank_status_snapshot_id == Some("board") {
            ""
        } else {
            "PVT_board"
        };
        let field_id = if state.blank_status_snapshot_id == Some("field") {
            ""
        } else {
            "FIELD_status"
        };
        return json!({"owner":{"projectV2":{"id":board_id,
            "fields":{"nodes":[{"id":field_id,"name":"Status","options":options}],
                "pageInfo":{"hasNextPage":false}},
            "items":{"nodes":nodes,"pageInfo":{
                "hasNextPage":state.blank_status_snapshot_id == Some("cursor"),
                "endCursor":(state.blank_status_snapshot_id == Some("cursor")).then_some("")}}
        }}});
    }
    if let Some(answered) = answer_a_session_call(&mut state, query, variables, &input) {
        return answered;
    }
    if query.contains("addComment(input:$input)") {
        let subject = input["subjectId"]
            .as_str()
            .expect("a subject id")
            .to_owned();
        assert!(
            state
                .items
                .iter()
                .any(|item| item.content_id == subject && item.typename == "Issue"),
            "addComment names {subject}, which is no issue this board holds"
        );
        let body = input["body"].as_str().expect("a comment body").to_owned();
        // Signed as the token's account whatever the input said, because GitHub's input has
        // nowhere to say anything else.
        let added = state.comment(&subject, Some(COMMENTER), &body);
        // GitHub moves the issue's `updatedAt` with every comment written on it.
        state.find(&json!(subject)).updated_at = Some(added.updated_at.clone());
        return json!({"addComment":{"subject":{"id":subject},
                                    "commentEdge":{"node":added.as_node()}}});
    }
    if query.contains("updateIssueComment(input:$input)") {
        let at = state.tick();
        let held = state
            .comments
            .iter_mut()
            .find(|held| input["id"] == held.id.as_str())
            .expect("updateIssueComment names a comment this board holds");
        held.body = input["body"].as_str().expect("a comment body").to_owned();
        held.updated_at = at.clone();
        let (issue, node) = (held.issue.clone(), held.as_node());
        // And with every comment edited on it, which is what a comment-activity read's
        // `updated:` search depends on.
        state.find(&json!(issue)).updated_at = Some(at);
        return json!({"updateIssueComment":{"issueComment":node}});
    }
    if query.contains("deleteIssueComment(input:$input)") {
        let id = input["id"].as_str().expect("a comment id").to_owned();
        assert!(
            state.comments.iter().any(|held| held.id == id),
            "deleteIssueComment names {id}, which this board does not hold"
        );
        state.comments.retain(|held| held.id != id);
        return json!({"deleteIssueComment":{"clientMutationId":null}});
    }
    if query.contains("on IssueComment{id issue{id}}") {
        let id = variables["id"].as_str().expect("a node id");
        if let Some(held) = state.comments.iter().find(|held| held.id == id) {
            return json!({"node":{"__typename":"IssueComment","id":id,
                                  "issue":{"id":held.issue}}});
        }
        // `refused` has already answered an id naming nothing, so this one names an item.
        let item = state
            .items
            .iter()
            .find(|item| item.content_id == id)
            .expect("an id this board holds");
        return json!({"node":{"__typename":item.typename}});
    }
    if query == onetaskgraph_github_projects::graphql::ISSUE_DETAILS {
        // Each alias is the one-item detail read of the id its own variable names, and the
        // comments ride along only when the batch asked for them.
        let mut answered = serde_json::Map::new();
        for slot in 0..onetaskgraph_github_projects::DETAIL_BATCH {
            let id = variables[format!("id{slot}")]
                .as_str()
                .expect("every slot of a batch names an id")
                .to_owned();
            let node = detail_node(
                &mut state,
                &id,
                &Value::Null,
                variables["first"].as_u64().expect("first") as usize,
                variables["comments"] == json!(true),
                asked,
            );
            answered.insert(format!("i{slot}"), node);
        }
        return Value::Object(answered);
    }
    if query == onetaskgraph_github_projects::graphql::ISSUE_DETAIL {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let node = detail_node(
            &mut state,
            &id,
            &variables["after"],
            variables["first"].as_u64().expect("first") as usize,
            true,
            asked,
        );
        return json!({ "node": node });
    }
    if query.contains("comments(first:$first,after:$after)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        state.comment_reads.push(id.clone());
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let comments = comment_connection(
            &state,
            &id,
            &variables["after"],
            variables["first"].as_u64().expect("first") as usize,
        );
        return json!({"node":{"__typename":"Issue","comments":comments}});
    }
    if query.contains("repository(owner:$owner,name:$name)") {
        // GitHub declares both arguments `String!`, so a lookup arriving without them is
        // a malformed request from the source under test, named rather than answered.
        let argument = |name: &str| {
            variables[name]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    panic!("a repository lookup without a string {name} argument: {variables}")
                })
        };
        let slug = format!("{}/{}", argument("owner"), argument("name"));
        return if variables["name"] == "missing" {
            json!({ "repository": null })
        } else {
            json!({"repository":{"id":repository_node_id(&slug),"nameWithOwner":slug}})
        };
    }
    if query.contains("deleteIssue(input:$input)") {
        let id = input["issueId"].as_str().expect("an issue id").to_owned();
        let repository = state
            .items
            .iter()
            .chain(state.pending.iter())
            .find(|item| item.content_id == id)
            .and_then(|item| item.repository.clone())
            .unwrap_or_else(|| "acme/work".to_owned());
        state.items.retain(|item| item.content_id != id);
        state.pending.retain(|item| item.content_id != id);
        return json!({"deleteIssue":{"repository":{"id":repository_node_id(&repository)}}});
    }
    if query.contains("createIssue(input:$input)") {
        state.next += 1;
        let id = format!("I_new{}", state.next);
        let mut created = Item::issue(&id, input["title"].as_str().unwrap_or_default());
        // Its own band, so an assertion about a created issue's number cannot pass against
        // a number a seeded board item happened to be wearing.
        created.number = 2000 + state.next as u64;
        created.body = input["body"].as_str().map(str::to_owned);
        // The issue is in the repository whose node id the input carried, which is how a
        // read of it derives the repository the source chose rather than the board's own.
        created.repository = Some(
            input["repositoryId"]
                .as_str()
                .and_then(|id| id.strip_prefix(REPOSITORY_NODE_PREFIX))
                .expect("createIssue names a repository this board resolved")
                .to_owned(),
        );
        let number = if state.creation_number_is_unreadable {
            json!("not-a-number")
        } else {
            state
                .creation_reports_number
                .then_some(created.number)
                .map_or(Value::Null, |number| json!(number))
        };
        // A create naming the board in `projectV2Ids` is filed on it, as GitHub filed one
        // when a real board was asked — after answering, so nothing of that filing is in the
        // answer, and the `addProjectV2ItemById` that would then follow is refused.
        let filed_at_creation = input["projectV2Ids"]
            .as_array()
            .is_some_and(|boards| boards.iter().any(|board| board == "PVT_board"));
        if filed_at_creation {
            state.items.push(created);
        } else {
            state.pending.push(created);
        }
        // GitHub answers the creating mutation with the issue's own address and number,
        // which is the only place a run learns either before its board read catches up.
        let mut issue = json!({"id":id, "url":format!("https://github.example/{id}")});
        if !number.is_null() {
            issue["number"] = number;
        }
        return json!({ "createIssue": { "issue": issue } });
    }
    if query.contains("addProjectV2ItemById(input:$input)") {
        let content = input["contentId"]
            .as_str()
            .expect("a content id")
            .to_owned();
        let position = state
            .pending
            .iter()
            .position(|item| item.content_id == content)
            .expect("the issue was created first");
        let item = state.pending.remove(position);
        let item_id = item.item_id.clone();
        state.items.push(item);
        return json!({"addProjectV2ItemById":{"item":{"id":item_id}}});
    }
    if query.contains("updateIssue(input:$input)") {
        let item = state.find(&input["id"]);
        if let Some(title) = input["title"].as_str() {
            item.title = title.to_owned();
        }
        if input.get("body").is_some() {
            item.body = input["body"].as_str().map(str::to_owned);
        }
        if let Some(state_input) = input.get("stateInput").filter(|value| !value.is_null()) {
            item.state = if state_input["value"] == "CLOSED" {
                "CLOSED"
            } else {
                "OPEN"
            };
            item.state_reason = state_input["stateReason"].as_str().map(str::to_owned);
        }
        return json!({"updateIssue":{"issue":{"id":input["id"]}}});
    }
    if query.contains("updateProjectV2DraftIssue(input:$input)") {
        let item = state.find(&input["draftIssueId"]);
        // `UpdateProjectV2DraftIssueInput.title` and `.body` are both nullable in the pinned
        // schema, and a field the input leaves out is one GitHub leaves as it is.
        if let Some(title) = input["title"].as_str() {
            item.title = title.to_owned();
        }
        if input.get("body").is_some() {
            item.body = input["body"].as_str().map(str::to_owned);
        }
        return json!({"updateProjectV2DraftIssue":{"draftIssue":{"id":input["draftIssueId"]}}});
    }
    if query.contains("updateProjectV2ItemFieldValue(input:$input)") {
        let item_id = input["itemId"].as_str().unwrap().to_owned();
        let option = input["value"]["singleSelectOptionId"]
            .as_str()
            .and_then(|id| {
                state
                    .options
                    .iter()
                    .find(|(known, _)| *known == id)
                    .map(|(_, name)| (*name).to_owned())
            });
        let text = input["value"]["text"].as_str().map(str::to_owned);
        let item = state
            .items
            .iter_mut()
            .find(|item| item.item_id == item_id)
            .expect("a field update names a board item");
        if let Some(option) = option {
            item.status = Some(option);
        }
        match (text, input["fieldId"].as_str()) {
            (Some(text), Some(field)) if TEXT_FIELD_NAMES.iter().any(|(id, _)| *id == field) => {
                item.texts.insert(field.to_owned(), text);
            }
            (Some(text), _) => item.origin = Some(text),
            (None, _) => {}
        }
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id}}});
    }
    if query.contains("clearProjectV2ItemFieldValue(input:$input)") {
        let item_id = input["itemId"].as_str().unwrap().to_owned();
        let item = state
            .items
            .iter_mut()
            .find(|item| item.item_id == item_id)
            .expect("a field clear names a board item");
        item.texts
            .remove(input["fieldId"].as_str().expect("a field id"));
        return json!({"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id}}});
    }
    if query.contains("addSubIssue(input:$input)") || query.contains("removeSubIssue(input:$input)")
    {
        let adding = query.contains("addSubIssue");
        let parent = input["issueId"].as_str().unwrap().to_owned();
        let child = input["subIssueId"].clone();
        state.find(&child).parent = adding.then(|| parent.clone());
        let held = state
            .items
            .iter()
            .filter(|item| item.parent.as_deref() == Some(parent.as_str()))
            .count() as u64;
        if let Some(item) = state
            .items
            .iter_mut()
            .find(|item| item.content_id == parent)
        {
            item.sub_issues = held;
        }
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
        let adding = query.contains("addBlockedBy");
        let issue = input["issueId"].as_str().unwrap().to_owned();
        let blocker = input["blockingIssueId"].as_str().unwrap().to_owned();
        let edges = state.blocked_by.entry(issue.clone()).or_default();
        if adding {
            edges.push(blocker.clone());
        } else {
            edges.retain(|held| held != &blocker);
        }
        let root = if adding {
            "addBlockedBy"
        } else {
            "removeBlockedBy"
        };
        return json!({root:{"issue":{"id":issue},"blockingIssue":{"id":blocker}}});
    }
    if query.contains("originItems:repositoryOwner(") {
        return answer_an_origin_lookup(&mut state, variables, asked);
    }
    if query.contains("search(query:$search") {
        assert_eq!(variables["type"], "ISSUE");
        let search = variables["search"].as_str().expect("a search query");
        let wanted = search
            .strip_prefix("project:octo-org/7 is:issue")
            .unwrap_or_else(|| panic!("a search scoped to the configured board: {search}"))
            .to_owned();
        state.searches.push(search.to_owned());
        // The server side of every qualifier this source sends: `in:title "..."`, which is
        // what makes naming a project by name one bounded query rather than a walk of the
        // board; `updated:>=<instant>`, which is how a read narrowed to comment activity asks
        // for the issues changed since; and the quoted phrases a text or metadata read asks
        // for, matched by token the way GitHub matches them.
        let wanted = IssueSearch::parse(&wanted);
        let offset = match &variables["after"] {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
            other => panic!("after must be null or a string: {other}"),
        };
        let first = variables["first"].as_u64().expect("first") as usize;
        // A search index is behind what the board really holds, exactly as GitHub's is —
        // which is what a read taken straight after a write has to answer through anyway.
        let visible = state.items.len().saturating_sub(state.lagging_reads);
        let options = state.options();
        let indexed: Vec<Item> = state.items[..visible]
            .iter()
            .map(|item| state.indexed(item))
            .collect();
        let mut matched = indexed
            .iter()
            .filter(|item| item.listed && item.typename == "Issue")
            .filter(|item| {
                wanted.admits(
                    &item.title,
                    item.body.as_deref().unwrap_or_default(),
                    item.updated_at.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        if state.search_order_varies_with_first {
            // One deterministic order per page size, and a different one at each: the same
            // request is always answered alike, and two sizes rarely agree.
            matched.sort_by_cached_key(|item| {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (&item.content_id, first).hash(&mut hasher);
                hasher.finish()
            });
        }
        let end = (offset + first).min(matched.len());
        let nodes = matched[offset.min(end)..end]
            .iter()
            .map(|item| item.as_issue(&options, asked))
            .collect::<Vec<_>>();
        return json!({"search":{"nodes":nodes,
            "pageInfo":{"hasNextPage":end < matched.len(),"endCursor":end.to_string()}}});
    }
    if query.contains("subIssues(first:$first") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(parent) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if parent.typename != "Issue" {
            return json!({"node":{"__typename":parent.typename}});
        }
        let offset = match &variables["after"] {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
            other => panic!("after must be null or a string: {other}"),
        };
        let first = variables["first"].as_u64().expect("first") as usize;
        let options = state.options();
        let children = state
            .items
            .iter()
            .filter(|item| item.parent.as_deref() == Some(id.as_str()))
            .collect::<Vec<_>>();
        let end = (offset + first).min(children.len());
        let nodes = children[offset.min(end)..end]
            .iter()
            .map(|item| item.as_issue(&options, asked))
            .collect::<Vec<_>>();
        return json!({"node":{"__typename":"Issue",
            "subIssues":{"nodes":nodes,
                "pageInfo":{"hasNextPage":end < children.len(),"endCursor":end.to_string()}}}});
    }
    if query.contains("on Issue{projectItems(first:$first,after:$after)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let offset = match &variables["after"] {
            Value::String(cursor) => cursor.parse::<usize>().unwrap_or_else(|_| {
                // The one cursor this board answers that is not an offset is the one a case
                // wedged it with; anything else is the source resuming from somewhere this
                // board never sent it.
                assert!(
                    asked.stuck_cursor.is_some(),
                    "a membership walk resumed from {cursor:?}, which this board never sent"
                );
                0
            }),
            other => panic!("a membership walk resumes from a cursor: {other}"),
        };
        let first = variables["first"].as_u64().expect("first") as usize;
        let options = state.options();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            // Not an issue, so the `... on Issue` arm selects nothing of it.
            return json!({ "node": {} });
        }
        let page = membership_page(&item.memberships(&options), offset, first, asked);
        return json!({"node":{"projectItems":page}});
    }
    if query.contains("query TerminalStatus") {
        let id = variables["id"].as_str().expect("an issue id");
        let item = state
            .items
            .iter()
            .find(|item| item.content_id == id)
            .expect("terminal read-back names an issue this board holds");
        return json!({"node":{"state":item.state,"stateReason":item.state_reason,
            "projectItems":{"nodes":[{"project":{"id":"PVT_board"},
                "fieldValues":item.field_values(&state.options())}]}}});
    }
    if query == onetaskgraph_github_projects::graphql::CREATION_CONTEXT {
        // The board's fields and the repository's id, each as its own document answers it.
        let slug = format!(
            "{}/{}",
            variables["repositoryOwner"]
                .as_str()
                .expect("a repository owner"),
            variables["repositoryName"]
                .as_str()
                .expect("a repository name")
        );
        let repository = if variables["repositoryName"] == "missing" {
            Value::Null
        } else {
            json!({"id":repository_node_id(&slug),"nameWithOwner":slug})
        };
        return json!({"boardFields":{"projectV2":{"id":"PVT_board","fields":state.fields()}},
                      "repository":repository});
    }
    if query.contains("boardFields:repositoryOwner(login:$owner)") {
        assert_eq!(variables["owner"], json!("octo-org"));
        assert_eq!(variables["number"], json!(7));
        return json!({"boardFields":{"projectV2":{"id":"PVT_board","fields":state.fields()}}});
    }
    if query.contains("on DraftIssue{") && query.contains("projectV2Items(first:$boardItems)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "DraftIssue" {
            // Not a draft, so the `... on DraftIssue` arm selects nothing of it.
            return json!({"node":{"__typename":item.typename}});
        }
        let options = state.options();
        let mut draft = item.content(asked);
        draft["projectV2Items"] = item.project_items(&options, asked);
        return json!({ "node": draft });
    }
    if query == onetaskgraph_github_projects::graphql::ISSUE {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let options = state.options();
        let mut node = item.as_issue(&options, asked);
        // The boards it sits on with their fields, and what blocks it, as the read by id asks.
        node["boards"] = json!({"nodes":item.memberships(&options).iter().map(|membership| {
            let mut project = membership["project"].clone();
            if project["number"] == 7 {
                project["fields"] = state.fields();
            } else {
                project["fields"] = json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
            }
            json!({"project":project})
        }).take(asked.board_items).collect::<Vec<_>>()});
        if item.fields_of_a_namesake_ahead {
            let options = state
                .options
                .iter()
                .map(|(id, name)| json!({"id":format!("{id}_elsewhere"),"name":name}))
                .collect::<Vec<_>>();
            node["boards"]["nodes"].as_array_mut().unwrap().insert(
                0,
                json!({"project":{"id":"PVT_elsewhere","number":7,"fields":{"nodes":[
                    {"__typename":"ProjectV2SingleSelectField","id":"FIELD_status_elsewhere",
                     "name":"Status","options":options},
                    {"__typename":"ProjectV2Field","id":"FIELD_origin_elsewhere",
                     "name":"onetaskgraph.origin"}
                ],"pageInfo":{"hasNextPage":false}}}}),
            );
        }
        node["blockedBy"] = json!({"nodes":related_issues(&state, state.blocked_by.get(&id).cloned().unwrap_or_default()),
            "pageInfo":{"hasNextPage":false,"endCursor":null}});
        return json!({ "node": node });
    }
    if query.contains("node(id:$id)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let related = |ids: Vec<String>| related_issues(&state, ids);
        let blocked = state.blocked_by.get(&id).cloned().unwrap_or_default();
        let blocking = state
            .blocked_by
            .iter()
            .filter(|(_, blockers)| blockers.contains(&id))
            .map(|(issue, _)| issue.clone())
            .collect::<Vec<_>>();
        return json!({"node":{"__typename":"Issue","body":item.body.clone(),
            "blockedBy":{"nodes":related(blocked),"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":related(blocking),"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    }
    assert!(
        query.contains("projectV2(number:$number)"),
        "the fixture received an unknown operation: {query}"
    );
    assert_eq!(variables["duplicates"], json!(true));
    let offset = match &variables["after"] {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
        other => panic!("after must be null or a string: {other}"),
    };
    let first = variables["first"].as_u64().expect("first") as usize;
    // What this connection lists is what it had caught up with, which is everything unless
    // this board was left behind; see `State::items_connection_behind_from`.
    let listed = state
        .items_connection_behind_from
        .unwrap_or(state.items.len())
        .min(state.items.len());
    let shown = state.items[..listed.saturating_sub(state.lagging_reads)]
        .iter()
        .filter(|item| item.listed)
        .collect::<Vec<_>>();
    let visible = shown.len();
    let end = (offset + first).min(visible);
    let options = state.options();
    let nodes = shown[offset.min(end)..end]
        .iter()
        .map(|item| {
            json!({"id":item.item_id,"fieldValues":item.field_values(&options),
                   "content":item.content(asked)})
        })
        .collect::<Vec<_>>();
    json!({"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap","fields":state.fields(),
        "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < visible,"endCursor":end.to_string()}}}}})
}

/// The far ends a dependency read selects, each as the `Related` fragment reads one.
fn related_issues(state: &State, ids: Vec<String>) -> Value {
    Value::Array(
        ids.into_iter()
            .map(|id| {
                let far = state.items.iter().find(|item| item.content_id == id);
                json!({"id":id,
                       "title":far.map(|item| item.title.clone()).unwrap_or_default(),
                       "body":far.and_then(|item| item.body.clone()),
                       "parent":far.and_then(|item| item.parent.clone()).map(|id| json!({"id":id})),
                       "subIssuesSummary":{"total":far.map_or(0, |item| item.sub_issues)}})
            })
            .collect(),
    )
}

/// One page of an issue's comments, oldest first, resumed from `after`.
fn comment_connection(state: &State, id: &str, after: &Value, first: usize) -> Value {
    let offset = match after {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
        other => panic!("after must be null or a string: {other}"),
    };
    let on = state
        .comments
        .iter()
        .filter(|held| held.issue == id)
        .collect::<Vec<_>>();
    let end = (offset + first).min(on.len());
    let nodes = on[offset.min(end)..end]
        .iter()
        .map(|held| held.as_node())
        .collect::<Vec<_>>();
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":end < on.len(),
                                      "endCursor":(end > offset).then(|| end.to_string())}})
}

/// One node a detail read reaches: the issue as every node read answers it, with a page of its
/// comments when they were asked for — and a draft or a pull request answered by its type
/// alone, as GitHub answers a fragment on `Issue` about something that is not one.
fn detail_node(
    state: &mut State,
    id: &str,
    after: &Value,
    first: usize,
    comments: bool,
    asked: Asked,
) -> Value {
    let Some(item) = state
        .items
        .iter()
        .find(|item| item.content_id == id)
        .cloned()
    else {
        return Value::Null;
    };
    if item.typename != "Issue" {
        return json!({"__typename":item.typename});
    }
    let mut node = item.as_issue(&state.options(), asked);
    if comments {
        state.comment_reads.push(id.to_owned());
        node["comments"] = comment_connection(state, id, after, first);
    }
    node
}

/// The calls a *session* makes that the source itself never does, answered by this board.
///
/// One whole session of the live journey is the source's own reads and writes plus the
/// journey's — a schema verification, the reconciliation with GitHub, the board and field
/// lookups, the residue sweep and the cleanup. Counting what a session costs means driving
/// all of it, so this board answers all of it. `None` means the document is one of the
/// source's own and [`answer`] goes on to it.
///
/// **What this stands in for, and what it therefore cannot prove.** The schema
/// introspection is answered *from the journey's own contract tables*, and the
/// `rateLimit(dryRun: true)` probe from this workspace's own `worst_case_node_count` and
/// `worst_case_point_cost`, so a drive against this board agrees with itself by
/// construction. That is the point: GitHub is the authority on all three, and the
/// credentialed drive is where they are really reconciled — what a board that reports
/// something else does to a run is `tests/reconciliation_gate.rs`, which points this same
/// journey at the same answers under [`Pricing::Overstating`]. What a drive against this
/// board measures is how many requests a session makes and what each one carries, which is a
/// property of the journey rather than of GitHub.
fn answer_a_session_call(
    state: &mut State,
    query: &str,
    variables: &Value,
    input: &Value,
) -> Option<Value> {
    // The probe, the introspection and the allowance read, answered from this workspace's
    // own tables and calculations rather than from anything this board holds. First, because
    // the production document a probe was joined to is still in the text and answering that
    // would run a query GitHub would not have. This board prices as this workspace computes;
    // `tests/reconciliation_gate.rs` is the same code answering something else.
    if let Some(answered) =
        board::answer_a_stateless_session_call(query, variables, Pricing::AsComputed)
    {
        return Some(answered);
    }
    // Narrowed to the lane's own board lookup, which selects the id and nothing else:
    // `graphql::BOARD` reaches `repositoryOwner` too, and answering it here would hand the
    // source a board with no items.
    if query.contains("projectV2(number:$number){id}}") {
        assert_eq!(variables["owner"], json!("octo-org"));
        assert_eq!(variables["number"], json!(7));
        return Some(json!({"repositoryOwner":{"projectV2":{"id":"PVT_board"}}}));
    }
    if query.contains("on ProjectV2{fields(first:100") {
        return Some(json!({"node":{"fields":state.fields()}}));
    }
    if query.contains("createProjectV2Field(input:$input)") {
        let name = input["name"].as_str().expect("a field name").to_owned();
        state.origin_field = true;
        return Some(json!({"createProjectV2Field":{"projectV2Field":
            {"id":"FIELD_origin","name":name}}}));
    }
    if query.contains("deleteProjectV2Field(input:$input)") {
        state.origin_field = false;
        return Some(json!({"deleteProjectV2Field":{"projectV2Field":{"id":input["fieldId"]}}}));
    }
    if query.contains("on ProjectV2{items(first:") {
        // The artifact sweep selects each item's content; the page-size probe selects only
        // its id. Both are answered off the one connection, in board order.
        let nodes = state
            .items
            .iter()
            .map(|item| match item.typename {
                "Issue" => json!({"id":item.item_id,
                    "content":{"__typename":"Issue","id":item.content_id,"title":item.title}}),
                "DraftIssue" => json!({"id":item.item_id,"content":{"title":item.title}}),
                _ => json!({"id":item.item_id,"content":{}}),
            })
            .collect::<Vec<_>>();
        return Some(json!({"node":{"items":{"nodes":nodes,
            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}));
    }
    if query.contains("deleteProjectV2Item(input:$input)") {
        let item_id = input["itemId"]
            .as_str()
            .expect("a board item id")
            .to_owned();
        state.items.retain(|item| item.item_id != item_id);
        return Some(json!({"deleteProjectV2Item":{"deletedItemId":item_id}}));
    }
    if query.contains("addLabelsToLabelable(input:$input)") {
        let labelable = input["labelableId"]
            .as_str()
            .expect("a labelable id")
            .to_owned();
        for label in input["labelIds"].as_array().expect("label ids") {
            let node_id = label.as_str().expect("a label node id");
            let (name, id) = state
                .labels
                .iter()
                .find(|(_, held)| held == node_id)
                .map(|(name, id)| (leaked(name), leaked(id)))
                .expect("a label this repository holds");
            let item = state
                .items
                .iter_mut()
                .find(|item| item.content_id == labelable)
                .expect("an issue this board holds");
            item.labels.push((id, name));
        }
        return Some(json!({"addLabelsToLabelable":{"labelable":{"id":labelable}}}));
    }
    None
}

/// GitHub's issue search, as far as this board models it: the qualifiers this source sends,
/// and the phrases it searches for, matched **by token** the way GitHub matches them.
///
/// A phrase holds of a field when its words appear there, one after another, as whole
/// words — case-insensitively, with every character that is not a letter or a digit a
/// separator. So `"Ship"` finds `Ship it` and never `Shipment`, which is the narrowing a
/// source that matches substrings has to state rather than hide; and GitHub's index covers
/// the whole body, the metadata comment at its end included.
struct IssueSearch {
    /// `updated:>=`, when the search carries it.
    updated_since: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// Whether a phrase may hold of the title.
    in_title: bool,
    /// Whether a phrase may hold of the body.
    in_body: bool,
    /// Every phrase, as its tokens; each must hold of one of the fields above.
    phrases: Vec<Vec<String>>,
}

impl IssueSearch {
    /// The search after its `project:… is:issue` scope.
    fn parse(wanted: &str) -> Self {
        let mut parsed = Self {
            updated_since: None,
            in_title: true,
            in_body: true,
            phrases: Vec::new(),
        };
        let mut chars = wanted.chars().peekable();
        while let Some(&next) = chars.peek() {
            if next.is_whitespace() {
                chars.next();
                continue;
            }
            if next == '"' {
                chars.next();
                let mut phrase = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => phrase.extend(chars.next()),
                        Some('"') | None => break,
                        Some(other) => phrase.push(other),
                    }
                }
                parsed.phrases.push(search_tokens(&phrase));
                continue;
            }
            let mut word = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                word.push(next);
                chars.next();
            }
            if let Some(fields) = word.strip_prefix("in:") {
                let fields: Vec<&str> = fields.split(',').collect();
                parsed.in_title = fields.contains(&"title");
                parsed.in_body = fields.contains(&"body");
            } else if let Some(instant) = word.strip_prefix("updated:>=") {
                parsed.updated_since = Some(
                    chrono::DateTime::parse_from_rfc3339(instant)
                        .unwrap_or_else(|error| panic!("an RFC 3339 instant in {wanted}: {error}")),
                );
            } else {
                parsed.phrases.push(search_tokens(&word));
            }
        }
        parsed
    }

    /// Whether an issue holding this title, body and `updatedAt` is in the answer.
    fn admits(&self, title: &str, body: &str, updated_at: Option<&str>) -> bool {
        let updated = self.updated_since.is_none_or(|since| {
            updated_at.is_some_and(|at| {
                chrono::DateTime::parse_from_rfc3339(at).expect("an instant") >= since
            })
        });
        let (title, body) = (search_tokens(title), search_tokens(body));
        let holds = |field: &[String], phrase: &[String]| {
            phrase.is_empty() || field.windows(phrase.len()).any(|window| window == phrase)
        };
        updated
            && self.phrases.iter().all(|phrase| {
                (self.in_title && holds(&title, phrase)) || (self.in_body && holds(&body, phrase))
            })
    }
}

/// The words GitHub's index holds of `text`: every run of letters and digits, lower-cased.
fn search_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The board's own field filter over its origin field, read out of the `query:` a
/// `ProjectV2.items` read carries: `onetaskgraph.origin:"<value>"`, quoted and escaped.
fn origin_filter(filter: &str) -> String {
    let quoted = filter
        .strip_prefix("onetaskgraph.origin:\"")
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("an origin field filter: {filter}"));
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => value.extend(chars.next()),
            other => value.push(other),
        }
    }
    value
}

/// One page of `rows` from `after`, `first` at a time, as a connection with its own
/// `totalCount`.
fn connection_page(rows: Vec<Value>, after: &Value, first: usize) -> Value {
    let offset = match after {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("a numeric cursor"),
        other => panic!("after must be null or a string: {other}"),
    };
    let end = (offset + first).min(rows.len());
    let nodes = rows[offset.min(end)..end].to_vec();
    json!({"totalCount":rows.len(),"nodes":nodes,
           "pageInfo":{"hasNextPage":end < rows.len(),
                       "endCursor":(end > 0).then(|| end.to_string())}})
}

/// The two halves of an origin lookup, answered the way GitHub answers each.
///
/// `originItems` is the board's own item connection under its field filter: an **exact**
/// match on the origin field's value, over the items that connection lists — so it lags a
/// fresh item exactly as the board's own item read does, which is
/// [`Fixture::items_connection_falls_behind`]. `search` is the board-scoped issue search for
/// the value as a phrase in the body, by token, over what the search index lists.
fn answer_an_origin_lookup(state: &mut State, variables: &Value, asked: Asked) -> Value {
    assert_eq!(variables["type"], "ISSUE");
    let filter = variables["filter"].as_str().expect("a field filter");
    let wanted = origin_filter(filter);
    state.origin_filters.push(filter.to_owned());
    let search = variables["search"].as_str().expect("a search query");
    state.searches.push(search.to_owned());
    let phrase = IssueSearch::parse(
        search
            .strip_prefix("project:octo-org/7 is:issue")
            .unwrap_or_else(|| panic!("a search scoped to the configured board: {search}")),
    );
    let first = variables["originFirst"].as_u64().expect("originFirst") as usize;
    let options = state.options();
    let listed = state
        .items_connection_behind_from
        .unwrap_or(state.items.len())
        .min(state.items.len());
    let carriers = state.items[..listed.saturating_sub(state.lagging_reads)]
        .iter()
        .map(|item| state.indexed(item))
        .filter(|item| item.listed && item.origin.as_deref() == Some(wanted.as_str()))
        .map(|item| {
            json!({"id":item.item_id,"fieldValues":item.field_values(&options),
                   "content":item.content(asked)})
        })
        .collect();
    let visible = state.items.len().saturating_sub(state.lagging_reads);
    let found = state.items[..visible]
        .iter()
        .map(|item| state.indexed(item))
        .filter(|item| item.listed && item.typename == "Issue")
        .filter(|item| {
            phrase.admits(
                &item.title,
                item.body.as_deref().unwrap_or_default(),
                item.updated_at.as_deref(),
            )
        })
        .map(|item| item.as_issue(&options, asked))
        .collect();
    json!({"originItems":{"projectV2":{
               "items":connection_page(carriers, &variables["itemsAfter"], first)}},
           "search":connection_page(found, &variables["searchAfter"], first)})
}

/// A name this board has to hand back with a `'static` lifetime it did not have.
///
/// The fixture's label sets are `&'static str` because every other test in this file spells
/// its labels as literals. A session creates one at run time, so this is where the two meet;
/// one leak per label of one test run.
fn leaked(value: &str) -> &'static str {
    Box::leak(value.to_owned().into_boxed_str())
}

fn is_mutation(query: &str) -> bool {
    query.trim_start().starts_with("mutation")
}

/// The operation this document carries, read out of the document itself.
///
/// Derived rather than matched against a list of the operations this source sends: a list
/// here would be a second copy of the production mutation inventory, and a mutation added
/// there and not here would go uncounted and unnamed in silence — which is exactly what
/// the pacing assertions below measure.
fn operation_name(query: &str) -> &str {
    let body = query
        .split_once('{')
        .map_or(query, |(_, rest)| rest)
        .trim_start();
    let root = &body[..body
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(body.len())];
    // The reads answer to what they read rather than to their GraphQL root, because
    // `owner` and `node` say nothing about what a test is counting — and three different
    // reads now share the `node` root. Which of them a document is, is still read off the
    // document: nothing is enumerated, and every mutation's name is the one its own
    // document spells.
    match root {
        "node" if query == onetaskgraph_github_projects::graphql::ISSUE => "issue",
        "owner" => "board",
        "node" if query.contains("subIssues(") => "projectTasks",
        "node" if query.contains("projectV2Items(") => "draft",
        "node" if query.contains("projectItems(first:$first") => "issueBoardItems",
        "node" if query.contains("blockedBy(") => "issueDependencies",
        "node" if query.contains("...BoardIssue ... on Issue{comments(") => "issueDetail",
        "i0" => "issueDetails",
        "node" if query.contains("comments(first:") => "issueComments",
        "node" if query.contains("on IssueComment{") => "comment",
        "node" => "issue",
        other => other,
    }
}

fn read_http_json(stream: &mut impl Read) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("a fixture request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header terminator")
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
        .expect("a content length");
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("a request body");
        assert!(count > 0, "the request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).expect("request JSON")
}

fn raw_server(status: &str, body: &str) -> String {
    raw_server_with_headers(status, body, "")
}

fn raw_server_with_headers(status: &str, body: &str, headers: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (body, status, headers) = (body.to_owned(), status.to_owned(), headers.to_owned());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let request = read_http_json(&mut stream);
            let (status, body) = match empty_board_search(&request) {
                Some(empty) => ("200 OK".to_owned(), empty),
                None => (status.clone(), body.clone()),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

/// An empty board-scoped issue search, for a request that is one.
///
/// A whole-board read is the union of GitHub's two enumerations of one board — see
/// `GitHubProjectsSource::board` — so a server scripting a board response is asked for the
/// search as well. The servers below script the *board* document and every case they carry
/// is about that response, so the search they answer holds nothing, which changes no case's
/// subject and no case's answer: an issue a board read leaves out is one this search does
/// not name either. It is the truthful answer for the one case where the two could disagree,
/// too — a token that cannot see an item's content cannot find that item by searching for it.
fn empty_board_search(request: &Value) -> Option<String> {
    request["query"]
        .as_str()
        // An origin lookup carries a search too, beside the board's own filtered items, and
        // is a request a case scripts rather than one this answers.
        .filter(|query| query.contains("search(query:$search") && !query.contains("originItems:"))
        .map(|_| {
            json!({"data":{"search":{"nodes":[],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}})
            .to_string()
        })
}

fn sequence_server(bodies: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let mut scripted = bodies.into_iter();
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_json(&mut stream);
            // Answered beside the script rather than out of it; see `empty_board_search`.
            // A script is an exchange somebody wrote down to make one refusal happen, and
            // spending one of its entries on the search would move every entry after it.
            let body = match empty_board_search(&request) {
                Some(empty) => empty,
                None => match scripted.next() {
                    Some(body) => body.to_string(),
                    // The script is spent: the server goes away, exactly as it did before,
                    // so a source asking for more is refused rather than answered.
                    None => return,
                },
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

/// A source over one fixture endpoint.
///
/// Pacing is switched off in the base configuration on purpose. The shipped default
/// spaces a mutation every 750 ms against github.com, and every write test here would
/// otherwise spend that per call proving something about status mapping or metadata. The
/// tests that are about pacing and about waiting a limiter out say so themselves, by
/// passing a `pacing` block of their own — so what is proven about the schedule is proven
/// where it is the subject, and `the_shipped_pacing_defaults_are_githubs_published_limits`
/// pins the shipped values against the ones a source built with no `pacing` block uses.
fn fixture_config(endpoint: &str, extra: &Value) -> Value {
    let mut config = json!({"owner":"octo-org","project_number":7,"endpoint":endpoint,
                            "repository":"acme/work",
                            "pacing":{"min_mutation_interval_ms":0,"retry_budget_ms":0}});
    for (key, value) in extra.as_object().expect("an object of overrides") {
        if value.is_null() {
            config.as_object_mut().unwrap().remove(key);
        } else {
            config[key] = value.clone();
        }
    }
    config
}

fn configured(endpoint: &str, extra: Value) -> Box<dyn TaskSource> {
    Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &fixture_config(endpoint, &extra),
            &Secrets,
        )
        .expect("a usable configuration")
}

/// The same source, recording every request it sends into an accounting this test holds
/// too — which is how a caller accounting for a whole session builds one.
fn recording(endpoint: &str, ledger: &Arc<Accounting>) -> Box<dyn TaskSource> {
    Plugin
        .build_recording_into(
            &SourceName::new("work").unwrap(),
            &fixture_config(endpoint, &json!({})),
            &Secrets,
            Arc::clone(ledger),
        )
        .expect("a usable configuration")
}

use onetaskgraph_github_projects::accounting::{
    Accounting, Basis, Budget, BudgetReport, Endpoint, Method, Mode, Outcome, RateLimit, Request,
    Session, StatusCode,
};
use onetaskgraph_github_projects::{
    DESIGN_TITLE_PREFIX, GitHubProjectsConfig, GitHubProjectsSource, Plugin, StatusOptionsMode,
    StatusOptionsOutcome, graphql,
};

fn source(fixture: &Fixture) -> Box<dyn TaskSource> {
    configured(&fixture.endpoint, json!({}))
}

fn status_options_source(fixture: &Fixture) -> GitHubProjectsSource {
    let config: GitHubProjectsConfig =
        serde_json::from_value(fixture_config(&fixture.endpoint, &json!({}))).unwrap();
    GitHubProjectsSource::new(&SourceName::new("work").unwrap(), config, &Secrets).unwrap()
}

fn refusal(error: SourceError) -> String {
    error.to_string()
}

/// The refusal a configuration that cannot be built answers with.
fn build_refusal(config: Value) -> String {
    match Plugin.build(&SourceName::new("work").unwrap(), &config, &Secrets) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("{config} was supposed to be refused"),
    }
}

fn task(id: &str, title: &str, status: Status) -> Task {
    Task {
        id: NativeId(id.to_owned()),
        key: None,
        title: title.to_owned(),
        content: None,
        status,
        priority: Priority::None,
        labels: vec![],
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    }
}

fn project(id: &str, title: &str, status: Status) -> Project {
    Project {
        id: NativeId(id.to_owned()),
        title: title.to_owned(),
        content: None,
        status,
        labels: vec![],
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

fn design(id: &str, title: &str) -> Item {
    Item::issue(id, &format!("{DESIGN_TITLE_PREFIX}{title}"))
}

fn document(id: &str, title: &str) -> Document {
    Document {
        id: NativeId(id.to_owned()),
        title: title.to_owned(),
        content: None,
        project: None,
        labels: vec![],
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

async fn selected_documents(source: &dyn TaskSource, query: &DocumentQuery) -> Vec<String> {
    source
        .query_documents(query, &page(10))
        .await
        .expect("the board answers a document query")
        .items
        .into_iter()
        .map(|document| document.id.0)
        .collect()
}

fn document_query(
    labels: LabelFilter,
    project: ProjectFilter,
    text: Option<TextQuery>,
) -> DocumentQuery {
    DocumentQuery {
        text,
        labels,
        project,
    }
}

fn status(category: StatusCategory, name: &str) -> Status {
    Status {
        category,
        name: name.to_owned(),
    }
}

fn write<T>(item: T) -> ItemWrite<T> {
    ItemWrite {
        target: None,
        item,
        depends_on: vec![],
    }
}

#[tokio::test]
async fn the_committed_board_fixture_maps_to_two_projects_their_tasks_an_orphan_and_no_pull_request()
 {
    // The committed fixtures are the drift artifacts the pinned-schema test validates, so
    // they are read here through a real socket rather than paraphrased.
    let source = committed_board();

    let projects = source
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .expect("the board lists its projects");
    assert_eq!(
        projects
            .items
            .iter()
            .map(|project| project.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_plan", "I_next"],
        "an issue with sub-issues and an issue carrying the kind marker are both projects"
    );
    assert_eq!(
        projects.items[0].content.as_deref(),
        Some("the delivery plan")
    );
    assert_eq!(projects.items[0].metadata["caller.enabled"], json!(true));
    assert_eq!(
        projects.items[0].repositories,
        vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()]
    );
    assert_eq!(
        projects.items[0].status,
        status(StatusCategory::InProgress, "In Progress")
    );

    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists its tasks");
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_task", "I_notes", "I_loose"],
        "each project's own sub-issue is a task, so is the issue under no parent, and the \
         pull request is neither"
    );
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.project.as_ref().map(|id| id.0.as_str()))
            .collect::<Vec<_>>(),
        [Some("I_plan"), Some("I_next"), None],
        "the board holds a task under each of its two projects and one under neither"
    );
    let one = &tasks.items[0];
    assert_eq!(one.project, Some(NativeId("I_plan".to_owned())));
    assert_eq!(one.content.as_deref(), Some("details"));
    assert_eq!(one.metadata["caller.number"], json!(7));
    assert_eq!(
        one.metadata["onetaskgraph.origin"],
        json!("notes:T-1"),
        "the copy origin is kept in a field of its own, not in the body slot"
    );
    assert!(
        !one.metadata.contains_key(ItemKind::METADATA_KEY),
        "the kind marker is this source's own encoding and never travels as metadata"
    );
    assert_eq!(
        one.labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["bug", "team"]
    );
}

/// The committed board fixture, served over a real socket.
///
/// It holds two projects, a task under each of them, a task under neither, and a pull
/// request, so one board answers every shape of the project filter.
///
/// Three committed artifacts rather than one, because this source reaches the same board
/// three ways and each way has a recorded shape of its own: `project.json` is the board's
/// own item connection, `issues.json` is the board-scoped issue search, and
/// `sub-issues.json` is one project's own sub-issues. Every one of them is validated
/// against its production document by the pinned-schema test, so the shapes this suite
/// reads cannot drift from the shapes those documents ask for.
fn committed_board() -> Box<dyn TaskSource> {
    configured(&committed_server(), json!({}))
}

/// The committed fixtures, served over a real socket by the document that asks for them.
fn committed_server() -> String {
    let board: Value = serde_json::from_str(include_str!("fixtures/project.json")).unwrap();
    let issues: Value = serde_json::from_str(include_str!("fixtures/issues.json")).unwrap();
    let children: Value = serde_json::from_str(include_str!("fixtures/sub-issues.json")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let request = read_http_json(&mut stream);
            let query = request["query"].as_str().expect("a GraphQL document");
            let variables = &request["variables"];
            let recorded = issues
                .pointer("/data/search/nodes")
                .unwrap()
                .as_array()
                .unwrap();
            let body = if query.contains("search(query:$search") {
                let search = variables["search"].as_str().expect("a search query");
                let wanted = IssueSearch::parse(
                    search
                        .strip_prefix("project:octo-org/7 is:issue")
                        .expect("a search scoped to the configured board"),
                );
                let matched = recorded
                    .iter()
                    .filter(|node| {
                        wanted.admits(
                            node["title"].as_str().unwrap_or_default(),
                            node["body"].as_str().unwrap_or_default(),
                            node["updatedAt"].as_str(),
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                json!({"data":{"search":{"nodes":matched,
                    "pageInfo":{"hasNextPage":false,"endCursor":null}}}})
            } else if query.contains("subIssues(first:$first") {
                let id = variables["id"].as_str().expect("a node id");
                if id == "I_plan" {
                    children.clone()
                } else if recorded.iter().any(|node| node["id"] == json!(id)) {
                    let held = recorded
                        .iter()
                        .filter(|node| node.pointer("/parent/id") == Some(&json!(id)))
                        .cloned()
                        .collect::<Vec<_>>();
                    json!({"data":{"node":{"__typename":"Issue",
                        "subIssues":{"nodes":held,
                            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}})
                } else {
                    json!({ "data": { "node": null } })
                }
            } else if query.contains("node(id:$id){__typename ...BoardIssue}") {
                let id = variables["id"].as_str().expect("a node id");
                let held = recorded.iter().find(|node| node["id"] == json!(id));
                json!({"data":{"node":held.cloned().unwrap_or(Value::Null)}})
            } else {
                board.clone()
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

async fn selected_tasks(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    source
        .query_tasks(query, &page(10))
        .await
        .expect("the board answers a task query")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect()
}

async fn selected_projects(source: &dyn TaskSource, query: &ProjectQuery) -> Vec<String> {
    source
        .query_projects(query, &page(10))
        .await
        .expect("the board answers a project query")
        .items
        .into_iter()
        .map(|project| project.id.0)
        .collect()
}

fn label_filter(any_of: &[&str], all_of: &[&str], none_of: &[&str]) -> LabelFilter {
    let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    LabelFilter {
        any_of: owned(any_of),
        all_of: owned(all_of),
        none_of: owned(none_of),
    }
}

fn text(terms: &str, fields: TextFields) -> Option<TextQuery> {
    Some(TextQuery {
        terms: terms.to_owned(),
        fields,
    })
}

#[tokio::test]
async fn a_task_read_scoped_to_one_project_returns_only_that_projects_tasks() {
    // This source declares `projects` native, so the engine pushes the filter down and
    // applies nothing of its own. Ignoring it here returned every task on the board, which
    // is how a second plan on one board corrupted the first.
    let source = committed_board();

    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_plan".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_task"],
        "a board holding a second project must not answer with that project's tasks"
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_next".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_notes"]
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Orphans,
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_loose"],
        "a task under no parent is the one a project-less selection keeps"
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_nothing".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        Vec::<String>::new(),
        "a project the board does not hold selects nothing rather than everything"
    );
    assert_eq!(
        selected_tasks(source.as_ref(), &TaskQuery::default()).await,
        ["I_task", "I_notes", "I_loose"],
        "an unconstrained query still answers with the whole board"
    );
}

/// A board holding `projects` projects, each with `tasks` tasks filed under it.
///
/// The shape a cost claim needs: several projects, each holding work of its own, so that a
/// read scoped to one of them can be told from a read of all of them.
fn board_of(projects: usize, tasks: usize) -> Fixture {
    let mut items = Vec::new();
    for plan in 1..=projects {
        let id = format!("I_p{plan}");
        // The kind marker as well as the sub-issue count, so that a plan holding nothing
        // yet is still a project — which is the state a project copy passes through
        // between creating the project and filing its first task.
        items.push(
            Item::issue(&id, &format!("Plan {plan}"))
                .status("Todo")
                .sub_issues(tasks as u64)
                .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->"),
        );
        for step in 1..=tasks {
            items.push(
                Item::issue(&format!("I_p{plan}t{step}"), &format!("Step {plan}.{step}"))
                    .status("Todo")
                    .parent(&id),
            );
        }
    }
    board(items)
}

/// The title of each task one project-scoped read reports, in its order.
async fn project_titles(source: &dyn TaskSource, project: &str) -> Vec<(String, String)> {
    source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Is(NativeId(project.to_owned())),
                ..TaskQuery::default()
            },
            &page(10),
        )
        .await
        .expect("the board answers a task query")
        .items
        .into_iter()
        .map(|task| (task.id.0, task.title))
        .collect()
}

#[tokio::test]
async fn a_projects_tasks_are_read_once_a_command_and_kept_in_step_with_its_own_writes() {
    // A caller pages through a project's tasks one page at a time, and each page is cut from
    // the whole list, so the list is read once a command and held — which is only right while
    // what is held keeps up with what this source writes, and is let go of when the command
    // ends, so the next one reads what a person has changed since.
    let fixture = board_of(2, 2);
    let source = source(&fixture);
    let reads = || fixture.requests("projectTasks");
    let titled = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(id, title)| ((*id).to_owned(), (*title).to_owned()))
            .collect()
    };

    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[("I_p1t1", "Step 1.1"), ("I_p1t2", "Step 1.2")])
    );
    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[("I_p1t1", "Step 1.1"), ("I_p1t2", "Step 1.2")])
    );
    assert_eq!(
        reads(),
        1,
        "the second read of the same project asked GitHub nothing"
    );
    // The project a task is moved into, read before the move, so its list is held too.
    assert_eq!(
        project_titles(source.as_ref(), "I_p2").await,
        titled(&[("I_p2t1", "Step 2.1"), ("I_p2t2", "Step 2.2")])
    );

    // Created under it, retitled in it, moved out of it and deleted: each is what the next
    // read of either project reports, without asking GitHub for its tasks again.
    let filed = source
        .write_task(&ItemWrite {
            target: None,
            item: Task {
                project: Some(NativeId("I_p1".to_owned())),
                ..task("ignored", "Step 1.3", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![],
        })
        .await
        .expect("a task this board accepts");
    for (id, title, project) in [
        ("I_p1t1", "Renamed", "I_p1"),
        ("I_p1t2", "Step 1.2", "I_p2"),
    ] {
        source
            .write_task(&ItemWrite {
                target: Some(NativeId(id.to_owned())),
                item: Task {
                    project: Some(NativeId(project.to_owned())),
                    ..task(id, title, status(StatusCategory::Todo, "Todo"))
                },
                depends_on: vec![],
            })
            .await
            .expect("an update this board accepts");
    }
    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[("I_p1t1", "Renamed"), (filed.0.as_str(), "Step 1.3")])
    );
    assert_eq!(
        project_titles(source.as_ref(), "I_p2").await,
        titled(&[
            ("I_p2t1", "Step 2.1"),
            ("I_p2t2", "Step 2.2"),
            ("I_p1t2", "Step 1.2")
        ]),
        "the task moved in is in the project it moved to"
    );
    // Deleted: one the project held before this command, so the answer is the held list's
    // own rather than the record of what this source created.
    source
        .delete_task(&NativeId("I_p1t1".to_owned()))
        .await
        .expect("a task this board holds");
    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[(filed.0.as_str(), "Step 1.3")])
    );
    assert_eq!(
        reads(),
        2,
        "every one of those answers came from the two held lists"
    );

    // A person retitles a task on GitHub. This command does not see it; the next does.
    fixture
        .state
        .lock()
        .unwrap()
        .items
        .iter_mut()
        .find(|item| item.content_id == filed.0)
        .expect("the fixture holds the task")
        .title = "Renamed by a person".to_owned();
    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[(filed.0.as_str(), "Step 1.3")])
    );
    source.end_command().await.expect("a command ends");
    assert_eq!(
        project_titles(source.as_ref(), "I_p1").await,
        titled(&[(filed.0.as_str(), "Renamed by a person")])
    );
    assert_eq!(
        reads(),
        3,
        "the next command read the project's tasks again"
    );
}

#[tokio::test]
async fn one_projects_tasks_come_from_that_project_and_never_from_the_boards_items() {
    // The read this whole shape exists for. A board read is charged for what its nested
    // connections could return rather than for what was asked, so answering "which tasks
    // are in this project?" by reading the board cost the same as answering "which tasks
    // are on this board?" — and this board holds three plans.
    let fixture = board_of(3, 2);
    let source = source(&fixture);

    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_p2".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_p2t1", "I_p2t2"],
        "the tasks filed under that project, and no other project's"
    );
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "a read scoped to one project asked the board for its items"
    );
    assert_eq!(
        fixture.requests("projectTasks"),
        1,
        "the tasks came from the project issue's own sub-issue relationship"
    );
    assert_eq!(
        fixture.searches(),
        Vec::<String>::new(),
        "a project named by its id is resolved from that id, not searched for"
    );
    assert_eq!(
        fixture.documents().len(),
        1,
        "and one request is the whole of what it took"
    );
}

#[tokio::test]
async fn an_item_named_by_its_qualified_id_is_resolved_from_that_id_and_nothing_else() {
    // A qualified id names the item, so nothing is searched for and nothing is walked: the
    // id is resolved, once, and an id this board does not hold is answered as not held
    // rather than by reading the board to discover that.
    let fixture = board_of(3, 2);
    let source = source(&fixture);

    assert_eq!(
        source
            .get_project(&NativeId("I_p2".to_owned()))
            .await
            .expect("the board answers a project read")
            .expect("the board holds that project")
            .title,
        "Plan 2"
    );
    assert_eq!(
        source
            .get_task(&NativeId("I_p2t1".to_owned()))
            .await
            .expect("the board answers a task read")
            .expect("the board holds that task")
            .title,
        "Step 2.1"
    );
    assert_eq!(
        source
            .get_project(&NativeId("I_nothing".to_owned()))
            .await
            .expect("the board answers a read of an id it does not hold"),
        None
    );
    assert_eq!(
        fixture.requests("issue"),
        3,
        "one request per id, and no more"
    );
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
    assert_eq!(fixture.documents().len(), 3);
}

/// One reading of one board issue, as one of the four documents answered for it.
#[derive(Debug, PartialEq)]
struct Reading {
    /// Which document answered, by the name `operation_name` gives it.
    path: &'static str,
    id: String,
    title: String,
    status: Status,
    labels: Vec<String>,
}

impl Reading {
    fn of_project(path: &'static str, project: &Project) -> Self {
        Self {
            path,
            id: project.id.0.clone(),
            title: project.title.clone(),
            status: project.status.clone(),
            labels: project
                .labels
                .iter()
                .map(|label| label.name.clone())
                .collect(),
        }
    }
    fn of_task(path: &'static str, task: &Task) -> Self {
        Self {
            path,
            id: task.id.0.clone(),
            title: task.title.clone(),
            status: task.status.clone(),
            labels: task.labels.iter().map(|label| label.name.clone()).collect(),
        }
    }
}

/// Every way this source reaches an item, driven once each against one board.
///
/// One call per production document, and the verb chosen is the only one that sends it:
/// `SEARCH_ISSUES` is what answers which projects a board holds, `SUB_ISSUES` what answers
/// one project's own tasks, `BOARD` what answers a task read of the whole board, and
/// `ISSUE` what answers an item named by its qualified id. Which kind of item each one can
/// report is the source's own shape rather than this test's choice — the board-scoped
/// search reports projects, and a project's sub-issues and the board's own items report
/// tasks — so the node-id read is taken for both items and every other path is pinned
/// against another reading of the very same issue.
async fn every_way_to_reach(source: &dyn TaskSource, plan: &str, step: &str) -> Vec<Reading> {
    let mut readings = Vec::new();
    for project in source
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .expect("the board lists its projects")
        .items
    {
        readings.push(Reading::of_project("search", &project));
    }
    for task in source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Is(NativeId(plan.to_owned())),
                ..TaskQuery::default()
            },
            &page(10),
        )
        .await
        .expect("the project lists its own tasks")
        .items
    {
        readings.push(Reading::of_task("projectTasks", &task));
    }
    for task in source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists its tasks")
        .items
    {
        readings.push(Reading::of_task("board", &task));
    }
    readings.push(Reading::of_project(
        "issue",
        &source
            .get_project(&NativeId(plan.to_owned()))
            .await
            .expect("the board answers a project read")
            .expect("the board holds that project"),
    ));
    readings.push(Reading::of_task(
        "issue",
        &source
            .get_task(&NativeId(step.to_owned()))
            .await
            .expect("the board answers a task read")
            .expect("the board holds that task"),
    ));
    readings
}

/// Which reading of an item disagrees with the first reading of it, or `None`.
///
/// Grouped by qualified id, so the comparison is always between two readings of one issue.
/// The message names the path that disagreed and what each of the two said, because a
/// reader looking at it needs to know which document to go and read.
fn disagreement(readings: &[Reading]) -> Option<String> {
    let mut by_id: BTreeMap<&str, Vec<&Reading>> = BTreeMap::new();
    for reading in readings {
        by_id.entry(reading.id.as_str()).or_default().push(reading);
    }
    by_id.values().find_map(|group| {
        let (first, rest) = group.split_first().expect("a group holds a reading");
        let said = |reading: &Reading| {
            format!(
                "{:?} / {:?} / {:?}",
                reading.title, reading.status, reading.labels
            )
        };
        rest.iter()
            .find(|other| {
                (&other.title, &other.status, &other.labels)
                    != (&first.title, &first.status, &first.labels)
            })
            .map(|other| {
                format!(
                    "{} read through {} reports {} but through {} reports {}",
                    other.id,
                    other.path,
                    said(other),
                    first.path,
                    said(first)
                )
            })
    })
}

/// A board of one project and its one task, each carrying labels of its own.
///
/// The labels are the point: they are what every one of the four reads has to agree
/// about, and an unlabelled board would let a check that dropped them all pass.
fn equivalence_board(plan: Item, step: Item) -> Fixture {
    board(vec![plan, step])
}

fn labelled_plan() -> Item {
    Item::issue("I_plan", "Delivery plan")
        .sub_issues(1)
        .status("In Progress")
        .labelled(&[("L_bug", "bug"), ("L_team", "team")])
}

fn labelled_step() -> Item {
    Item::issue("I_step", "First step")
        .parent("I_plan")
        .status("Todo")
        .labelled(&[("L_bug", "bug"), ("L_team", "team")])
}

#[tokio::test]
async fn an_item_reports_the_same_labels_title_status_and_id_however_it_is_reached() {
    // No document selects the board's built-in `Labels` field any more — dropping it from
    // the shared fragment is what took `search` and `subIssues` under GitHub's node limit,
    // and dropping it from the board read is what took the board's own cost down. It is
    // only sound because every one of the four reads takes an item's labels from its
    // content, and this is where that is measured.
    let fixture = equivalence_board(labelled_plan(), labelled_step());
    let source = source(&fixture);

    let readings = every_way_to_reach(source.as_ref(), "I_plan", "I_step").await;

    assert_eq!(disagreement(&readings), None);
    assert_eq!(
        readings
            .iter()
            .map(|reading| (reading.path, reading.id.as_str()))
            .collect::<Vec<_>>(),
        [
            ("search", "I_plan"),
            ("projectTasks", "I_step"),
            ("board", "I_step"),
            ("issue", "I_plan"),
            ("issue", "I_step"),
        ],
        "every one of the four documents answered, and each for an issue another of them \
         also answered for"
    );
    assert!(
        readings
            .iter()
            .all(|reading| reading.labels == ["bug", "team"]),
        "an empty label set would let this check pass while proving nothing: {readings:?}"
    );

    assert!(
        fixture
            .documents()
            .iter()
            .all(|document| !document.contains("ProjectV2ItemFieldLabelValue")),
        "and not one of the documents this session sent asked for the board `Labels` field"
    );
}

/// One more board than a read of an issue carries memberships for.
///
/// `BOARD_ITEMS_PAGE_SIZE` is what decides how many memberships ride along on a read, and
/// one more than that ahead of this board's own entry is what puts that entry past the
/// page — the whole case the recovery read exists for. It is read out of
/// `largest_page_sizes`, which binds that constant, rather than written down here, so these
/// cases go on being the case a page really misses however the constant moves. The numbers
/// are other people's boards; nothing about them but their being ahead matters.
fn boards_ahead_of_this_one() -> Vec<u64> {
    let page = u64::from(
        onetaskgraph_github_projects::largest_page_sizes()
            .get("boardItems")
            .copied()
            .expect("this source binds a board-membership page size"),
    );
    (0..=page).map(|offset| 101 + offset).collect()
}

#[tokio::test]
async fn an_item_whose_board_entry_sits_past_the_page_is_still_an_item_of_this_board() {
    // The page of `Issue.projectItems` a read carries is where the search for this board's
    // entry starts, not where it ends: an issue on several boards can have its entry for
    // this one past that page, and before the recovery read that was refused outright. What
    // it has to report now is exactly what every other way of reaching it reports.
    let fixture = equivalence_board(
        labelled_plan().also_on(&boards_ahead_of_this_one()),
        labelled_step().also_on(&boards_ahead_of_this_one()),
    );
    let source = source(&fixture);

    let readings = every_way_to_reach(source.as_ref(), "I_plan", "I_step").await;

    assert_eq!(disagreement(&readings), None);
    assert_eq!(
        readings
            .iter()
            .map(|reading| (reading.path, reading.id.as_str()))
            .collect::<Vec<_>>(),
        [
            ("search", "I_plan"),
            ("projectTasks", "I_step"),
            ("board", "I_step"),
            ("issue", "I_plan"),
            ("issue", "I_step"),
        ],
        "the board-scoped search, the project's sub-issues and the read by qualified id all \
         answered, each for an issue another of them also answered for"
    );
    assert!(
        readings
            .iter()
            .all(|reading| reading.labels == ["bug", "team"]),
        "an empty label set would let this check pass while proving nothing: {readings:?}"
    );
    assert!(
        readings.iter().all(|reading| reading.status.category
            == match reading.id.as_str() {
                "I_plan" => StatusCategory::InProgress,
                _ => StatusCategory::Todo,
            }),
        "and each reading carries the status its board item holds, which is the field the \
         recovered membership entry is what carries: {readings:?}"
    );
    assert!(
        fixture.membership_walks() >= 3,
        "the search, the sub-issue read and the node read each had to recover the entry, \
         and this board was asked {} times",
        fixture.membership_walks()
    );
}

#[tokio::test]
async fn an_issue_whose_whole_membership_connection_names_other_boards_is_not_held_here() {
    // The other half, and the one a walk to exhaustion is what settles: an issue that is
    // really on four other boards and not on this one. The refusal this replaces could not
    // tell it from the case above, because both arrive as a page with no entry for this
    // board and more of the connection to come.
    let fixture = board(vec![
        labelled_plan(),
        labelled_step(),
        Item::issue("I_theirs", "Another board's plan")
            .sub_issues(1)
            .only_on(&boards_ahead_of_this_one()),
        Item::issue("I_their_step", "Another board's step")
            .parent("I_plan")
            .only_on(&boards_ahead_of_this_one()),
    ]);
    let source = source(&fixture);

    // By qualified id: a positive absence, not a failure and not an empty stand-in for one.
    assert_eq!(
        source
            .get_task(&NativeId("I_theirs".to_owned()))
            .await
            .expect("a board that does not hold an issue answers rather than failing"),
        None
    );
    // Through the board-scoped search, which reaches every issue the board's own search
    // returns and resolves each of them the same way.
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .expect("the board lists its projects")
        .items
        .iter()
        .map(|project| project.id.0.clone())
        .collect::<Vec<_>>();
    assert_eq!(projects, ["I_plan"]);
    // And through a project's own sub-issues, the third caller of the same resolver.
    let tasks = source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_plan".to_owned())),
                ..TaskQuery::default()
            },
            &page(10),
        )
        .await
        .expect("the project lists its own tasks")
        .items
        .iter()
        .map(|task| task.id.0.clone())
        .collect::<Vec<_>>();
    assert_eq!(tasks, ["I_step"]);

    assert!(
        fixture.membership_walks() > 0,
        "and it really was decided by reading the rest of the connection"
    );
}

#[tokio::test]
async fn an_issue_this_board_does_not_hold_costs_no_further_request() {
    // The recovery read is only owed to a connection with more of itself to come. An issue
    // whose memberships fit in the page it arrived with is answered from that page alone,
    // so the ordinary case of an id naming another board's issue costs nothing extra — and
    // what proves it is what this board was asked for rather than what came back.
    let fixture = board(vec![
        Item::issue("I_theirs", "Another board's plan").only_on(&[101]),
    ]);
    let source = source(&fixture);

    assert_eq!(
        source
            .get_task(&NativeId("I_theirs".to_owned()))
            .await
            .expect("a board that does not hold an issue answers rather than failing"),
        None
    );

    assert_eq!(
        fixture.membership_walks(),
        0,
        "this board was asked for one issue's memberships although the page it sent \
         reported no more of them: {:?}",
        fixture.documents()
    );
}

#[tokio::test]
async fn a_membership_walk_whose_cursor_does_not_advance_is_refused_rather_than_spun_on() {
    // The recovery walk is a page walk like every other one here, and is held to the same
    // guard: a board answering every page with the cursor it was resumed from would
    // otherwise be walked for ever. The issue is on other boards alone, so the walk keeps
    // asking for the next page rather than stopping at an entry it found.
    let fixture = board(vec![
        Item::issue("I_theirs", "Another board's step").only_on(&boards_ahead_of_this_one()),
    ]);
    fixture.wedge_membership_cursor("stuck");
    let source = source(&fixture);

    let refused = refusal(
        source
            .get_task(&NativeId("I_theirs".to_owned()))
            .await
            .expect_err("a cursor that does not advance is refused"),
    );

    assert!(refused.contains("did not advance"), "{refused}");
    assert!(
        fixture.membership_walks() > 0,
        "and the guard caught it on the walk rather than before it: {refused}"
    );
}

#[tokio::test]
async fn a_membership_page_offering_an_empty_cursor_is_refused_before_the_walk_starts() {
    // The other way a connection offers no progress. This one is caught on the page the
    // read arrived with, so nothing is sent at all.
    let fixture = board(vec![labelled_step().also_on(&boards_ahead_of_this_one())]);
    fixture.wedge_membership_cursor("");
    let source = source(&fixture);

    let refused = refusal(
        source
            .get_task(&NativeId("I_step".to_owned()))
            .await
            .expect_err("an empty cursor is refused"),
    );

    assert!(refused.contains("empty"), "{refused}");
    assert_eq!(
        fixture.membership_walks(),
        0,
        "and no walk was started from a cursor that goes nowhere"
    );
}

/// No document at all selects the board's built-in `Labels` field.
///
/// Over [`graphql::DOCUMENTS`] rather than over a list of names written out here, which is
/// what makes this cover a document nobody thought about:
/// `documents_are_all_inventoried` already fails when a `pub const` in that module is left
/// out of the inventory, so a selection put back into a document later — including one
/// added after this was written — fails here without this test being edited.
#[test]
fn no_document_selects_the_boards_own_labels_field() {
    let selecting = graphql::DOCUMENTS
        .iter()
        .filter(|(document, _)| document.contains("ProjectV2ItemFieldLabelValue"))
        .map(|(_, doing)| *doing)
        .collect::<Vec<_>>();
    assert!(
        selecting.is_empty(),
        "the document for {} selects the board's built-in `Labels` field; GitHub derives \
         that field from the item's content, so it holds nothing the content does not \
         already say and costs a label connection two page sizes deep. Next: select the \
         content's own `labels` instead",
        selecting.join(", ")
    );
}

#[tokio::test]
async fn a_board_item_whose_content_is_a_draft_reports_no_labels_at_all() {
    // The half the board read used to be kept for. `DraftIssue` exposes no `labels` field,
    // and the board's built-in `Labels` field is absent from `ProjectV2CustomFieldType` and
    // unwritable through `ProjectV2FieldValue` — so a draft has nothing to derive one from
    // and cannot carry one. What it reports is an empty set: never a failure, and never the
    // labels of the issue beside it on the same board.
    let fixture = board(vec![
        Item::issue("I_loose", "Loose end").labelled(&[("L_bug", "bug"), ("L_team", "team")]),
        Item::draft("DI_note", "Sketch"),
    ]);
    let source = source(&fixture);

    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists its tasks");

    let labels = |id: &str| {
        tasks
            .items
            .iter()
            .find(|task| task.id.0 == id)
            .unwrap_or_else(|| panic!("the board lists {id}"))
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(labels("DI_note"), Vec::<&str>::new());
    assert_eq!(
        labels("I_loose"),
        ["bug", "team"],
        "and the issue on the same board still reports its own, so the empty set above is \
         the draft's answer rather than this board answering nobody"
    );
}

#[tokio::test]
async fn the_equivalence_check_names_the_path_whose_labels_disagree() {
    // Watched failing, because a check that agrees with itself over every tree is not
    // evidence. This board answers the read of its own items with a label set of its own,
    // which is what a path resolving from the wrong document, or mapping the field wrongly,
    // would look like from outside.
    let fixture = equivalence_board(
        labelled_plan(),
        labelled_step().labels_on("board", &[("L_other", "other")]),
    );
    let source = source(&fixture);

    let readings = every_way_to_reach(source.as_ref(), "I_plan", "I_step").await;

    let failure = disagreement(&readings).expect("the board path disagrees with the other two");
    assert!(
        failure.contains("I_step") && failure.contains("board"),
        "the failure names the item and the path that disagreed: {failure}"
    );
    assert!(
        failure.contains("other") && failure.contains("bug"),
        "and what each of the two said: {failure}"
    );
}

#[tokio::test]
async fn the_work_one_projects_read_does_is_that_projects_size_and_not_the_boards() {
    // The property the cost claim rests on, asserted the only way it can be: the same read
    // against two boards of very different size, and what it asked for compared.
    let small = board_of(2, 2);
    let large = board_of(12, 2);
    let scoped = TaskQuery {
        project: ProjectFilter::Is(NativeId("I_p2".to_owned())),
        ..TaskQuery::default()
    };

    let from_small = selected_tasks(source(&small).as_ref(), &scoped).await;
    let from_large = selected_tasks(source(&large).as_ref(), &scoped).await;

    assert_eq!(from_small, ["I_p2t1", "I_p2t2"]);
    assert_eq!(
        from_large, from_small,
        "the same project of a board six times the size answers the same"
    );
    assert_eq!(
        large.documents(),
        small.documents(),
        "and asked for exactly the same thing to do it"
    );
    assert_eq!(large.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn the_projects_a_board_holds_come_from_a_search_scoped_to_it_and_not_from_its_items() {
    // An orphan task is on this board too, so `parent` really is doing the work of telling
    // a project from a task: GitHub accepts `-has:parent` as a search qualifier and
    // silently ignores it, which is why the discriminator cannot live in the search.
    let mut items = board_of(2, 1);
    items = {
        let mut all = items.state.lock().unwrap().items.clone();
        all.push(Item::issue("I_loose", "Sweep the backlog").status("Todo"));
        drop(items);
        board(all)
    };
    let source = source(&items);

    assert_eq!(
        selected_projects(source.as_ref(), &ProjectQuery::default()).await,
        ["I_p1", "I_p2"],
        "the issues with no parent and sub-issues of their own, and not the loose one"
    );
    assert_eq!(
        items.board_item_reads(),
        Vec::<String>::new(),
        "listing the board's projects asked the board for its items"
    );
    assert_eq!(
        items.searches(),
        ["project:octo-org/7 is:issue"],
        "one search, scoped to the configured board"
    );
}

#[tokio::test]
async fn a_project_named_by_name_is_found_by_one_bounded_search_that_filters_at_the_server() {
    // A selector GitHub cannot resolve as a node id is a project *name*. Discovering it
    // must not become a walk of the board, so the name goes into the search as a qualifier
    // and the server does the narrowing.
    let fixture = board_of(3, 2);
    let by_name = source(&fixture);

    assert_eq!(
        selected_tasks(
            by_name.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("Plan 2".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        ["I_p2t1", "I_p2t2"],
        "a project named by its name answers with its own tasks"
    );
    assert_eq!(
        fixture.searches(),
        ["project:octo-org/7 is:issue in:title \"Plan 2\""],
        "one bounded search, filtering on that name at the server"
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());

    // And a name nothing on this board carries selects nothing rather than everything.
    let other = board_of(3, 2);
    let elsewhere = source(&other);
    assert_eq!(
        selected_tasks(
            elsewhere.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("Plan 9".to_owned())),
                ..TaskQuery::default()
            },
        )
        .await,
        Vec::<String>::new()
    );
    assert_eq!(other.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_read_taken_straight_after_a_write_answers_with_what_was_written() {
    // GitHub's issue search is an index and is eventually consistent, so it cannot supply
    // this: a project written a moment ago is routinely absent from the very next search.
    // What supplies it is this source's own record of what it wrote, which every read is
    // completed from. `read_behind` is that index being behind, and nothing else here
    // changes.
    let fixture = board_of(1, 0);
    let source = source(&fixture);

    let created = source
        .write_project(&write(project(
            "ignored",
            "Second plan",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a project this board accepts");
    fixture.read_behind(1);

    assert_eq!(
        selected_projects(source.as_ref(), &ProjectQuery::default()).await,
        ["I_p1", created.0.as_str()],
        "the project this run wrote is reported though the search cannot see it yet"
    );
    assert!(
        !fixture.searches().is_empty(),
        "and the search really was the discovery path"
    );

    let filed = source
        .write_task(&ItemWrite {
            target: None,
            item: Task {
                project: Some(created.clone()),
                ..task(
                    "ignored",
                    "First step",
                    status(StatusCategory::Todo, "Todo"),
                )
            },
            depends_on: vec![],
        })
        .await
        .expect("a task this board accepts");
    fixture.read_behind(2);

    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(created.clone()),
                ..TaskQuery::default()
            },
        )
        .await,
        [filed.0.as_str()],
        "and so is the task this run filed under it"
    );
    assert_eq!(
        source
            .get_project(&created)
            .await
            .expect("the board answers a project read")
            .expect("the project this run wrote")
            .title,
        "Second plan"
    );
}

/// A whole-board read reports an item `ProjectV2.items` has not caught up with.
///
/// Every read here is through a source that did none of the writing, which is what makes it
/// about GitHub's data rather than about `GitHubProjectsSource::created`: that record is
/// empty in a source built after the write, and a correction it could answer would have
/// deleted the property the credentialed journey's own wait exists for.
#[tokio::test]
async fn a_board_read_reports_an_item_the_boards_own_item_connection_is_behind_on() {
    let fixture = board_of(1, 1);
    fixture.items_connection_falls_behind();

    let writer = source(&fixture);
    let created = writer
        .write_task(&ItemWrite {
            target: None,
            item: task(
                "ignored",
                "Third step",
                status(StatusCategory::Todo, "Todo"),
            ),
            depends_on: vec![],
        })
        .await
        .expect("a task this board accepts");

    let reader = source(&fixture);
    let held = selected_tasks(reader.as_ref(), &TaskQuery::default()).await;
    assert!(
        held.contains(&created.0),
        "a source that did none of the writing did not report the item the board was \
         given: {held:?}"
    );
    assert!(
        !fixture.searches().is_empty(),
        "and the board's own issue search really was the discovery path"
    );

    // The other half, so the assertion above is about the search rather than about anything
    // this source kept: hold the search back over the same item and the same read reports
    // it no longer. Nothing but the board changed.
    fixture.read_behind(1);
    let blind = source(&fixture);
    assert!(
        !selected_tasks(blind.as_ref(), &TaskQuery::default())
            .await
            .contains(&created.0),
        "with both of GitHub's enumerations behind there is nothing left to answer from"
    );
}

/// What this run writes to an item only the board's search reported reaches the read.
///
/// The half a source keeps of that search is a third place one item can sit, beside the
/// board it read and what it created — and for an item the board's own item connection is
/// behind on it is the *only* place, so a write or a delete that missed it would put the
/// stale record back on exactly the items the union in `GitHubProjectsSource::board` exists
/// for. Both halves are driven here through one source's own reads.
#[tokio::test]
async fn a_write_and_a_delete_reach_an_item_only_the_boards_search_reported() {
    async fn titled(source: &dyn TaskSource) -> Vec<String> {
        let mut held = source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect("the board answers a task query")
            .items
            .into_iter()
            .map(|task| task.title)
            .collect::<Vec<_>>();
        held.sort();
        held
    }

    let fixture = board_of(1, 1);
    fixture.items_connection_falls_behind();
    let created = source(&fixture)
        .write_task(&ItemWrite {
            target: None,
            item: task(
                "ignored",
                "Third step",
                status(StatusCategory::Todo, "Todo"),
            ),
            depends_on: vec![],
        })
        .await
        .expect("a task this board accepts");

    // A source that did none of the writing, whose first read is what fills its view of
    // this board — the item connection in one half and the search in the other.
    let reader = source(&fixture);
    assert_eq!(
        titled(reader.as_ref()).await,
        ["Step 1.1", "Third step"],
        "the first read reports the item only the search knows about"
    );

    reader
        .write_task(&ItemWrite {
            target: Some(created.clone()),
            item: task(
                "ignored",
                "Third step, revised",
                status(StatusCategory::Todo, "Todo"),
            ),
            depends_on: vec![],
        })
        .await
        .expect("an update of an item this board holds");
    assert_eq!(
        titled(reader.as_ref()).await,
        ["Step 1.1", "Third step, revised"],
        "and reports the title this run just wrote onto it rather than the one it read"
    );

    reader
        .delete_task(&created)
        .await
        .expect("a delete of an item this board holds");
    assert_eq!(
        titled(reader.as_ref()).await,
        ["Step 1.1"],
        "and stops reporting it once this run has taken it off"
    );
}

/// A board read whose search fails says so, and the next read asks the search again.
///
/// The union in `GitHubProjectsSource::board` made the search a half of every whole-board
/// read, where before it answered the project list alone — so a search that fails is a new
/// way for a board read to fail, and what it must never do is answer with the half that did
/// work. An item only the search reports is exactly the item the union exists for, so a read
/// silently missing it would be the defect this correction was written to remove, arriving
/// by another route.
///
/// The second half is the one a cache makes possible: the source holds the search for the
/// length of a command, and a failed walk that got written down there would be permanent for
/// that command. The refusal is a single scripted 503 — the shape a publication of this
/// repository was really refused in — so the same source, asked again, has a working search
/// to reach.
#[tokio::test]
async fn a_board_read_whose_search_fails_is_refused_rather_than_answered_short() {
    let fixture = board_of(1, 1);
    // Only the search can report what this run is about to write; see
    // `State::items_connection_behind_from`.
    fixture.items_connection_falls_behind();
    let created = source(&fixture)
        .write_task(&ItemWrite {
            target: None,
            item: task(
                "ignored",
                "Third step",
                status(StatusCategory::Todo, "Todo"),
            ),
            depends_on: vec![],
        })
        .await
        .expect("a task this board accepts");

    fixture.script_for("search", vec![Refusal::unavailable()]);

    // A source that did none of the writing, so the only place the item above sits is the
    // search this board is about to refuse.
    let reader = source(&fixture);
    let message = refusal(
        reader
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a board read whose search failed is a failure, not a short answer"),
    );
    assert!(
        message.contains("503"),
        "and says what GitHub answered rather than something of its own: {message}"
    );

    // The scripted refusal is spent, so the very same source asking again reaches a working
    // search — which it only does if the failed walk was not kept.
    let held = selected_tasks(reader.as_ref(), &TaskQuery::default()).await;
    assert!(
        held.contains(&created.0),
        "the retried search reports the item the board's own item connection is behind on, \
         and it is the very item this run created: {held:?}"
    );
    // Counted by arrival, refused one included: the write above lists nothing, so the two
    // are the refused walk and its retry.
    assert_eq!(
        fixture.requests("search"),
        2,
        "and the second read really did ask the search again rather than answer from a \
         record of the walk that failed"
    );
}

#[tokio::test]
async fn a_board_of_more_than_one_page_of_projects_and_of_tasks_is_read_completely() {
    // GitHub caps a connection page at 100, so a board holding more projects than that —
    // or a project holding more tasks than that — is only read completely if both walks
    // page. Neither walk is the caller's paging: the caller's page is cut from the answer
    // afterwards.
    let fixture = board_of(140, 0);
    let many = source(&fixture);
    let listed = selected_projects(many.as_ref(), &ProjectQuery::default()).await;
    assert_eq!(listed.len(), 10, "the caller asked for a page of ten");
    assert!(
        fixture.requests("search") >= 2,
        "a board of 140 projects is two pages of GitHub's own maximum"
    );

    let plan = board_of(1, 140);
    let one_plan = source(&plan);
    let scoped = TaskQuery {
        project: ProjectFilter::Is(NativeId("I_p1".to_owned())),
        ..TaskQuery::default()
    };
    let held = one_plan
        .query_tasks(&scoped, &page(100))
        .await
        .expect("the board answers a task query");
    assert_eq!(held.items.len(), 100);
    assert!(
        held.next.is_some(),
        "and the caller is told there is more of it"
    );
    let rest = one_plan
        .query_tasks(&scoped, &resume(&held.next.unwrap().0, 100))
        .await
        .expect("the board answers the rest");
    assert_eq!(rest.items.len(), 40);
    assert!(rest.next.is_none());
    assert!(
        plan.requests("projectTasks") >= 2,
        "a project of 140 tasks is two pages of sub-issues"
    );
    assert_eq!(plan.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn every_predicate_a_task_query_carries_is_applied() {
    let source = committed_board();
    let query =
        |labels: LabelFilter, statuses: Vec<StatusCategory>, text: Option<TextQuery>| TaskQuery {
            text,
            labels,
            statuses,
            project: ProjectFilter::Any,
            priorities: Vec::new(),
            commented_since: None,
            metadata: Vec::new(),
            origin: None,
        };
    let none = LabelFilter::default();

    // `I_loose` carries `chore` and `I_task` carries `bug`, so a label filter that is
    // applied and one that is dropped answer with different rows. Under a board where both
    // carried `bug` the two answers were the same list.
    for (expected, query) in [
        (
            vec!["I_task"],
            query(label_filter(&["bug"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_loose"],
            query(label_filter(&["bug", "chore"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_notes"],
            query(label_filter(&["bug", "docs"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task"],
            query(label_filter(&[], &["bug", "team"], &[]), vec![], None),
        ),
        (
            vec!["I_notes", "I_loose"],
            query(label_filter(&[], &[], &["bug"]), vec![], None),
        ),
        (
            vec![],
            query(label_filter(&["bug"], &[], &["bug"]), vec![], None),
        ),
        // Names match case-insensitively, the way the local Markdown source matches them.
        (
            vec!["I_task"],
            query(label_filter(&["BUG"], &[], &[]), vec![], None),
        ),
        (
            vec!["I_task", "I_loose"],
            query(none.clone(), vec![StatusCategory::Todo], None),
        ),
        (
            vec!["I_notes"],
            query(none.clone(), vec![StatusCategory::InProgress], None),
        ),
        (
            vec!["I_task", "I_notes", "I_loose"],
            query(
                none.clone(),
                vec![StatusCategory::Todo, StatusCategory::InProgress],
                None,
            ),
        ),
        (
            vec!["I_loose"],
            query(none.clone(), vec![], text("sweep", TextFields::Title)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("filed", TextFields::Title)),
        ),
        (
            vec!["I_loose"],
            query(none.clone(), vec![], text("filed", TextFields::Content)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("sweep", TextFields::Content)),
        ),
        (
            vec!["I_notes"],
            query(
                none.clone(),
                vec![],
                text("QUARTER", TextFields::TitleOrContent),
            ),
        ),
        (
            vec!["I_task"],
            query(none.clone(), vec![], text("sHIP", TextFields::Title)),
        ),
        // Every predicate at once narrows rather than widens.
        (
            vec!["I_loose"],
            TaskQuery {
                text: text("work", TextFields::Content),
                labels: label_filter(&["chore"], &[], &["team"]),
                statuses: vec![StatusCategory::Todo],
                project: ProjectFilter::Orphans,
                priorities: Vec::new(),
                commented_since: None,
                metadata: Vec::new(),
                origin: None,
            },
        ),
    ] {
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            expected,
            "{query:?}"
        );
    }
}

#[tokio::test]
async fn every_predicate_a_project_query_carries_is_applied() {
    let fixture = board(vec![
        Item::issue("P_engine", "Engine plan")
            .body("runtime work")
            .status("Todo")
            .sub_issues(1)
            .labelled(&[("L_core", "core")]),
        Item::issue("P_docs", "Docs plan")
            .body("prose work")
            .status("In Progress")
            .sub_issues(1)
            .labelled(&[("L_chore", "chore")]),
        Item::issue("I_task", "a task").status("Todo"),
    ]);
    let source = source(&fixture);
    let query = |labels: LabelFilter, statuses: Vec<StatusCategory>, text: Option<TextQuery>| {
        ProjectQuery {
            text,
            labels,
            statuses,
        }
    };
    let none = LabelFilter::default();

    for (expected, query) in [
        (
            vec!["P_engine", "P_docs"],
            query(none.clone(), vec![], None),
        ),
        (
            vec!["P_engine"],
            query(label_filter(&["core"], &[], &[]), vec![], None),
        ),
        (
            vec![],
            query(label_filter(&[], &["core", "chore"], &[]), vec![], None),
        ),
        (
            vec!["P_docs"],
            query(label_filter(&[], &[], &["core"]), vec![], None),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![StatusCategory::InProgress], None),
        ),
        (
            vec!["P_engine", "P_docs"],
            query(
                none.clone(),
                vec![StatusCategory::Todo, StatusCategory::InProgress],
                None,
            ),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![], text("docs", TextFields::Title)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("prose", TextFields::Title)),
        ),
        (
            vec!["P_docs"],
            query(none.clone(), vec![], text("prose", TextFields::Content)),
        ),
        (
            vec![],
            query(none.clone(), vec![], text("docs", TextFields::Content)),
        ),
        (
            vec!["P_engine", "P_docs"],
            query(
                none.clone(),
                vec![],
                text("work", TextFields::TitleOrContent),
            ),
        ),
        (
            vec!["P_docs"],
            ProjectQuery {
                text: text("plan", TextFields::Title),
                labels: label_filter(&["chore"], &[], &[]),
                statuses: vec![StatusCategory::InProgress],
            },
        ),
    ] {
        assert_eq!(
            selected_projects(source.as_ref(), &query).await,
            expected,
            "{query:?}"
        );
    }
}

#[tokio::test]
async fn a_filtered_task_or_project_result_is_paged_after_it_is_filtered() {
    // Paging the board and then filtering the page would answer this walk with one item
    // and then stop: `I_2` would consume the first page and leave nothing in it.
    let fixture = board(
        (1..=5)
            .map(|n| {
                let item = Item::issue(&format!("I_{n}"), "step").status("Todo");
                if n % 2 == 1 {
                    item.labelled(&[("L_keep", "keep")])
                } else {
                    item.labelled(&[("L_drop", "drop")])
                }
            })
            .chain((1..=3).map(|n| {
                let item = Item::issue(&format!("P_{n}"), "plan")
                    .status("Todo")
                    .sub_issues(1);
                if n % 2 == 1 {
                    item.labelled(&[("L_keep", "keep")])
                } else {
                    item.labelled(&[("L_drop", "drop")])
                }
            }))
            .collect(),
    );
    let source = source(&fixture);
    let query = TaskQuery {
        labels: label_filter(&["keep"], &[], &[]),
        ..TaskQuery::default()
    };

    let first = source.query_tasks(&query, &page(2)).await.unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_1", "I_3"],
        "a page of a filtered result is a page of the survivors"
    );
    assert!(first.next.is_some(), "a third survivor is still owed");

    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let answered = source.query_tasks(&query, &request).await.unwrap();
        walked.extend(answered.items.into_iter().map(|task| task.id.0));
        match answered.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        walked,
        ["I_1", "I_3", "I_5"],
        "a walk to exhaustion returns every survivor exactly once in a stable order"
    );

    let projects = ProjectQuery {
        labels: label_filter(&["keep"], &[], &[]),
        ..ProjectQuery::default()
    };
    let first = source.query_projects(&projects, &page(1)).await.unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|project| project.id.0.as_str())
            .collect::<Vec<_>>(),
        ["P_1"]
    );
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let answered = source.query_projects(&projects, &request).await.unwrap();
        walked.extend(answered.items.into_iter().map(|project| project.id.0));
        match answered.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        walked,
        ["P_1", "P_3"],
        "a page smaller than the surviving projects walks to exhaustion over survivors"
    );
}

#[tokio::test]
async fn a_pull_request_is_neither_a_project_nor_a_task() {
    // A behaviour change: this source used to map every `ProjectV2Item` content shape it
    // recognised into a task, so a pull request on the board was listed as one. A pull
    // request is somebody's change rather than a unit of plan, and it now appears in
    // neither listing and cannot be fetched by either id.
    let fixture = board(vec![
        Item::issue("I_1", "a task"),
        Item::pull_request("PR_1"),
    ]);
    let source = source(&fixture);
    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap();
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_1"]
    );
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        source
            .get_task(&NativeId("PR_1".to_owned()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        source
            .get_project(&NativeId("PR_1".to_owned()))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn every_arm_of_the_project_or_task_rule_decides_the_same_way() {
    let marker = |kind: &str| {
        format!("<!-- onetaskgraph.metadata\n{{\"onetaskgraph.item_kind\":\"{kind}\"}}\n-->")
    };
    let fixture = board(vec![
        // Sub-issues and no marker: a project a person authored by hand.
        Item::issue("I_subs", "authored plan").sub_issues(2),
        // A marker and no sub-issues: the empty project a copy passes through.
        Item::issue("I_marked", "empty plan").body(&marker("project")),
        // Neither: an ordinary task.
        Item::issue("I_plain", "a task"),
        // A sub-issue that has sub-issues of its own AND claims to be a project. Being a
        // sub-issue wins, and no marker overrides it.
        Item::issue("I_deep", "a deep task")
            .parent("I_subs")
            .sub_issues(3)
            .body(&marker("project")),
        // A marker saying `task` carries no information the sub-issue rules did not
        // already decide, so an unmarked-looking task stays a task.
        Item::issue("I_said_task", "a marked task").body(&marker("task")),
    ]);
    let source = source(&fixture);
    assert_eq!(
        source
            .query_projects(&ProjectQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|project| project.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_subs", "I_marked"]
    );
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|task| task.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_plain", "I_deep", "I_said_task"]
    );
    assert_eq!(
        source
            .get_task(&NativeId("I_deep".to_owned()))
            .await
            .unwrap()
            .expect("a sub-issue is a task")
            .project,
        Some(NativeId("I_subs".to_owned()))
    );
}

#[tokio::test]
async fn a_malformed_kind_marker_is_refused_by_name() {
    let fixture = board(vec![Item::issue("I_1", "a task").body(
        "<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"epic\"}\n-->",
    )]);
    let message = refusal(
        source(&fixture)
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a marker this contract cannot read is refused"),
    );
    assert!(message.contains("onetaskgraph.item_kind"), "{message}");
    assert!(message.contains("I_1"), "{message}");
}

#[tokio::test]
async fn unbounded_caller_metadata_and_long_prose_round_trip_through_the_body_slot() {
    // The board's own `shortDescription` is capped at 300 characters and a project text
    // field is length-bounded, which is why neither is where this goes.
    let goal = "g".repeat(400);
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = project(
        "P-source",
        "Published roadmap",
        status(StatusCategory::Todo, "Todo"),
    );
    item.content = Some(goal.clone());
    item.metadata = BTreeMap::from([
        ("caller.shape".to_owned(), json!({"nested":[1, true, null]})),
        ("caller.number".to_owned(), json!(3.5)),
        ("caller.text".to_owned(), json!("plain")),
        ("onepipeline.steps".to_owned(), json!(["a".repeat(500)])),
    ]);
    let written = source
        .write_project(&write(item.clone()))
        .await
        .expect("a project longer than any GitHub text field copies");
    let read = source
        .get_project(&written)
        .await
        .unwrap()
        .expect("the created project reads back");
    assert_eq!(read.content.as_deref(), Some(goal.as_str()));
    assert_eq!(read.metadata, item.metadata);
    assert!(
        fixture.item(&written.0).body.unwrap().ends_with("\n-->"),
        "the slot is a trailing Markdown comment, which GitHub does not render"
    );
}

#[tokio::test]
async fn a_project_with_ten_kilobytes_of_caller_metadata_copies_into_its_body_slot_and_reads_back_equal()
 {
    // About ten kilobytes of JSON under one caller key — a plan's budget answers — beside a
    // rendering's provenance, with every character the slot has to carry. GitHub caps an issue
    // body at 65,536 characters, so this fits whole, and it must come back whole.
    let budgets = (0..60)
        .map(|index| {
            json!({
                "issue": format!("plan:T-{index}"),
                "tokens": 120_000 + index,
                "note": format!(
                    "Budget {index}: \"quoted\", back\\slash, <tag> & `tick` --> naïve café — {}",
                    "x".repeat(40)
                ),
            })
        })
        .collect::<Vec<_>>();
    let answers = json!({"version": 3, "budgets": budgets});
    let encoded = serde_json::to_string(&answers).unwrap();
    assert!(
        (9_500..20_000).contains(&encoded.len()),
        "about ten kilobytes of JSON: {}",
        encoded.len()
    );
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = project("P-source", "The plan", status(StatusCategory::Todo, "Todo"));
    item.content = Some("The plan's description.".to_owned());
    item.metadata = BTreeMap::from([
        ("onepipeline.budgets".to_owned(), answers.clone()),
        (
            "onetaskgraph.template".to_owned(),
            json!({"template": "plan-description", "digest": format!("sha256:{}", "a".repeat(64)),
                   "body_digest": format!("sha256:{}", "b".repeat(64)),
                   "answers_digest": format!("sha256:{}", "c".repeat(64))}),
        ),
    ]);
    let written = source
        .write_project(&write(item.clone()))
        .await
        .expect("a project carrying this much metadata copies");
    let body = fixture.item(&written.0).body.unwrap();
    assert!(body.len() > encoded.len(), "the whole value is in the body");
    let read = source
        .get_project(&written)
        .await
        .unwrap()
        .expect("the created project reads back");
    assert_eq!(read.content.as_deref(), Some("The plan's description."));
    assert_eq!(read.metadata, item.metadata);

    // A second key of the same size, set later through the narrow write, keeps the first.
    let key = MetadataKey::new("onepipeline.budgets_previous").unwrap();
    let after = source
        .set_project_metadata(&written, &key, &answers)
        .await
        .unwrap()
        .expect("the project is held");
    assert_eq!(after.metadata.get(key.as_str()), Some(&answers));
    assert_eq!(after.metadata.get("onepipeline.budgets"), Some(&answers));
}

#[tokio::test]
async fn a_comment_that_is_not_at_the_end_is_the_authors_own_content() {
    let fixture = board(vec![Item::issue("I_1", "a task").body(
        "<!-- onetaskgraph.metadata\n{\"caller.x\":1}\n-->\n\nand then more prose",
    )]);
    let held = source(&fixture)
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert!(held.metadata.is_empty());
    assert!(held.content.unwrap().contains("more prose"));
}

#[tokio::test]
async fn a_slot_this_source_cannot_read_is_refused_rather_than_dropped() {
    for (body, problem) in [
        (
            "<!-- onetaskgraph.metadata\n{\"caller.x\":1}",
            "unterminated",
        ),
        ("<!-- onetaskgraph.metadata\nnot json\n-->", "invalid"),
    ] {
        let fixture = board(vec![Item::issue("I_1", "a task").body(body)]);
        let message = refusal(
            source(&fixture)
                .get_task(&NativeId("I_1".to_owned()))
                .await
                .expect_err("a slot this source cannot read is refused"),
        );
        assert!(message.contains(problem), "{message}");
    }
}

#[tokio::test]
async fn a_write_without_a_configured_repository_is_refused_naming_the_field() {
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"repository": null}));
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a board has no repository of its own"),
    );
    assert!(message.contains("repository"), "{message}");
    assert!(message.contains("owner/name"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    // An item whose own single repository would place it is refused the same way: the
    // configured repository is required for a write exactly as before the item's own
    // field decided where an issue is created.
    let placed = refusal(
        source
            .write_task(&task_under(None, "Placed", &["acme/tooling"]))
            .await
            .expect_err("the configured repository is required whatever the item names"),
    );
    assert_eq!(placed, message);
    assert!(
        fixture.seen().is_empty(),
        "nothing is written before the refusal"
    );
}

#[tokio::test]
async fn a_repository_the_token_cannot_see_is_refused_by_name() {
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"repository":"acme/missing"}));
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a repository nothing resolves is refused"),
    );
    assert!(message.contains("acme"), "{message}");
    assert!(message.contains("missing"), "{message}");
}

#[tokio::test]
async fn repositories_are_derived_from_the_issue_and_recorded_only_when_they_differ() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let own = Repository::try_from("github.com/acme/work".to_owned()).unwrap();
    let elsewhere = Repository::try_from("github.com/acme/other".to_owned()).unwrap();

    let mut derived = task("T-1", "Derived", status(StatusCategory::Todo, "Todo"));
    derived.repositories = vec![own.clone()];
    let derived_id = source.write_task(&write(derived)).await.unwrap();
    assert!(
        !fixture
            .item(&derived_id.0)
            .body
            .unwrap_or_default()
            .contains(Repository::METADATA_KEY),
        "a list that is exactly the issue's own repository is derived, never written down"
    );
    assert_eq!(
        source
            .get_task(&derived_id)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![own.clone()]
    );

    let mut recorded = task("T-2", "Recorded", status(StatusCategory::Todo, "Todo"));
    recorded.repositories = vec![elsewhere.clone(), own.clone()];
    let recorded_id = source.write_task(&write(recorded)).await.unwrap();
    assert!(
        fixture
            .item(&recorded_id.0)
            .body
            .unwrap()
            .contains(Repository::METADATA_KEY)
    );
    assert_eq!(
        source
            .get_task(&recorded_id)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![elsewhere, own],
        "a plan node naming its own repositories is reported as it named them"
    );
}

fn repo(slug: &str) -> Repository {
    Repository::try_from(format!("github.com/{slug}")).unwrap()
}

fn created_in(fixture: &Fixture) -> Vec<String> {
    fixture
        .seen()
        .into_iter()
        .filter(|call| call[0] == "createIssue")
        .map(|call| {
            call[1]["repositoryId"]
                .as_str()
                .and_then(|id| id.strip_prefix(REPOSITORY_NODE_PREFIX))
                .expect("createIssue names a repository this board resolved")
                .to_owned()
        })
        .collect()
}

fn task_under(parent: Option<&str>, title: &str, repositories: &[&str]) -> ItemWrite<Task> {
    write(Task {
        project: parent.map(|id| NativeId(id.to_owned())),
        repositories: repositories.iter().map(|slug| repo(slug)).collect(),
        ..task("ignored", title, status(StatusCategory::Todo, "Todo"))
    })
}

fn document_under(parent: Option<&str>, title: &str, repositories: &[&str]) -> ItemWrite<Document> {
    write(Document {
        project: parent.map(|id| NativeId(id.to_owned())),
        repositories: repositories.iter().map(|slug| repo(slug)).collect(),
        ..document("ignored", title)
    })
}

fn project_naming(title: &str, repositories: &[&str]) -> ItemWrite<Project> {
    write(Project {
        repositories: repositories.iter().map(|slug| repo(slug)).collect(),
        ..project("ignored", title, status(StatusCategory::Todo, "Todo"))
    })
}

#[tokio::test]
async fn a_task_naming_one_repository_is_created_there_as_a_sub_issue_of_its_project() {
    // The configured repository is acme/work and so is the project issue's; the task names
    // acme/tooling, another repository of the same owner, which is where a person changing
    // that repository looks for it.
    let fixture = board(vec![Item::issue("I_plan", "Plan").sub_issues(1)]);
    let source = source(&fixture);

    let filed = source
        .write_task(&task_under(
            Some("I_plan"),
            "Change tooling",
            &["acme/tooling"],
        ))
        .await
        .expect("a task naming one repository of the same owner is created there");

    assert_eq!(created_in(&fixture), ["acme/tooling"]);
    assert!(
        fixture.seen().iter().any(|call| call[0] == "addSubIssue"
            && call[1]["issueId"] == "I_plan"
            && call[1]["subIssueId"] == filed.0.as_str()),
        "and it is filed as a sub-issue of its project: {:?}",
        fixture.seen()
    );
    let held = fixture.item(&filed.0);
    assert_eq!(held.repository.as_deref(), Some("acme/tooling"));
    assert!(
        !held
            .body
            .unwrap_or_default()
            .contains(Repository::METADATA_KEY),
        "the one repository is where the issue lives, so it is derived rather than recorded"
    );
    let read = source.get_task(&filed).await.unwrap().unwrap();
    assert_eq!(read.repositories, vec![repo("acme/tooling")]);
    assert_eq!(read.project, Some(NativeId("I_plan".to_owned())));
}

#[tokio::test]
async fn a_task_naming_none_or_several_is_created_in_its_projects_repository() {
    // The project issue is in acme/tooling — not the configured acme/work — as its own
    // single entry would have put it, so where the task lands is the parent's repository
    // and not the fallback.
    let fixture = board(vec![
        Item::issue("I_plan", "Plan")
            .sub_issues(1)
            .in_repository("acme/tooling"),
    ]);
    let source = source(&fixture);

    let none = source
        .write_task(&task_under(Some("I_plan"), "Unplaced", &[]))
        .await
        .expect("a task naming no repository is created in its project's");
    let several = source
        .write_task(&task_under(
            Some("I_plan"),
            "Spanning",
            &["acme/work", "acme/tooling"],
        ))
        .await
        .expect("a task naming several is created in its project's");

    assert_eq!(created_in(&fixture), ["acme/tooling", "acme/tooling"]);
    let none_held = fixture.item(&none.0);
    assert_eq!(none_held.repository.as_deref(), Some("acme/tooling"));
    assert!(
        none_held
            .body
            .unwrap()
            .contains(&format!("{:?}:[]", Repository::METADATA_KEY)),
        "an empty list is recorded, or the read would derive the project's repository"
    );
    assert_eq!(
        source.get_task(&none).await.unwrap().unwrap().repositories,
        Vec::<Repository>::new()
    );
    assert!(
        fixture
            .item(&several.0)
            .body
            .unwrap()
            .contains(Repository::METADATA_KEY)
    );
    assert_eq!(
        source
            .get_task(&several)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![repo("acme/work"), repo("acme/tooling")],
        "several are recorded in the order they were named"
    );
}

#[tokio::test]
async fn a_parent_created_earlier_in_the_command_places_its_tasks_from_this_processs_own_record() {
    // GitHub's board read is eventually consistent, so the project this command just
    // created is not on the board it reads back; the parent's repository comes from what
    // this process remembers creating.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&project_naming("Tooling plan", &["acme/tooling"]))
        .await
        .expect("a project naming one repository is created there");
    fixture.read_behind(1);

    let none = source
        .write_task(&task_under(Some(&plan.0), "Unplaced", &[]))
        .await
        .expect("the parent is known from this process's own record");
    fixture.read_behind(2);
    let several = source
        .write_task(&task_under(
            Some(&plan.0),
            "Spanning",
            &["acme/work", "acme/tooling"],
        ))
        .await
        .expect("the parent is still known");

    assert_eq!(
        created_in(&fixture),
        ["acme/tooling", "acme/tooling", "acme/tooling"],
        "the project went where its one entry said, and both tasks followed it"
    );
    assert_eq!(
        source.get_task(&none).await.unwrap().unwrap().repositories,
        Vec::<Repository>::new()
    );
    assert_eq!(
        source
            .get_task(&several)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![repo("acme/work"), repo("acme/tooling")]
    );
}

#[tokio::test]
async fn a_project_and_an_unparented_item_go_to_their_one_repository_else_the_configured_one() {
    let fixture = board(vec![]);
    let source = source(&fixture);

    source
        .write_project(&project_naming("One", &["acme/tooling"]))
        .await
        .expect("a project naming one repository is created there");
    source
        .write_project(&project_naming("None", &[]))
        .await
        .expect("a project naming none is created in the configured repository");
    source
        .write_project(&project_naming("Several", &["acme/tooling", "acme/other"]))
        .await
        .expect("a project naming several is created in the configured repository");
    source
        .write_task(&task_under(None, "Orphan one", &["acme/tooling"]))
        .await
        .expect("a task with no parent naming one repository is created there");
    source
        .write_task(&task_under(None, "Orphan none", &[]))
        .await
        .expect("a task with no parent naming none falls back");
    source
        .write_task(&task_under(
            None,
            "Orphan several",
            &["acme/tooling", "acme/other"],
        ))
        .await
        .expect("a task with no parent naming several falls back");
    source
        .write_document(&document_under(None, "Loose one", &["acme/tooling"]))
        .await
        .expect("a document with no parent naming one repository is created there");
    source
        .write_document(&document_under(None, "Loose none", &[]))
        .await
        .expect("a document with no parent naming none falls back");

    assert_eq!(
        created_in(&fixture),
        [
            "acme/tooling",
            "acme/work",
            "acme/work",
            "acme/tooling",
            "acme/work",
            "acme/work",
            "acme/tooling",
            "acme/work",
        ]
    );
}

#[tokio::test]
async fn a_document_follows_the_task_rule() {
    let fixture = board(vec![
        Item::issue("I_plan", "Plan")
            .sub_issues(1)
            .in_repository("acme/tooling"),
    ]);
    let source = source(&fixture);

    let one = source
        .write_document(&document_under(Some("I_plan"), "Design", &["acme/other"]))
        .await
        .expect("a document naming one repository is created there");
    let none = source
        .write_document(&document_under(Some("I_plan"), "Notes", &[]))
        .await
        .expect("a document naming none is created in its project's repository");
    let several = source
        .write_document(&document_under(
            Some("I_plan"),
            "Survey",
            &["acme/work", "acme/other"],
        ))
        .await
        .expect("a document naming several is created in its project's repository");

    assert_eq!(
        created_in(&fixture),
        ["acme/other", "acme/tooling", "acme/tooling"]
    );
    assert_eq!(
        source
            .get_document(&one)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![repo("acme/other")]
    );
    assert_eq!(
        source
            .get_document(&none)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        Vec::<Repository>::new()
    );
    assert_eq!(
        source
            .get_document(&several)
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![repo("acme/work"), repo("acme/other")]
    );
}

#[tokio::test]
async fn a_repository_under_another_owner_than_the_parents_is_refused_before_anything_is_created() {
    let fixture = board(vec![
        Item::issue("I_plan", "Plan").sub_issues(1),
        design("I_design", "Spec").parent("I_plan"),
    ]);
    let source = source(&fixture);

    let task_refusal = refusal(
        source
            .write_task(&task_under(Some("I_plan"), "Elsewhere", &["contoso/work"]))
            .await
            .expect_err("GitHub files a sub-issue only under the same owner as its parent"),
    );
    let document_refusal = refusal(
        source
            .write_document(&document_under(
                Some("I_plan"),
                "Far spec",
                &["contoso/work"],
            ))
            .await
            .expect_err("a document is a sub-issue too"),
    );
    for (message, kind, title) in [
        (&task_refusal, "task", "\"Elsewhere\""),
        (&document_refusal, "document", "\"Far spec\""),
    ] {
        assert!(message.contains(kind), "{message}");
        assert!(message.contains(title), "{message}");
        assert!(message.contains("contoso/work"), "{message}");
        assert!(message.contains("acme"), "both owners are named: {message}");
        assert!(
            message.contains("contoso"),
            "both owners are named: {message}"
        );
        assert!(message.contains("acme/work"), "{message}");
    }
    assert!(
        fixture.seen().is_empty(),
        "nothing was created before either refusal: {:?}",
        fixture.seen()
    );

    // A project's own issue has no parent, so no owner check reaches it: an unparented
    // project under another owner is simply created there.
    source
        .write_project(&project_naming("Contoso plan", &["contoso/work"]))
        .await
        .expect("a project has no parent to agree with");
    assert_eq!(created_in(&fixture), ["contoso/work"]);
}

#[tokio::test]
async fn a_named_repository_the_token_cannot_see_is_refused_naming_the_item() {
    let fixture = board(vec![Item::issue("I_plan", "Plan").sub_issues(1)]);
    let source = source(&fixture);

    let message = refusal(
        source
            .write_task(&task_under(Some("I_plan"), "Unseen", &["acme/missing"]))
            .await
            .expect_err("a repository nothing resolves is refused"),
    );
    assert!(message.contains("task"), "{message}");
    assert!(message.contains("\"Unseen\""), "{message}");
    assert!(message.contains("acme/missing"), "{message}");
    let project_message = refusal(
        source
            .write_project(&project_naming("Unseen plan", &["acme/missing"]))
            .await
            .expect_err("the same for a project"),
    );
    assert!(project_message.contains("project"), "{project_message}");
    assert!(
        project_message.contains("\"Unseen plan\""),
        "{project_message}"
    );
    assert!(
        project_message.contains("acme/missing"),
        "{project_message}"
    );
    assert!(
        fixture.seen().is_empty(),
        "no issue is created for an item its repository refuses: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_repository_that_is_not_on_github_is_refused_naming_the_item() {
    let fixture = board(vec![Item::issue("I_plan", "Plan").sub_issues(1)]);
    let source = source(&fixture);

    for (write, kind, title) in [
        (
            source
                .write_task(&write(Task {
                    project: Some(NativeId("I_plan".to_owned())),
                    repositories: vec![
                        Repository::try_from("gitlab.com/acme/work".to_owned()).unwrap(),
                    ],
                    ..task(
                        "ignored",
                        "Hosted elsewhere",
                        status(StatusCategory::Todo, "Todo"),
                    )
                }))
                .await,
            "task",
            "\"Hosted elsewhere\"",
        ),
        (
            source
                .write_task(&write(Task {
                    repositories: vec![
                        Repository::try_from("github.com/acme/work/nested".to_owned()).unwrap(),
                    ],
                    ..task("ignored", "Too deep", status(StatusCategory::Todo, "Todo"))
                }))
                .await,
            "task",
            "\"Too deep\"",
        ),
    ] {
        let message =
            refusal(write.expect_err("not a GitHub repository this source can create in"));
        assert!(message.contains(kind), "{message}");
        assert!(message.contains(title), "{message}");
        assert!(
            message.contains("gitlab.com/acme/work")
                || message.contains("github.com/acme/work/nested"),
            "{message}"
        );
        assert!(message.contains("github.com"), "{message}");
    }
    let project_message = refusal(
        source
            .write_project(&write(Project {
                repositories: vec![
                    Repository::try_from("gitlab.com/acme/work".to_owned()).unwrap(),
                ],
                ..project(
                    "ignored",
                    "Elsewhere plan",
                    status(StatusCategory::Todo, "Todo"),
                )
            }))
            .await
            .expect_err("the same for a project"),
    );
    assert!(project_message.contains("project"), "{project_message}");
    assert!(
        project_message.contains("\"Elsewhere plan\""),
        "{project_message}"
    );
    assert!(
        project_message.contains("gitlab.com/acme/work"),
        "{project_message}"
    );
    assert!(
        fixture.seen().is_empty(),
        "nothing was created: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_parent_neither_on_the_board_nor_in_this_processs_record_is_refused_before_creation() {
    let fixture = board(vec![]);
    let source = source(&fixture);

    let message = refusal(
        source
            .write_task(&task_under(Some("I_gone"), "Orphaned", &["acme/tooling"]))
            .await
            .expect_err("a parent nothing holds cannot be filed under"),
    );
    assert!(message.contains("I_gone"), "{message}");
    assert!(message.contains("task"), "{message}");
    assert!(message.contains("\"Orphaned\""), "{message}");
    let document_message = refusal(
        source
            .write_document(&document_under(Some("I_gone"), "Orphaned spec", &[]))
            .await
            .expect_err("the same for a document naming no repository"),
    );
    assert!(document_message.contains("I_gone"), "{document_message}");
    assert!(document_message.contains("document"), "{document_message}");
    assert!(
        document_message.contains("\"Orphaned spec\""),
        "{document_message}"
    );
    assert!(
        fixture.seen().is_empty(),
        "nothing was created: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_parent_that_is_a_draft_is_refused_before_creation_because_a_draft_has_no_sub_issues() {
    // A draft is on the board, so the parent is found; but a draft has no repository to
    // place the task in and GitHub gives it no sub-issues, so `addSubIssue` would refuse
    // the task only after `createIssue` had made it. Refused first, whether the task names
    // a repository of its own or leaves the choice to its parent.
    let fixture = board(vec![Item::draft("DI_note", "Sketch")]);
    let source = source(&fixture);

    let named = refusal(
        source
            .write_task(&task_under(
                Some("DI_note"),
                "Under a draft",
                &["acme/tooling"],
            ))
            .await
            .expect_err("a draft cannot have sub-issues"),
    );
    assert!(named.contains("DI_note"), "{named}");
    assert!(named.contains("draft"), "{named}");
    assert!(named.contains("task"), "{named}");
    assert!(named.contains("\"Under a draft\""), "{named}");
    let unnamed = refusal(
        source
            .write_document(&document_under(Some("DI_note"), "Draft spec", &[]))
            .await
            .expect_err("the same for a document leaving the choice to its parent"),
    );
    assert!(unnamed.contains("DI_note"), "{unnamed}");
    assert!(unnamed.contains("draft"), "{unnamed}");
    assert!(unnamed.contains("document"), "{unnamed}");
    assert!(unnamed.contains("\"Draft spec\""), "{unnamed}");
    assert!(
        fixture.seen().is_empty(),
        "nothing was created: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_parent_in_a_repository_this_source_cannot_spell_is_refused_before_creation() {
    // The contract's `Repository` accepts any `host/owner/name`, and this source's
    // `owner/name` grammar is a floor narrower than GitHub's — an Enterprise Managed User's
    // login carries an underscore no ordinary login may — so a board can report a parent in
    // a repository this source cannot name for `createIssue` or compare an owner against.
    // Refused before anything is created, naming the parent and where the board says it
    // is, whether the task names a repository or leaves the choice to it.
    let fixture = board(vec![
        Item::issue("I_odd", "Plan")
            .sub_issues(1)
            .in_repository("octocat_acme/work"),
    ]);
    let source = source(&fixture);

    for (title, repositories) in [
        ("Placed by parent", &[][..]),
        ("Named", &["acme/tooling"][..]),
    ] {
        let error = source
            .write_task(&task_under(Some("I_odd"), title, repositories))
            .await
            .expect_err("a parent in a repository this source cannot spell is refused");
        assert!(matches!(error, SourceError::Malformed { .. }), "{error}");
        let message = refusal(error);
        assert!(message.contains("I_odd"), "{message}");
        assert!(message.contains("octocat_acme/work"), "{message}");
        assert!(message.contains("task"), "{message}");
        assert!(message.contains(&format!("{title:?}")), "{message}");
    }
    assert!(
        fixture.seen().is_empty(),
        "nothing was created: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn an_existing_issue_is_never_moved_and_a_differing_list_is_recorded() {
    let fixture = board(vec![
        Item::issue("I_1", "Settled").in_repository("acme/tooling"),
    ]);
    let source = source(&fixture);

    let mut revised = task("ignored", "Settled", status(StatusCategory::Todo, "Todo"));
    revised.repositories = vec![repo("acme/other")];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("an update of an existing issue");

    assert!(created_in(&fixture).is_empty(), "no issue was created");
    assert!(
        !fixture.seen().iter().any(|call| call[0] == "deleteIssue"),
        "and none deleted"
    );
    let held = fixture.item("I_1");
    assert_eq!(
        held.repository.as_deref(),
        Some("acme/tooling"),
        "the issue stays put"
    );
    assert!(held.body.unwrap().contains(Repository::METADATA_KEY));
    assert_eq!(
        source
            .get_task(&NativeId("I_1".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .repositories,
        vec![repo("acme/other")],
        "the list is reported as written"
    );
}

#[tokio::test]
async fn each_distinct_repository_is_looked_up_once_per_command() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&project_naming("Plan", &[]))
        .await
        .expect("a project in the configured repository");
    for (title, slug) in [
        ("First", "acme/one"),
        ("Second", "acme/two"),
        ("Third", "acme/three"),
        ("First again", "acme/one"),
        ("Second again", "acme/two"),
        ("Unplaced", "acme/work"),
    ] {
        source
            .write_task(&task_under(Some(&plan.0), title, &[slug]))
            .await
            .expect("a task of the plan");
    }
    // The configured repository is read with the board's fields by the first create, and
    // each of the other three on its own the first time an item names it.
    assert_eq!(
        fixture.requests("repository"),
        3,
        "acme/one, acme/two and acme/three, each once: {:?}",
        fixture.documents()
    );
    assert_eq!(
        fixture
            .documents()
            .iter()
            .filter(|document| *document == onetaskgraph_github_projects::graphql::CREATION_CONTEXT)
            .count(),
        1,
        "acme/work, once, with the board's fields"
    );
    assert_eq!(
        created_in(&fixture),
        [
            "acme/work",
            "acme/one",
            "acme/two",
            "acme/three",
            "acme/one",
            "acme/two",
            "acme/work",
        ]
    );
}

#[tokio::test]
async fn the_shipped_mapping_puts_each_category_where_it_says_it_does() {
    let fixture = queued_board(vec![]);
    let source = source(&fixture);
    for (category, name, expected_option, expected_state) in [
        (StatusCategory::Backlog, "Backlog", Some("Backlog"), "OPEN"),
        (StatusCategory::Todo, "Todo", Some("Todo"), "OPEN"),
        (StatusCategory::Queued, "Queued", Some("Queued"), "OPEN"),
        (
            StatusCategory::InProgress,
            "In Progress",
            Some("In Progress"),
            "OPEN",
        ),
        (StatusCategory::Done, "Done", Some("Done"), "CLOSED"),
        (
            StatusCategory::Cancelled,
            "Cancelled",
            Some("Cancelled"),
            "CLOSED",
        ),
    ] {
        let id = source
            .write_task(&write(task("T", "one", status(category, name))))
            .await
            .unwrap_or_else(|error| panic!("{category:?}: {error}"));
        let held = fixture.item(&id.0);
        assert_eq!(held.status.as_deref(), expected_option, "{category:?}");
        assert_eq!(held.state, expected_state, "{category:?}");
    }
    let closed = fixture
        .seen()
        .into_iter()
        .filter(|call| call[0] == "updateIssue")
        .map(|call| call[1]["stateInput"]["stateReason"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        closed,
        vec![json!("COMPLETED"), json!("NOT_PLANNED")],
        "done is precisely COMPLETED and cancelled is precisely NOT_PLANNED"
    );
}

#[tokio::test]
async fn done_closes_by_default_and_an_override_names_its_terminal_column() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"done":"Shipped"}}),
    );
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Done, "Shipped"),
        )))
        .await
        .unwrap();
    let held = fixture.item(&id.0);
    assert_eq!(held.state, "CLOSED", "an overridden done remains terminal");
    assert_eq!(held.status.as_deref(), Some("Shipped"));
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().status,
        status(StatusCategory::Done, "Shipped")
    );
}

#[tokio::test]
async fn a_disabled_status_is_refused_naming_the_status_and_the_instance() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"backlog":null}}),
    );
    let message = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "one",
                status(StatusCategory::Backlog, "Backlog"),
            )))
            .await
            .expect_err("a disabled status cannot be written"),
    );
    assert!(message.contains("backlog"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    assert!(message.contains("status_mapping"), "{message}");
    assert!(fixture.seen().is_empty(), "nothing is written first");
}

#[tokio::test]
async fn draft_is_refused_because_a_draft_issue_cannot_have_sub_issues() {
    let fixture = board(vec![]);
    let message = refusal(
        source(&fixture)
            .write_task(&write(task(
                "T-1",
                "one",
                status(StatusCategory::Draft, "Draft"),
            )))
            .await
            .expect_err("draft is disabled by the shipped mapping"),
    );
    assert!(message.contains("draft"), "{message}");
    assert!(message.contains("work"), "the instance is named: {message}");
    assert!(message.contains("sub-issue"), "{message}");
}

#[tokio::test]
async fn a_status_the_board_cannot_represent_is_refused_naming_the_status_and_the_instance() {
    for (extra, missing) in [
        (json!({"status_mapping":{"todo":"Nowhere"}}), "Nowhere"),
        (json!({}), "Backlog"),
    ] {
        let fixture = board_with(vec![], !extra.as_object().unwrap().is_empty(), true);
        let source = configured(&fixture.endpoint, extra.clone());
        let category = if extra.as_object().unwrap().is_empty() {
            (StatusCategory::Backlog, "Backlog")
        } else {
            (StatusCategory::Todo, "Todo")
        };
        let message = refusal(
            source
                .write_task(&write(task("T-1", "one", status(category.0, category.1))))
                .await
                .expect_err("a status the board cannot hold is refused"),
        );
        assert!(message.contains(missing), "{message}");
        assert!(message.contains("work"), "the instance is named: {message}");
        assert!(fixture.seen().is_empty(), "nothing is written first");
    }
}

#[tokio::test]
async fn a_missing_terminal_option_refuses_before_either_representation_changes() {
    let fixture = board(vec![
        Item::issue("I_1", "addressed").status("Todo"),
        Item::issue("I_2", "unrelated").status("In Progress"),
    ]);
    fixture
        .state
        .lock()
        .unwrap()
        .options
        .retain(|(_, name)| *name != "Done");
    let source = source(&fixture);
    let message = refusal(
        source
            .set_task_status(&id("I_1"), StatusCategory::Done)
            .await
            .expect_err("done needs its mapped board option before it can close"),
    );
    assert!(message.contains("Done"), "{message}");
    assert!(fixture.seen().is_empty(), "neither half was written first");
    assert_eq!(
        (fixture.item("I_1").state, fixture.item("I_1").status),
        ("OPEN", Some("Todo".to_owned()))
    );
    assert_eq!(
        (fixture.item("I_2").state, fixture.item("I_2").status),
        ("OPEN", Some("In Progress".to_owned())),
        "an unrelated item is untouched"
    );
}

#[tokio::test]
async fn a_general_write_missing_its_terminal_option_refuses_before_any_mutation() {
    let fixture = board(vec![Item::issue("I_1", "addressed").status("Todo")]);
    fixture
        .state
        .lock()
        .unwrap()
        .options
        .retain(|(_, name)| *name != "Cancelled");
    let source = source(&fixture);
    let mut revised = task(
        "T-1",
        "addressed",
        status(StatusCategory::Cancelled, "Cancelled"),
    );
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let message = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(id("I_1")),
                item: revised,
                depends_on: vec![],
            })
            .await
            .expect_err("cancelled needs its mapped option before the general write starts"),
    );
    assert!(message.contains("Cancelled"), "{message}");
    assert!(fixture.seen().is_empty(), "no mutation was sent first");
    assert_eq!(
        (fixture.item("I_1").state, fixture.item("I_1").status),
        ("OPEN", Some("Todo".to_owned()))
    );
}

#[tokio::test]
async fn a_terminal_close_refusal_is_reported_after_the_option_write_on_both_write_paths() {
    let narrow = board(vec![Item::issue("I_1", "addressed").status("Todo")]);
    narrow.refuse("updateIssue");
    let error = source(&narrow)
        .set_task_status(&id("I_1"), StatusCategory::Done)
        .await
        .expect_err("GitHub refused the close after accepting the mapped option");
    assert!(error.to_string().contains("updateIssue"), "{error}");
    assert_eq!(narrow.item("I_1").state, "OPEN");
    assert_eq!(narrow.item("I_1").status.as_deref(), Some("Done"));

    let created = board(vec![]);
    created.refuse("updateIssue");
    let maker = source(&created);
    let error = maker
        .write_task(&write(task(
            "T-1",
            "new terminal task",
            status(StatusCategory::Done, "Done"),
        )))
        .await
        .expect_err("the general write's terminal close was refused");
    assert!(error.to_string().contains("updateIssue"), "{error}");
    let seen = created.seen();
    let option = seen
        .iter()
        .position(|call| call[0] == "updateProjectV2ItemFieldValue")
        .expect("the mapped option landed before the close");
    let close = seen
        .iter()
        .position(|call| call[0] == "updateIssue")
        .expect("the close was attempted");
    let cleanup = seen
        .iter()
        .position(|call| call[0] == "deleteIssue")
        .expect("the failed newly-created item was taken back");
    assert!(option < close && close < cleanup, "{seen:?}");
    assert!(
        maker
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "the failed general write left no artifact"
    );
}

#[tokio::test]
async fn a_closed_issue_reports_the_closed_category_and_its_column_name() {
    let fixture = board(vec![
        Item::issue("I_done", "shipped")
            .closed(Some("COMPLETED"))
            .status("Shipped"),
        Item::issue("I_bare", "shipped").closed(Some("COMPLETED")),
        Item::issue("I_cancelled", "dropped").closed(Some("NOT_PLANNED")),
        Item::issue("I_duplicate", "again")
            .closed(Some("DUPLICATE"))
            .status("Shipped"),
        Item::issue("I_reopened", "odd").closed(Some("REOPENED")),
        Item::issue("I_legacy", "old").closed(None),
        Item::issue("I_open", "doing").status("In Progress"),
    ]);
    let source = source(&fixture);
    async fn read(source: &dyn TaskSource, id: &str) -> Status {
        source
            .get_task(&NativeId(id.to_owned()))
            .await
            .unwrap()
            .unwrap()
            .status
    }
    assert_eq!(
        read(source.as_ref(), "I_done").await,
        status(StatusCategory::Done, "Shipped"),
        "the closed state decides the category and the option decides the name"
    );
    assert_eq!(
        read(source.as_ref(), "I_bare").await,
        status(StatusCategory::Done, "Done")
    );
    assert_eq!(
        read(source.as_ref(), "I_cancelled").await,
        status(StatusCategory::Cancelled, "Cancelled")
    );
    assert_eq!(
        read(source.as_ref(), "I_duplicate").await,
        status(StatusCategory::Unknown, "Shipped"),
        "a closed-as-duplicate task is not finished work, whatever column it sits in"
    );
    assert_eq!(
        read(source.as_ref(), "I_reopened").await,
        status(StatusCategory::Unknown, "Closed")
    );
    assert_eq!(
        read(source.as_ref(), "I_legacy").await,
        status(StatusCategory::Done, "Done")
    );
    assert_eq!(
        read(source.as_ref(), "I_open").await,
        status(StatusCategory::InProgress, "In Progress"),
        "an open issue reads both from its option"
    );
}

#[tokio::test]
async fn writing_a_non_terminal_status_reopens_a_closed_issue_so_a_copy_settles() {
    let fixture = board(vec![
        Item::issue("I_1", "shipped")
            .closed(Some("COMPLETED"))
            .status("Shipped"),
    ]);
    let source = source(&fixture);
    let mut back = task("T-1", "shipped", status(StatusCategory::Todo, "Todo"));
    back.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: back.clone(),
            depends_on: vec![],
        })
        .await
        .expect("a non-terminal status is writable over a closed issue");
    assert_eq!(fixture.item("I_1").state, "OPEN");
    let read = source
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.status,
        status(StatusCategory::Todo, "Todo"),
        "without the reopen this would read Unknown and a copy would report a change forever"
    );
    assert_eq!(read.title, back.title);
}

#[tokio::test]
async fn an_unknown_status_category_key_names_the_instance() {
    let message = build_refusal(json!({"owner":"octo-org","project_number":7,
        "endpoint":"https://api.github.com/graphql",
        "status_mapping":{"shipped":"Shipped"}}));
    assert!(message.contains("shipped"), "{message}");
    assert!(message.contains("work"), "{message}");
    assert!(message.contains("in-progress"), "{message}");
}

#[tokio::test]
async fn a_status_mapping_accepts_only_the_shared_grammar() {
    // An object is a per-kind name now, so `closed` — the shape this once refused as not an
    // option name at all — is refused as an item kind the grammar does not have.
    let message = build_refusal(json!({"owner":"octo-org","project_number":7,
        "endpoint":"https://api.github.com/graphql",
        "status_mapping":{"unknown":{"closed":"completed"}}}));
    assert!(
        message.contains(
            "source work: status_mapping.unknown names \"closed\", which is not an item kind"
        ),
        "{message}"
    );
}

#[tokio::test]
async fn two_categories_cannot_share_one_board_option() {
    let message = build_refusal(json!({"owner":"octo-org","project_number":7,
        "endpoint":"https://api.github.com/graphql",
        "status_mapping":{"todo":"Backlog"}}));
    assert!(message.contains("Backlog"), "{message}");
    assert!(message.contains("todo"), "{message}");
    assert!(message.contains("backlog"), "{message}");
}

#[tokio::test]
async fn a_blank_option_name_and_a_malformed_target_are_refused() {
    for mapping in [json!({"todo":""}), json!({"todo":{"closed":"maybe"}})] {
        assert!(
            Plugin
                .build(
                    &SourceName::new("work").unwrap(),
                    &json!({"owner":"octo-org","project_number":7,
                            "endpoint":"https://api.github.com/graphql",
                            "status_mapping":mapping}),
                    &Secrets,
                )
                .is_err(),
            "{mapping}"
        );
    }
}

/// A board whose `Status` field also offers `Queued`, where the shipped mapping sends
/// `queued`.
fn queued_board(items: Vec<Item>) -> Fixture {
    let fixture = board(items);
    fixture.offer_option("OPT_queued", "Queued");
    fixture
}

fn refs(entries: &[&str]) -> Vec<TaskRef> {
    entries
        .iter()
        .map(|entry| TaskRef::new(*entry).expect("a task id"))
        .collect()
}

fn id(native: &str) -> NativeId {
    NativeId(native.to_owned())
}

/// A body holding prose and then a slot of exactly `slot`.
fn slotted(prose: &str, slot: &Value) -> String {
    format!("{prose}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->")
}

/// One stored body split around its metadata slot: the bytes before the slot's JSON, that
/// JSON parsed, and the bytes after it.
fn split_slot(body: &str) -> (&str, Value, &str) {
    let open = "<!-- onetaskgraph.metadata\n";
    let start = body.rfind(open).expect("a slot") + open.len();
    let end = start + body[start..].find("\n-->").expect("a closed slot");
    (
        &body[..start],
        serde_json::from_str(&body[start..end]).expect("the slot's JSON"),
        &body[end..],
    )
}

/// Every mutation this board received, as its operation and the input fields it carried.
fn mutation_fields(fixture: &Fixture) -> Vec<(String, BTreeSet<String>)> {
    fixture
        .seen()
        .iter()
        .map(|call| {
            (
                call[0].as_str().expect("an operation").to_owned(),
                call[1]
                    .as_object()
                    .expect("an input object")
                    .keys()
                    .cloned()
                    .collect(),
            )
        })
        .collect()
}

fn fields(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test]
async fn an_open_item_at_the_queued_option_reads_back_as_queued_named_queued() {
    let fixture = queued_board(vec![Item::issue("I_held", "claimed").status("Queued")]);
    let source = source(&fixture);
    let written = source
        .write_task(&write(task(
            "T-1",
            "claimed too",
            status(StatusCategory::Queued, "Queued"),
        )))
        .await
        .expect("queued lands where the shipped mapping sends it");
    assert_eq!(fixture.item(&written.0).status.as_deref(), Some("Queued"));
    assert_eq!(fixture.item(&written.0).state, "OPEN");
    // A source that wrote nothing reads both from GitHub rather than from its own record.
    let fresh = source_of(&fixture);
    for held in ["I_held", written.0.as_str()] {
        assert_eq!(
            fresh.get_task(&id(held)).await.unwrap().unwrap().status,
            status(StatusCategory::Queued, "Queued"),
            "{held}"
        );
    }
}

fn source_of(fixture: &Fixture) -> Box<dyn TaskSource> {
    configured(&fixture.endpoint, json!({}))
}

#[tokio::test]
async fn status_mapping_queued_names_the_board_option_queued_lands_on() {
    let fixture = board(vec![Item::issue("I_1", "held").status("Shipped")]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"queued":"Shipped"}}),
    );
    assert_eq!(
        source.get_task(&id("I_1")).await.unwrap().unwrap().status,
        status(StatusCategory::Queued, "Shipped")
    );
    let written = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Queued, "Shipped"),
        )))
        .await
        .expect("queued lands on the configured option");
    assert_eq!(fixture.item(&written.0).status.as_deref(), Some("Shipped"));
}

#[tokio::test]
async fn queued_cannot_share_a_board_option_with_another_category() {
    for (mapping, earlier, later, option) in [
        (json!({"queued":"Todo"}), "todo", "queued", "Todo"),
        (
            json!({"in-progress":"Queued"}),
            "queued",
            "in-progress",
            "Queued",
        ),
        (json!({"queued":"backlog"}), "backlog", "queued", "backlog"),
    ] {
        let message = build_refusal(json!({"owner":"octo-org","project_number":7,
            "endpoint":"https://api.github.com/graphql","status_mapping":mapping}));
        assert!(
            message.contains(&format!("sends both {earlier} and {later}")),
            "{message}"
        );
        assert!(message.contains(&format!("{option:?}")), "{message}");
    }
}

#[tokio::test]
async fn queued_over_a_board_without_that_option_is_refused_naming_it() {
    let fixture = board(vec![Item::issue("I_1", "held").status("Todo")]);
    let source = source(&fixture);
    let written = refusal(
        source
            .write_task(&write(task(
                "T-1",
                "one",
                status(StatusCategory::Queued, "Queued"),
            )))
            .await
            .expect_err("the board has no Queued option"),
    );
    let set = refusal(
        source
            .set_task_status(&id("I_1"), StatusCategory::Queued)
            .await
            .expect_err("the board has no Queued option"),
    );
    for message in [&written, &set] {
        assert!(message.contains("\"Queued\""), "{message}");
        assert!(message.contains("status queued"), "{message}");
        assert!(message.contains("work"), "the instance is named: {message}");
    }
    assert!(fixture.seen().is_empty(), "nothing is written first");
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("Todo"));
}

#[tokio::test]
async fn delivers_and_delivered_by_land_in_the_slot_and_never_among_the_callers_metadata() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "Ship it", status(StatusCategory::Todo, "Todo"));
    item.content = Some("The prose.".to_owned());
    item.delivers = refs(&["I_far", "plans:P-9"]);
    item.delivered_by = refs(&["plans:P-1"]);
    item.metadata = BTreeMap::from([
        ("caller.kept".to_owned(), json!({"nested":[1, true]})),
        (TaskRef::DELIVERS_KEY.to_owned(), json!(["stale:ignored"])),
        (
            TaskRef::DELIVERED_BY_KEY.to_owned(),
            json!("not even a list"),
        ),
    ]);
    let written = source
        .write_task(&write(item))
        .await
        .expect("a task with both lists is written");
    let body = fixture.item(&written.0).body.expect("a body");
    let (_, slot, _) = split_slot(&body);
    assert_eq!(
        slot[TaskRef::DELIVERS_KEY],
        json!(["I_far", "plans:P-9"]),
        "the typed field is what lands, not the caller's key of the same name"
    );
    assert_eq!(slot[TaskRef::DELIVERED_BY_KEY], json!(["plans:P-1"]));

    let fresh = source_of(&fixture);
    for reader in [source.as_ref(), fresh.as_ref()] {
        let read = reader.get_task(&written).await.unwrap().unwrap();
        assert_eq!(read.delivers, refs(&["I_far", "plans:P-9"]));
        assert_eq!(read.delivered_by, refs(&["plans:P-1"]));
        assert_eq!(
            read.metadata,
            BTreeMap::from([("caller.kept".to_owned(), json!({"nested":[1, true]}))]),
            "neither reserved key is ever among the caller's own metadata"
        );
        assert_eq!(read.content.as_deref(), Some("The prose."));
    }

    let mut cleared = task("T-1", "Ship it", status(StatusCategory::Todo, "Todo"));
    cleared.content = Some("The prose.".to_owned());
    cleared.repositories = vec![repo("acme/work")];
    source
        .write_task(&ItemWrite {
            target: Some(written.clone()),
            item: cleared,
            depends_on: vec![],
        })
        .await
        .expect("an update emptying both lists");
    let body = fixture.item(&written.0).body.unwrap();
    assert!(!body.contains("onetaskgraph.deliver"), "{body}");
    let read = fresh.get_task(&written).await.unwrap().unwrap();
    assert!(read.delivers.is_empty() && read.delivered_by.is_empty());
}

#[tokio::test]
async fn a_project_or_a_document_neither_stores_nor_reports_either_delivery_key() {
    let fixture = board(vec![
        Item::issue("I_plan", "a plan").body(&slotted(
            "plan prose",
            &json!({"caller.x":1,"onetaskgraph.delivers":"not a list",
                    "onetaskgraph.item_kind":"project"}),
        )),
        design("I_doc", "notes").body(&slotted(
            "doc prose",
            &json!({"caller.x":1,"onetaskgraph.delivered_by":["I_doc"]}),
        )),
    ]);
    let source = source(&fixture);
    let caller = BTreeMap::from([("caller.x".to_owned(), json!(1))]);
    assert_eq!(
        source
            .get_project(&id("I_plan"))
            .await
            .expect("a project's delivery key is not read, so it cannot be refused")
            .unwrap()
            .metadata,
        caller
    );
    assert_eq!(
        source
            .get_document(&id("I_doc"))
            .await
            .unwrap()
            .unwrap()
            .metadata,
        caller
    );

    let reserved = BTreeMap::from([
        ("caller.x".to_owned(), json!(1)),
        (TaskRef::DELIVERS_KEY.to_owned(), json!(["plans:T-1"])),
        (TaskRef::DELIVERED_BY_KEY.to_owned(), json!(["plans:T-2"])),
    ]);
    let mut plan = project("P-1", "a new plan", status(StatusCategory::Todo, "Todo"));
    plan.metadata = reserved.clone();
    let plan = source.write_project(&write(plan)).await.unwrap();
    let mut notes = document("D-1", "new notes");
    notes.metadata = reserved;
    let notes = source.write_document(&write(notes)).await.unwrap();
    for written in [plan, notes] {
        let body = fixture.item(&written.0).body.unwrap();
        assert!(!body.contains("onetaskgraph.deliver"), "{body}");
        assert!(body.contains("caller.x"), "{body}");
    }
}

#[tokio::test]
async fn a_delivery_list_this_source_cannot_read_is_refused_naming_the_task_and_the_entry() {
    for (key, value, entry) in [
        (TaskRef::DELIVERS_KEY, json!("I_2"), "\"I_2\""),
        (TaskRef::DELIVERS_KEY, json!([7]), "7"),
        (TaskRef::DELIVERED_BY_KEY, json!([""]), "\"\""),
        (TaskRef::DELIVERED_BY_KEY, json!(["plans:"]), "plans:"),
        (TaskRef::DELIVERS_KEY, json!(["I_1"]), "I_1"),
        (TaskRef::DELIVERED_BY_KEY, json!(["work:I_1"]), "work:I_1"),
        (
            TaskRef::DELIVERED_BY_KEY,
            json!(["plans:P-1", "plans:P-1"]),
            "plans:P-1",
        ),
        (
            TaskRef::DELIVERS_KEY,
            json!(["I_2", "work:I_2"]),
            "work:I_2",
        ),
    ] {
        let mut slot = serde_json::Map::new();
        slot.insert(key.to_owned(), value.clone());
        let fixture = board(vec![
            Item::issue("I_1", "a task").body(&slotted("prose", &Value::Object(slot))),
        ]);
        let error = source(&fixture)
            .get_task(&id("I_1"))
            .await
            .expect_err("a delivery list this source cannot read is refused");
        assert!(
            matches!(error, SourceError::Malformed { .. }),
            "{value}: {error:?}"
        );
        let message = error.to_string();
        assert!(message.contains(key), "{message}");
        assert!(message.contains("task I_1"), "{message}");
        assert!(message.contains(entry), "{message}");
    }
}

#[tokio::test]
async fn a_delivery_list_naming_its_own_task_or_one_task_twice_is_refused_before_any_request() {
    let fixture = board(vec![Item::issue("I_1", "held")]);
    let source = source(&fixture);
    for (target, delivers, delivered_by, near, entry) in [
        (None, refs(&["T-1"]), vec![], "T-1", "T-1"),
        (None, vec![], refs(&["work:T-1"]), "T-1", "work:T-1"),
        (Some("I_1"), refs(&["work:I_1"]), vec![], "I_1", "work:I_1"),
        (
            Some("I_1"),
            vec![],
            refs(&["plans:P-1", "plans:P-1"]),
            "I_1",
            "plans:P-1",
        ),
        (None, refs(&["I_2", "work:I_2"]), vec![], "T-1", "work:I_2"),
    ] {
        let mut item = task("T-1", "x", status(StatusCategory::Todo, "Todo"));
        item.delivers = delivers;
        item.delivered_by = delivered_by;
        let error = source
            .write_task(&ItemWrite {
                target: target.map(id),
                item,
                depends_on: vec![],
            })
            .await
            .expect_err("the list is refused");
        assert!(matches!(error, SourceError::Refused { .. }), "{error:?}");
        let message = error.to_string();
        assert!(message.contains(&format!("task {near}")), "{message}");
        assert!(message.contains(entry), "{message}");
    }
    assert!(
        fixture.documents().is_empty(),
        "nothing is read or written before the lists are checked"
    );
}

#[tokio::test]
async fn a_status_set_to_a_column_reopens_a_closed_issue_moves_its_option_and_nothing_else() {
    let fixture = queued_board(vec![
        Item::issue("I_1", "shipped early")
            .body(&slotted("prose", &json!({"caller.x":1})))
            .labelled(&[("L_1", "bug")])
            .closed(Some("COMPLETED"))
            .status("Shipped"),
        Item::issue("I_bare", "no column").closed(Some("NOT_PLANNED")),
    ]);
    let source = source(&fixture);
    let before = fixture.item("I_1");
    let answered = source
        .set_task_status(&id("I_1"), StatusCategory::Queued)
        .await
        .expect("a column status is writable over a closed issue")
        .expect("a task of this board");
    assert_eq!(answered, status(StatusCategory::Queued, "Queued"));
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["updateIssue", {"id":"I_1","stateInput":{"value":"OPEN"}}]),
            json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board",
                "itemId":"PVTI_I_1","fieldId":"FIELD_status",
                "value":{"singleSelectOptionId":"OPT_queued"}}]),
        ],
        "a reopen carrying its state alone, then the option"
    );
    let after = fixture.item("I_1");
    assert_eq!(
        (after.state, after.status.as_deref()),
        ("OPEN", Some("Queued"))
    );
    assert_eq!(after.title, before.title);
    assert_eq!(after.body, before.body);
    assert_eq!(after.labels, before.labels);
    assert_eq!(
        source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .unwrap()
            .status,
        answered,
        "the answer is what a re-read reports"
    );
    assert!(
        fixture.board_item_reads().is_empty(),
        "the item named its board and its Status field, so the board was not read"
    );

    // An item holding no `Status` value cannot say what the field's options are from its
    // own values — but its read by id carries the board's field definitions beside them, so
    // the board's fields are not read on their own, and nothing lists its items.
    assert_eq!(
        source
            .set_task_status(&id("I_bare"), StatusCategory::Todo)
            .await
            .unwrap(),
        Some(status(StatusCategory::Todo, "Todo"))
    );
    assert_eq!(fixture.requests("boardFields"), 0);
    assert!(fixture.board_item_reads().is_empty());
    assert_eq!(fixture.item("I_bare").state, "OPEN");
}

#[tokio::test]
async fn a_status_set_to_a_column_on_an_open_issue_or_a_draft_moves_only_its_option() {
    let fixture = board(vec![
        Item::issue("I_open", "doing").status("Todo"),
        Item::draft("D_1", "a draft")
            .body("draft prose")
            .status("Todo"),
    ]);
    let source = source(&fixture);
    for held in ["I_open", "D_1"] {
        let answered = source
            .set_task_status(&id(held), StatusCategory::InProgress)
            .await
            .unwrap()
            .expect("a task of this board");
        assert_eq!(
            answered,
            status(StatusCategory::InProgress, "In Progress"),
            "{held}"
        );
        assert_eq!(
            source_of(&fixture)
                .get_task(&id(held))
                .await
                .unwrap()
                .unwrap()
                .status,
            answered,
            "{held}"
        );
    }
    assert_eq!(
        mutation_fields(&fixture),
        vec![
            (
                "updateProjectV2ItemFieldValue".to_owned(),
                fields(&["fieldId", "itemId", "projectId", "value"])
            );
            2
        ]
    );
    assert_eq!(fixture.item("I_open").state, "OPEN");
    assert_eq!(fixture.item("D_1").title, "a draft");
    assert_eq!(fixture.item("D_1").body.as_deref(), Some("draft prose"));
}

#[tokio::test]
async fn a_terminal_status_set_selects_its_mapped_option_then_closes_with_its_reason() {
    let fixture = board(vec![
        Item::issue("I_1", "one")
            .status("Shipped")
            .body("kept prose"),
        Item::issue("I_2", "two"),
        Item::issue("I_3", "unrelated").status("In Progress"),
    ]);
    let source = source(&fixture);
    let done = source
        .set_task_status(&id("I_1"), StatusCategory::Done)
        .await
        .unwrap()
        .unwrap();
    let cancelled = source
        .set_task_status(&id("I_2"), StatusCategory::Cancelled)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done, status(StatusCategory::Done, "Done"));
    assert_eq!(cancelled, status(StatusCategory::Cancelled, "Cancelled"));
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board",
                "itemId":"PVTI_I_1","fieldId":"FIELD_status",
                "value":{"singleSelectOptionId":"OPT_done"}}]),
            json!(["updateIssue", {"id":"I_1",
                "stateInput":{"value":"CLOSED","stateReason":"COMPLETED"}}]),
            json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board",
                "itemId":"PVTI_I_2","fieldId":"FIELD_status",
                "value":{"singleSelectOptionId":"OPT_cancelled"}}]),
            json!(["updateIssue", {"id":"I_2",
                "stateInput":{"value":"CLOSED","stateReason":"NOT_PLANNED"}}]),
        ]
    );
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("Done"));
    assert_eq!(fixture.item("I_1").body.as_deref(), Some("kept prose"));
    assert_eq!(
        (fixture.item("I_3").state, fixture.item("I_3").status),
        ("OPEN", Some("In Progress".to_owned())),
        "the write changes only the addressed items"
    );
    let fresh = source_of(&fixture);
    for (held, answered) in [("I_1", done), ("I_2", cancelled)] {
        assert_eq!(
            fresh.get_task(&id(held)).await.unwrap().unwrap().status,
            answered,
            "{held}"
        );
    }
}

#[tokio::test]
async fn a_status_set_refuses_what_a_write_of_it_refuses_and_answers_none_for_no_task() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft").status("Todo"),
        Item::issue("I_plan", "a plan").sub_issues(1),
        Item::issue("I_step", "a step")
            .parent("I_plan")
            .status("Todo"),
        design("I_doc", "notes"),
    ]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"backlog":null}}),
    );
    let closed = refusal(
        source
            .set_task_status(&id("D_1"), StatusCategory::Done)
            .await
            .expect_err("a draft has no closed state"),
    );
    let written = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(id("D_1")),
                item: task("D_1", "a draft", status(StatusCategory::Done, "Done")),
                depends_on: vec![],
            })
            .await
            .expect_err("a draft has no closed state"),
    );
    assert_eq!(closed, written, "a status set says what a write says");
    assert!(closed.contains("draft"), "{closed}");

    for category in [
        StatusCategory::Backlog,
        StatusCategory::Draft,
        StatusCategory::Unknown,
    ] {
        let set = refusal(
            source
                .set_task_status(&id("I_step"), category)
                .await
                .expect_err("a disabled status"),
        );
        let written = refusal(
            source
                .write_task(&write(task("T-1", "x", status(category, "x"))))
                .await
                .expect_err("a disabled status"),
        );
        assert_eq!(set, written, "{category:?}");
    }

    for held in ["I_plan", "I_doc", "I_missing"] {
        assert_eq!(
            source
                .set_task_status(&id(held), StatusCategory::Todo)
                .await
                .unwrap(),
            None,
            "{held} is no task of this board"
        );
    }
    assert!(fixture.seen().is_empty(), "nothing was written");
}

#[tokio::test]
async fn a_status_set_is_what_the_rest_of_the_command_reads() {
    let fixture = queued_board(vec![
        Item::issue("I_1", "one").status("Todo"),
        Item::issue("I_2", "two").closed(Some("COMPLETED")),
    ]);
    let source = source(&fixture);
    async fn statuses(source: &dyn TaskSource) -> Vec<(String, Status)> {
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|task| (task.id.0, task.status))
            .collect()
    }
    assert_eq!(
        statuses(source.as_ref()).await,
        vec![
            ("I_1".to_owned(), status(StatusCategory::Todo, "Todo")),
            ("I_2".to_owned(), status(StatusCategory::Done, "Done")),
        ]
    );
    source
        .set_task_status(&id("I_1"), StatusCategory::Queued)
        .await
        .unwrap();
    source
        .set_task_status(&id("I_2"), StatusCategory::InProgress)
        .await
        .unwrap();
    let expected = vec![
        ("I_1".to_owned(), status(StatusCategory::Queued, "Queued")),
        (
            "I_2".to_owned(),
            status(StatusCategory::InProgress, "In Progress"),
        ),
    ];
    assert_eq!(statuses(source.as_ref()).await, expected);
    assert_eq!(
        fixture.board_item_reads().len(),
        1,
        "the second list is answered from this command's own view of the board"
    );
    assert_eq!(statuses(source_of(&fixture).as_ref()).await, expected);
}

#[tokio::test]
async fn delivered_by_is_replaced_by_one_body_update_that_changes_only_the_slot() {
    let held = "Prose with a `<!-- comment -->` in it.\n\n\n  and trailing spaces  \n\n\
                <!-- onetaskgraph.metadata\n{ \"caller.x\" : 1, \"onetaskgraph.delivers\": [\"I_9\"], \
                \"onetaskgraph.delivered_by\": [\"old:T-1\"], \"onetaskgraph.item_kind\":\"task\" }\n-->";
    let fixture = board(vec![
        Item::issue("I_1", "a task")
            .body(held)
            .labelled(&[("L_1", "bug")])
            .status("Todo"),
    ]);
    let source = source(&fixture);
    assert_eq!(
        source
            .set_delivered_by(&id("I_1"), &refs(&["plans:P-1", "work:I_2"]))
            .await
            .expect("a delivered_by list is writable"),
        Some(())
    );
    assert_eq!(
        mutation_fields(&fixture),
        vec![("updateIssue".to_owned(), fields(&["body", "id"]))],
        "one update, carrying the body and nothing else"
    );
    let stored = fixture.item("I_1").body.unwrap();
    let (before_slot, _, after_slot) = split_slot(held);
    let (prefix, slot, suffix) = split_slot(&stored);
    assert_eq!(
        (prefix, suffix),
        (before_slot, after_slot),
        "every byte outside the slot is as it was"
    );
    assert_eq!(
        slot,
        json!({"caller.x":1,"onetaskgraph.delivers":["I_9"],
               "onetaskgraph.delivered_by":["plans:P-1","work:I_2"],
               "onetaskgraph.item_kind":"task"}),
        "the list is replaced rather than merged, and every other key stays"
    );
    let read = source_of(&fixture)
        .get_task(&id("I_1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.delivered_by, refs(&["plans:P-1", "work:I_2"]));
    assert_eq!(read.delivers, refs(&["I_9"]));
    assert_eq!(
        read.metadata,
        BTreeMap::from([("caller.x".to_owned(), json!(1))])
    );
    assert_eq!(read.title, "a task");
    assert_eq!(read.status, status(StatusCategory::Todo, "Todo"));
    assert_eq!(read.labels.len(), 1);
}

#[tokio::test]
async fn delivered_by_adds_a_slot_where_there_was_none_and_removes_one_it_empties() {
    let only = json!({"onetaskgraph.delivered_by":["plans:P-1"]});
    let slot_alone = format!("<!-- onetaskgraph.metadata\n{only}\n-->");
    let fixture = board(vec![
        Item::issue("I_prose", "prose only").body("Just prose.\n"),
        Item::issue("I_empty", "no body"),
        Item::issue("I_emptied", "slot emptied").body(&slotted("Kept prose.", &only)),
        Item::issue("I_bare", "slot alone").body(&slot_alone),
    ]);
    let source = source(&fixture);
    for (held, entries, stored) in [
        (
            "I_prose",
            refs(&["plans:P-1"]),
            format!("Just prose.\n\n\n{slot_alone}"),
        ),
        ("I_empty", refs(&["plans:P-1"]), slot_alone.clone()),
        ("I_emptied", vec![], "Kept prose.".to_owned()),
        ("I_bare", vec![], String::new()),
    ] {
        source
            .set_delivered_by(&id(held), &entries)
            .await
            .unwrap()
            .expect("a task of this board");
        assert_eq!(
            fixture.item(held).body.as_deref(),
            Some(stored.as_str()),
            "{held}"
        );
        assert_eq!(
            source_of(&fixture)
                .get_task(&id(held))
                .await
                .unwrap()
                .unwrap()
                .delivered_by,
            entries,
            "{held}"
        );
    }
}

#[tokio::test]
async fn delivered_by_already_as_asked_sends_nothing() {
    let fixture = board(vec![
        Item::issue("I_1", "held").body(&slotted(
            "prose",
            &json!({"onetaskgraph.delivered_by":["plans:P-1"]}),
        )),
        Item::issue("I_2", "none").body("prose"),
    ]);
    let source = source(&fixture);
    source
        .set_delivered_by(&id("I_1"), &refs(&["plans:P-1"]))
        .await
        .unwrap();
    source.set_delivered_by(&id("I_2"), &[]).await.unwrap();
    assert!(fixture.seen().is_empty(), "{:?}", fixture.seen());
}

#[tokio::test]
async fn delivered_by_naming_its_own_task_or_one_task_twice_is_refused_and_no_task_is_none() {
    let fixture = board(vec![
        Item::issue("I_1", "a task"),
        Item::issue("I_plan", "a plan").sub_issues(1),
        design("I_doc", "notes"),
    ]);
    let source = source(&fixture);
    for (entries, entry) in [
        (refs(&["work:I_1"]), "work:I_1"),
        (refs(&["I_1"]), "I_1"),
        (refs(&["plans:P-1", "plans:P-1"]), "plans:P-1"),
    ] {
        let error = source
            .set_delivered_by(&id("I_1"), &entries)
            .await
            .expect_err("the list is refused");
        assert!(matches!(error, SourceError::Refused { .. }), "{error:?}");
        let message = error.to_string();
        assert!(message.contains(TaskRef::DELIVERED_BY_KEY), "{message}");
        assert!(message.contains("task I_1"), "{message}");
        assert!(message.contains(entry), "{message}");
    }
    assert!(
        fixture.documents().is_empty(),
        "nothing is read before the list is checked"
    );
    for held in ["I_plan", "I_doc", "I_missing"] {
        assert_eq!(
            source
                .set_delivered_by(&id(held), &refs(&["plans:P-1"]))
                .await
                .unwrap(),
            None,
            "{held} is no task of this board"
        );
    }
    assert!(fixture.seen().is_empty(), "nothing was written");
}

#[tokio::test]
async fn a_drafts_delivered_by_is_written_through_the_draft_update_with_its_body_alone() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft")
            .body("Draft prose.")
            .status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .set_delivered_by(&id("D_1"), &refs(&["plans:P-1"]))
        .await
        .unwrap()
        .expect("a draft is a task of this board");
    let expected = format!(
        "Draft prose.\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        json!({"onetaskgraph.delivered_by":["plans:P-1"]})
    );
    assert_eq!(
        fixture.seen(),
        vec![json!(["updateProjectV2DraftIssue", {"draftIssueId":"D_1","body":expected}])]
    );
    assert_eq!(fixture.item("D_1").title, "a draft");
    assert_eq!(
        source_of(&fixture)
            .get_task(&id("D_1"))
            .await
            .unwrap()
            .unwrap()
            .delivered_by,
        refs(&["plans:P-1"])
    );
}

fn key(name: &str) -> MetadataKey {
    MetadataKey::new(name).expect("a caller's own key")
}

/// A task, a project and a design document, each holding a body a narrow metadata write
/// has to keep byte for byte outside its slot.
fn metadata_board() -> Fixture {
    board(vec![
        Item::issue("I_task", "a task")
            .body(
                "Prose with a `<!-- comment -->` in it.\n\n\n  and trailing spaces  \n\n\
                 <!-- onetaskgraph.metadata\n{ \"caller.x\" : 1, \"onetaskgraph.delivers\": \
                 [\"I_9\"], \"onetaskgraph.item_kind\":\"task\" }\n-->",
            )
            .labelled(&[("L_1", "bug")])
            .status("Todo"),
        Item::issue("I_plan", "a plan")
            .body(
                "Plan prose.\n\n<!-- onetaskgraph.metadata\n{\"myapp.stage\":\"draft\",\
                 \"onetaskgraph.item_kind\":\"project\",\"other.kept\":[true]}\n-->",
            )
            .status("In Progress"),
        design("I_doc", "notes").body("Design prose."),
    ])
}

#[tokio::test]
async fn the_copy_link_is_kept_in_the_body_slot_and_reads_back_on_every_kind() {
    // `onetaskgraph.copies` is the one reserved key a copy writes through the narrow
    // metadata write, and it is small, so it rides in the body's slot beside the caller's
    // keys rather than in a field of its own.
    let fixture = metadata_board();
    let source = source(&fixture);
    let link = json!({"notes": "notes:T-1"});

    let task = source
        .set_task_metadata(&id("I_task"), &MetadataKey::copies(), &link)
        .await
        .expect("a task's link is writable")
        .expect("a task of this board");
    let project = source
        .set_project_metadata(&id("I_plan"), &MetadataKey::copies(), &link)
        .await
        .expect("a project's link is writable")
        .expect("a project of this board");
    let document = source
        .set_document_metadata(&id("I_doc"), &MetadataKey::copies(), &link)
        .await
        .expect("a document's link is writable")
        .expect("a document of this board");
    for metadata in [&task.metadata, &project.metadata, &document.metadata] {
        assert_eq!(metadata.get(MetadataKey::COPIES_KEY), Some(&link));
    }
    assert!(
        fixture
            .item("I_doc")
            .body
            .as_deref()
            .is_some_and(|body| body.contains(r#""onetaskgraph.copies":{"notes":"notes:T-1"}"#)),
        "the link is in the body's slot"
    );

    // And a later read — a fresh source, so nothing it wrote is remembered — reads it back.
    let reader = source_of(&fixture);
    let read = reader.get_task(&id("I_task")).await.unwrap().unwrap();
    assert_eq!(read.metadata.get(MetadataKey::COPIES_KEY), Some(&link));
    assert_eq!(read.metadata.get("caller.x"), Some(&json!(1)));
}

#[tokio::test]
async fn a_metadata_key_is_set_by_one_body_update_that_changes_only_the_slot() {
    let fixture = metadata_board();
    let before = source_of(&fixture);
    let task_before = before.get_task(&id("I_task")).await.unwrap().unwrap();
    let project_before = before.get_project(&id("I_plan")).await.unwrap().unwrap();
    let document_before = before.get_document(&id("I_doc")).await.unwrap().unwrap();
    let source = source(&fixture);

    let task = source
        .set_task_metadata(&id("I_task"), &key("myapp.new"), &json!({"a":[1,"b"]}))
        .await
        .expect("a task's metadata is writable")
        .expect("a task of this board");
    let project = source
        .set_project_metadata(&id("I_plan"), &key("myapp.stage"), &json!("shipped"))
        .await
        .expect("a project's metadata is writable")
        .expect("a project of this board");
    let document = source
        .set_document_metadata(&id("I_doc"), &key("myapp.reviewed"), &json!(null))
        .await
        .expect("a document's metadata is writable")
        .expect("a document of this board");

    let task_body = "Prose with a `<!-- comment -->` in it.\n\n\n  and trailing spaces  \n\n\
                     <!-- onetaskgraph.metadata\n{\"caller.x\":1,\"myapp.new\":{\"a\":[1,\"b\"]},\
                     \"onetaskgraph.delivers\":[\"I_9\"],\"onetaskgraph.item_kind\":\"task\"}\n-->";
    let project_body = "Plan prose.\n\n<!-- onetaskgraph.metadata\n{\"myapp.stage\":\"shipped\",\
                        \"onetaskgraph.item_kind\":\"project\",\"other.kept\":[true]}\n-->";
    let document_body =
        "Design prose.\n\n<!-- onetaskgraph.metadata\n{\"myapp.reviewed\":null}\n-->";
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["updateIssue", {"id":"I_task","body":task_body}]),
            json!(["updateIssue", {"id":"I_plan","body":project_body}]),
            json!(["updateIssue", {"id":"I_doc","body":document_body}]),
        ],
        "one body update per write, and no title, label, state or board field request"
    );
    for (held, body) in [
        ("I_task", task_body),
        ("I_plan", project_body),
        ("I_doc", document_body),
    ] {
        assert_eq!(fixture.item(held).body.as_deref(), Some(body), "{held}");
    }

    let mut task_expected = task_before;
    task_expected
        .metadata
        .insert("myapp.new".to_owned(), json!({"a":[1,"b"]}));
    let mut project_expected = project_before;
    project_expected
        .metadata
        .insert("myapp.stage".to_owned(), json!("shipped"));
    let mut document_expected = document_before;
    document_expected
        .metadata
        .insert("myapp.reviewed".to_owned(), json!(null));
    assert_eq!(task, task_expected);
    assert_eq!(project, project_expected);
    assert_eq!(document, document_expected);
    assert_eq!(task.delivers, refs(&["I_9"]), "a reserved slot key is kept");

    let after = source_of(&fixture);
    assert_eq!(
        after.get_task(&id("I_task")).await.unwrap().unwrap(),
        task_expected,
        "the answer is what a fresh read of the task reports"
    );
    assert_eq!(
        after.get_project(&id("I_plan")).await.unwrap().unwrap(),
        project_expected
    );
    assert_eq!(
        after.get_document(&id("I_doc")).await.unwrap().unwrap(),
        document_expected
    );
    assert_eq!(
        source.get_task(&id("I_task")).await.unwrap().unwrap(),
        task_expected,
        "this command's own next read agrees with the write"
    );
}

/// The write here is the composer's and the reads are the parser's, so this is what fails
/// when the two disagree about the separator between prose and slot.
#[tokio::test]
async fn a_metadata_write_keeps_the_visible_bodys_trailing_whitespace_byte_for_byte() {
    for prose in [
        "Design prose.\n",
        "Design prose.\n\n\n",
        "Design prose.  ",
        "Design prose.\t",
        "Design prose. \t\n \n\t\n",
        "Design prose.\n\n",
    ] {
        let fixture = board(vec![
            Item::issue("I_1", "a task").body(prose).status("Todo"),
        ]);
        let before = source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .expect("a task of this board");
        assert_eq!(
            before.content.as_deref(),
            Some(prose),
            "an issue carrying no slot reads as its whole body: {prose:?}"
        );

        let source = source(&fixture);
        let written = source
            .set_task_metadata(&id("I_1"), &key("myapp.k"), &json!(1))
            .await
            .expect("a task's metadata is writable")
            .expect("a task of this board");
        let body = format!("{prose}\n\n<!-- onetaskgraph.metadata\n{{\"myapp.k\":1}}\n-->");
        assert_eq!(
            fixture.seen(),
            vec![json!(["updateIssue", {"id":"I_1","body":body}])],
            "the slot is added after the prose as it was, and nothing of it is trimmed: {prose:?}"
        );
        assert_eq!(fixture.item("I_1").body.as_deref(), Some(body.as_str()));

        let mut expected = before;
        expected.metadata.insert("myapp.k".to_owned(), json!(1));
        assert_eq!(written, expected, "{prose:?}");
        assert_eq!(
            source_of(&fixture)
                .get_task(&id("I_1"))
                .await
                .unwrap()
                .unwrap(),
            expected,
            "a fresh read of the board carries the prose byte for byte: {prose:?}"
        );
        assert_eq!(
            source.get_task(&id("I_1")).await.unwrap().unwrap(),
            expected,
            "and so does this command's own next read: {prose:?}"
        );
    }
}

/// A slot a person spelled by hand, with one newline or none before it, is read off the
/// body without the separator the composer would have put there: the bytes before the slot
/// are the content, and a metadata write replaces the JSON in place and leaves them alone.
#[tokio::test]
async fn a_slot_not_after_the_canonical_separator_leaves_the_bytes_before_it_intact() {
    for prose in ["Hand-spelled.\n", "Hand-spelled.", "Hand-spelled.  \n"] {
        let held = format!("{prose}<!-- onetaskgraph.metadata\n{{\"caller.x\":1}}\n-->");
        let fixture = board(vec![
            Item::issue("I_1", "a task").body(&held).status("Todo"),
        ]);
        let before = source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .expect("a task of this board");
        assert_eq!(before.content.as_deref(), Some(prose), "{held:?}");
        assert_eq!(before.metadata.get("caller.x"), Some(&json!(1)));

        let source = source(&fixture);
        let written = source
            .set_task_metadata(&id("I_1"), &key("myapp.k"), &json!(2))
            .await
            .unwrap()
            .expect("a task of this board");
        let body =
            format!("{prose}<!-- onetaskgraph.metadata\n{{\"caller.x\":1,\"myapp.k\":2}}\n-->");
        assert_eq!(
            fixture.seen(),
            vec![json!(["updateIssue", {"id":"I_1","body":body}])],
            "the JSON is replaced in place and the bytes before the slot are as they were: {held:?}"
        );
        assert_eq!(written.content.as_deref(), Some(prose), "{held:?}");
        assert_eq!(
            source_of(&fixture)
                .get_task(&id("I_1"))
                .await
                .unwrap()
                .unwrap()
                .content
                .as_deref(),
            Some(prose),
            "{held:?}"
        );
    }
}

#[tokio::test]
async fn a_metadata_key_already_holding_the_value_sends_nothing() {
    let fixture = metadata_board();
    let source = source(&fixture);
    let task = source
        .set_task_metadata(&id("I_task"), &key("caller.x"), &json!(1))
        .await
        .unwrap()
        .expect("a task of this board");
    let project = source
        .set_project_metadata(&id("I_plan"), &key("other.kept"), &json!([true]))
        .await
        .unwrap()
        .expect("a project of this board");
    assert_eq!(task.metadata.get("caller.x"), Some(&json!(1)));
    assert_eq!(project.metadata.get("other.kept"), Some(&json!([true])));
    assert!(fixture.seen().is_empty(), "{:?}", fixture.seen());
}

#[tokio::test]
async fn a_metadata_write_naming_no_item_of_that_kind_answers_none_and_writes_nothing() {
    let fixture = metadata_board();
    let source = source(&fixture);
    let value = json!("x");
    let name = key("myapp.k");
    for held in ["I_plan", "I_doc", "I_missing"] {
        assert_eq!(
            source
                .set_task_metadata(&id(held), &name, &value)
                .await
                .unwrap(),
            None,
            "{held} is no task of this board"
        );
    }
    for held in ["I_task", "I_doc", "I_missing"] {
        assert_eq!(
            source
                .set_project_metadata(&id(held), &name, &value)
                .await
                .unwrap(),
            None,
            "{held} is no project of this board"
        );
    }
    for held in ["I_task", "I_plan", "I_missing"] {
        assert_eq!(
            source
                .set_document_metadata(&id(held), &name, &value)
                .await
                .unwrap(),
            None,
            "{held} is no document of this board"
        );
    }
    assert!(fixture.seen().is_empty(), "nothing was written");
}

#[tokio::test]
async fn a_drafts_metadata_key_is_written_through_the_draft_update_with_its_body_alone() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft")
            .body("Draft prose.")
            .status("Todo"),
    ]);
    let source = source(&fixture);
    let task = source
        .set_task_metadata(&id("D_1"), &key("myapp.k"), &json!(2))
        .await
        .unwrap()
        .expect("a draft is a task of this board");
    let expected = "Draft prose.\n\n<!-- onetaskgraph.metadata\n{\"myapp.k\":2}\n-->";
    assert_eq!(
        fixture.seen(),
        vec![json!(["updateProjectV2DraftIssue", {"draftIssueId":"D_1","body":expected}])]
    );
    assert_eq!(task.metadata.get("myapp.k"), Some(&json!(2)));
    assert_eq!(task.title, "a draft");
    assert_eq!(task.content.as_deref(), Some("Draft prose."));
    assert_eq!(
        source.get_task(&id("D_1")).await.unwrap().unwrap(),
        task,
        "this command's own view of the board, which is where a draft is read, is brought up to the write"
    );
}

#[tokio::test]
async fn a_metadata_write_to_a_task_this_command_created_is_what_the_command_reads_next() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "fresh", status(StatusCategory::Todo, "Todo"));
    item.content = Some("prose".to_owned());
    let written = source.write_task(&write(item)).await.unwrap();
    let set = source
        .set_task_metadata(&written, &key("myapp.k"), &json!("v"))
        .await
        .unwrap()
        .expect("a task this command created");
    assert_eq!(set.metadata.get("myapp.k"), Some(&json!("v")));
    assert_eq!(source.get_task(&written).await.unwrap().unwrap(), set);
    assert_eq!(
        source_of(&fixture)
            .get_task(&written)
            .await
            .unwrap()
            .unwrap()
            .metadata,
        set.metadata
    );
}

#[tokio::test]
async fn a_narrow_write_to_an_item_this_command_created_is_what_the_command_reads_next() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "fresh", status(StatusCategory::Todo, "Todo"));
    item.content = Some("prose".to_owned());
    let written = source.write_task(&write(item)).await.unwrap();
    source
        .set_delivered_by(&written, &refs(&["plans:P-1"]))
        .await
        .unwrap()
        .expect("a task this command created");
    let done = source
        .set_task_status(&written, StatusCategory::Done)
        .await
        .unwrap()
        .expect("a task this command created");
    let own = source.get_task(&written).await.unwrap().unwrap();
    let fresh = source_of(&fixture)
        .get_task(&written)
        .await
        .unwrap()
        .unwrap();
    for read in [&own, &fresh] {
        assert_eq!(read.delivered_by, refs(&["plans:P-1"]));
        assert_eq!(read.content.as_deref(), Some("prose"));
        assert_eq!(read.status, done);
    }
    assert_eq!(done, status(StatusCategory::Done, "Done"));
}

#[tokio::test]
async fn a_project_copy_creates_an_issue_files_its_tasks_under_it_and_never_writes_the_board() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("a project is created as an issue");
    let mut child = task("T-1", "First step", status(StatusCategory::Todo, "Todo"));
    child.project = Some(plan.clone());
    let filed = source.write_task(&write(child)).await.unwrap();

    assert_eq!(
        source
            .get_project(&plan)
            .await
            .unwrap()
            .expect("the empty project was readable before it had a task")
            .title,
        "Published roadmap"
    );
    assert_eq!(
        source.get_task(&filed).await.unwrap().unwrap().project,
        Some(plan.clone())
    );
    assert_eq!(fixture.item(&plan.0).sub_issues, 1);
    assert!(
        fixture
            .seen()
            .iter()
            .all(|call| call[0] != "updateProjectV2"),
        "nothing this source does writes the board's own fields"
    );
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addSubIssue" && call[1]["issueId"] == plan.0.as_str())
    );
}

#[tokio::test]
async fn a_task_moved_between_projects_leaves_the_one_it_came_from() {
    let fixture = board(vec![
        Item::issue("I_a", "plan a").sub_issues(1),
        Item::issue("I_b", "plan b")
            .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->"),
        Item::issue("I_task", "a step").parent("I_a").status("Todo"),
    ]);
    let source = source(&fixture);
    let mut moved = task("T", "a step", status(StatusCategory::Todo, "Todo"));
    moved.project = Some(NativeId("I_b".to_owned()));
    moved.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_task".to_owned())),
            item: moved,
            depends_on: vec![],
        })
        .await
        .unwrap();
    assert_eq!(fixture.item("I_task").parent.as_deref(), Some("I_b"));
    let calls = fixture.seen();
    assert!(calls.iter().any(|call| call[0] == "removeSubIssue"));
    assert!(calls.iter().any(|call| call[0] == "addSubIssue"));
}

#[tokio::test]
async fn a_second_copy_of_the_same_item_updates_it_rather_than_duplicating_it() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let first = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .unwrap();
    let mut revised = task(
        "T-1",
        "Publish, revised",
        status(StatusCategory::Todo, "Todo"),
    );
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let second = source
        .write_task(&ItemWrite {
            target: Some(first.clone()),
            item: revised,
            depends_on: vec![],
        })
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(fixture.item(&first.0).title, "Publish, revised");
}

#[tokio::test]
async fn the_copy_origin_is_kept_in_the_boards_own_text_field_and_mirrored_in_the_slot() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.content = Some("the prose a person wrote".to_owned());
    item.metadata = BTreeMap::from([
        ("onetaskgraph.origin".to_owned(), json!("notes:T-1")),
        ("team.owner".to_owned(), json!("ada")),
    ]);
    let id = source.write_task(&write(item)).await.unwrap();
    let held = fixture.item(&id.0);
    assert_eq!(
        held.origin.as_deref(),
        Some("notes:T-1"),
        "the field holds it"
    );
    let body = held.body.clone().unwrap_or_default();
    let slot = raw_slot(&body);
    assert_eq!(
        slot["onetaskgraph.origin"],
        json!("notes:T-1"),
        "the slot mirrors exactly the field's value, so the issue search can find it: {body}"
    );
    assert!(
        body.starts_with("the prose a person wrote\n\n<!-- onetaskgraph.metadata\n"),
        "the mirror is in the slot and never in the caller's own prose: {body}"
    );
    let read = source.get_task(&id).await.unwrap().unwrap();
    assert_eq!(read.content.as_deref(), Some("the prose a person wrote"));
    assert_eq!(read.metadata["onetaskgraph.origin"], json!("notes:T-1"));
    assert_eq!(read.metadata["team.owner"], json!("ada"));

    // The release before this one reads the slot, drops the keys it treats as an encoding,
    // and puts the field's origin over whatever the slot held under that key — so it sees
    // one origin, the field's.
    let before = read_as_the_release_before(&held);
    assert_eq!(before["onetaskgraph.origin"], json!("notes:T-1"));
    assert_eq!(before.get("team.owner"), Some(&json!("ada")));
    assert_eq!(
        before
            .keys()
            .filter(|key| key.contains("origin"))
            .collect::<Vec<_>>(),
        ["onetaskgraph.origin"],
        "exactly one origin: {before:?}"
    );
}

#[tokio::test]
async fn the_origin_field_is_the_origin_whatever_the_slot_says() {
    // A slot that disagrees with the field — a hand edit, or a field someone cleared — is
    // never read as a second origin: the field decides, and an item whose field holds none
    // has none.
    let fixture = board(vec![
        Item::issue("I_edited", "edited")
            .carrying("notes:T-1")
            .body(&slotted(
                "prose",
                &json!({"onetaskgraph.origin": "notes:T-2"}),
            )),
        Item::issue("I_cleared", "cleared")
            .holding_no_origin_value()
            .body(&slotted(
                "prose",
                &json!({"onetaskgraph.origin": "notes:T-3"}),
            )),
    ]);
    let source = source(&fixture);
    let origin = |id: &str| {
        let source = &source;
        let id = NativeId(id.to_owned());
        async move {
            source
                .get_task(&id)
                .await
                .unwrap()
                .unwrap()
                .metadata
                .get("onetaskgraph.origin")
                .cloned()
        }
    };
    assert_eq!(origin("I_edited").await, Some(json!("notes:T-1")));
    assert_eq!(origin("I_cleared").await, None);
    // And neither is answered by an origin query naming what the slot holds.
    for mirrored in ["notes:T-2", "notes:T-3"] {
        assert_eq!(
            selected_tasks(source.as_ref(), &origin_query(mirrored)).await,
            Vec::<String>::new(),
            "{mirrored}"
        );
    }
}

#[tokio::test]
async fn a_board_without_the_origin_field_refuses_an_item_that_carries_one() {
    let fixture = board_with(vec![], true, false);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    let message = refusal(source.write_task(&write(item)).await.expect_err("no field"));
    assert!(message.contains("onetaskgraph.origin"), "{message}");
}

#[tokio::test]
async fn write_refusals_name_stale_targets_and_labels_this_destination_cannot_carry() {
    let fixture = board(vec![Item::issue("I_1", "held").labelled(&[("L_1", "bug")])]);
    let source = source(&fixture);
    let stale = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("I_missing".to_owned())),
                item: task("T", "x", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![],
            })
            .await
            .expect_err("a target the destination no longer holds"),
    );
    assert!(stale.contains("I_missing"), "{stale}");

    let created = refusal(
        source
            .write_task(&write(Task {
                labels: vec![Label {
                    id: NativeId("L_1".to_owned()),
                    name: "bug".to_owned(),
                    color: None,
                }],
                ..task("T", "x", status(StatusCategory::Todo, "Todo"))
            }))
            .await
            .expect_err("creation carries no labels"),
    );
    assert!(created.contains("labels"), "{created}");

    let mismatched = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: task("T", "x", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![],
            })
            .await
            .expect_err("labels differ from the ones held"),
    );
    assert!(mismatched.contains("labels"), "{mismatched}");
}

#[tokio::test]
async fn a_draft_item_is_a_task_this_destination_updates_but_never_closes() {
    let fixture = board(vec![Item::draft("D_1", "a draft").status("Todo")]);
    let source = source(&fixture);
    let held = source
        .get_task(&NativeId("D_1".to_owned()))
        .await
        .unwrap()
        .expect("a draft reads as a task");
    assert_eq!(held.project, None);
    assert!(held.repositories.is_empty());

    let mut revised = task(
        "D_1",
        "a revised draft",
        status(StatusCategory::Todo, "Todo"),
    );
    revised.content = Some("prose".to_owned());
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("D_1".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("a draft's visible fields are writable");
    assert_eq!(fixture.item("D_1").title, "a revised draft");

    let closed = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("D_1".to_owned())),
                item: task("D_1", "done", status(StatusCategory::Done, "Done")),
                depends_on: vec![],
            })
            .await
            .expect_err("a draft has no open or closed state"),
    );
    assert!(closed.contains("draft"), "{closed}");

    let filed = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("D_1".to_owned())),
                item: Task {
                    project: Some(NativeId("I_plan".to_owned())),
                    ..task("D_1", "x", status(StatusCategory::Todo, "Todo"))
                },
                depends_on: vec![],
            })
            .await
            .expect_err("a draft cannot be a sub-issue"),
    );
    assert!(filed.contains("sub-issue"), "{filed}");
}

/// Every edge one direction reports, walked to exhaustion.
///
/// The native connection is answered first and the recorded tail resumes under a cursor of
/// its own, so a caller that stops at the first page has read half the answer.
async fn walk(
    source: &dyn TaskSource,
    id: &str,
    kind: ItemKind,
    direction: Direction,
    limit: u32,
) -> Result<Vec<DependencyEdge>, SourceError> {
    let mut cursor = None;
    let mut edges = Vec::new();
    loop {
        let request = match cursor {
            None => page(limit),
            Some(Cursor(ref cursor)) => resume(cursor, limit),
        };
        let id = NativeId(id.to_owned());
        let read = match kind {
            ItemKind::Task => source.task_dependencies(&id, direction, &request).await?,
            ItemKind::Project => {
                source
                    .project_dependencies(&id, direction, &request)
                    .await?
            }
        };
        edges.extend(read.items);
        match read.next {
            Some(next) => cursor = Some(next),
            None => return Ok(edges),
        }
    }
}

fn edge(from: (&str, ItemKind), to: (&str, ItemKind)) -> DependencyEdge {
    DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId(from.0.to_owned()), from.1),
        to: DependencyEndpoint::from_native(NativeId(to.0.to_owned()), to.1),
        kind: DependencyKind::Blocks,
    }
}

#[tokio::test]
async fn project_dependencies_are_answered_by_the_issues_own_blocked_by() {
    // The aggregate walk over `projectItems` existed only because one board was one
    // project. With project issues the native relationship answers directly.
    let fixture = board(vec![
        Item::issue("I_p1", "plan one").sub_issues(1),
        Item::issue("I_p2", "plan two").sub_issues(1),
        Item::issue("I_t1", "step").parent("I_p1").status("Todo"),
        Item::issue("I_t2", "step").parent("I_p2").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_project(&ItemWrite {
            target: Some(NativeId("I_p1".to_owned())),
            item: Project {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..project("P", "plan one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![edge(
                ("I_p1", ItemKind::Project),
                ("I_p2", ItemKind::Project),
            )],
        })
        .await
        .expect("a project dependency is native");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == "I_p2")
    );
    let forward = source
        .project_dependencies(
            &NativeId("I_p1".to_owned()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        forward.items,
        vec![edge(
            ("I_p1", ItemKind::Project),
            ("I_p2", ItemKind::Project)
        )]
    );
    let reverse = source
        .project_dependencies(
            &NativeId("I_p2".to_owned()),
            Direction::DependedOnBy,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        reverse.items, forward.items,
        "one relationship reads the same from either end"
    );
}

#[tokio::test]
async fn a_task_dependency_reports_the_far_ends_own_kind() {
    let fixture = board(vec![
        Item::issue("I_plan", "plan").sub_issues(1),
        Item::issue("I_task", "step")
            .parent("I_plan")
            .status("Todo"),
        Item::issue("I_other", "another step").status("Todo"),
    ]);
    let source = source(&fixture);
    let mut item = task("T", "step", status(StatusCategory::Todo, "Todo"));
    item.project = Some(NativeId("I_plan".to_owned()));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_task".to_owned())),
            item,
            depends_on: vec![edge(
                ("I_task", ItemKind::Task),
                ("I_other", ItemKind::Task),
            )],
        })
        .await
        .unwrap();
    let forward = source
        .task_dependencies(
            &NativeId("I_task".to_owned()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        forward.items,
        vec![edge(
            ("I_task", ItemKind::Task),
            ("I_other", ItemKind::Task)
        )]
    );
}

#[tokio::test]
async fn a_far_end_no_issue_relationship_can_name_is_read_from_the_reserved_key() {
    let fixture = board(vec![
        Item::issue("I_1", "step").status("Todo").body(
            "<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[{\"id\":\"elsewhere:P-9\",\"kind\":\"project\"}]}\n-->",
        ),
    ]);
    let source = source(&fixture);
    let forward = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .unwrap();
    assert_eq!(forward.len(), 1);
    assert_eq!(forward[0].to.id(), "elsewhere:P-9");
    assert_eq!(forward[0].to.kind, ItemKind::Project);
    assert!(
        walk(
            source.as_ref(),
            "I_1",
            ItemKind::Task,
            Direction::DependedOnBy,
            10
        )
        .await
        .unwrap()
        .is_empty(),
        "the reverse of a recorded edge belongs to the far end"
    );
}

#[tokio::test]
async fn an_item_may_not_record_a_far_end_its_own_relationship_can_name() {
    for (recorded, near, kind, problem) in [
        (json!(["I_2"]), "I_1", ItemKind::Task, "relate natively"),
        (
            json!([{"id":"work:I_2","kind":"task"}]),
            "I_1",
            ItemKind::Task,
            "relate natively",
        ),
        (
            json!({"id":"elsewhere:P-9"}),
            "I_1",
            ItemKind::Task,
            "not a list of dependency endpoints",
        ),
        (
            json!([{"id":"bad source:P-9","kind":"project"}]),
            "I_1",
            ItemKind::Task,
            "source name",
        ),
    ] {
        let body = format!(
            "<!-- onetaskgraph.metadata\n{}\n-->",
            json!({ "onetaskgraph.depends_on": recorded })
        );
        let fixture = board(vec![
            Item::issue("I_1", "step").status("Todo").body(&body),
            Item::issue("I_2", "other").status("Todo"),
        ]);
        let source = source(&fixture);
        let message = refusal(
            match kind {
                ItemKind::Task => {
                    source
                        .task_dependencies(
                            &NativeId(near.to_owned()),
                            Direction::DependsOn,
                            &page(10),
                        )
                        .await
                }
                ItemKind::Project => {
                    source
                        .project_dependencies(
                            &NativeId(near.to_owned()),
                            Direction::DependsOn,
                            &page(10),
                        )
                        .await
                }
            }
            .expect_err("a reserved key holding what it must not"),
        );
        assert!(message.contains(problem), "{recorded}: {message}");
        assert!(message.contains("onetaskgraph.depends_on"), "{message}");
    }
}

#[tokio::test]
async fn a_draft_may_record_the_far_end_an_issue_may_not() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft")
            .status("Todo")
            .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[\"I_2\"]}\n-->"),
        Item::issue("I_2", "other").status("Todo"),
    ]);
    let source = source(&fixture);
    let edges = walk(
        source.as_ref(),
        "D_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .expect("a backend with no relationship at all records anything");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to.id(), "I_2");
}

#[tokio::test]
async fn a_recorded_tail_pages_and_refuses_a_cursor_no_reverse_walk_issues() {
    let recorded = json!({"onetaskgraph.depends_on":[
        {"id":"elsewhere:A","kind":"task"},
        {"id":"elsewhere:B","kind":"task"}
    ]});
    let fixture =
        board(vec![Item::issue("I_1", "step").status("Todo").body(
            &format!("<!-- onetaskgraph.metadata\n{recorded}\n-->"),
        )]);
    let source = source(&fixture);
    let walked = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        walked
            .iter()
            .map(|edge| edge.to.id().to_owned())
            .collect::<Vec<_>>(),
        ["elsewhere:A", "elsewhere:B"]
    );
    // The recorded tail is read out of the issue's own body, which the dependency read
    // already carries — not out of a walk of every item on the board.
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "reading an issue's recorded edges read the whole board"
    );
    let cursor = source
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(1))
        .await
        .unwrap()
        .next
        .expect("a recorded tail resumes");

    let message = refusal(
        source
            .task_dependencies(
                &NativeId("I_1".to_owned()),
                Direction::DependedOnBy,
                &resume(&cursor.0, 1),
            )
            .await
            .expect_err("a reverse read never issues a recorded cursor"),
    );
    assert!(message.contains("resume it in the direction"), "{message}");
    for invalid in ["onetaskgraph.depends_on:nope"] {
        assert!(
            source
                .task_dependencies(
                    &NativeId("I_1".to_owned()),
                    Direction::DependsOn,
                    &resume(invalid, 1)
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn a_dependency_write_refuses_a_far_end_of_a_kind_this_board_says_it_is_not() {
    // The caller names the far end's kind and the board holds the far end itself, so a
    // disagreement is settled rather than stored: `I_2` has sub-issues, which is what
    // makes it a project, and an edge naming it a task would otherwise be written as a
    // native `blockedBy` link standing for a relationship at a level it is not at.
    let fixture = board(vec![
        Item::issue("I_1", "step").status("Todo"),
        Item::issue("I_2", "plan").status("Todo").sub_issues(1),
    ]);
    let message = refusal(
        source(&fixture)
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: task("T", "step", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task))],
            })
            .await
            .expect_err("a far end the board says is a project"),
    );
    assert!(
        message.contains("I_2") && message.contains("project") && message.contains("task"),
        "the entry and both kinds are named: {message}"
    );
}

#[tokio::test]
async fn a_dependency_write_refuses_a_same_source_far_end_the_board_does_not_hold() {
    let fixture = board(vec![Item::issue("I_1", "step").status("Todo")]);
    let message = refusal(
        source(&fixture)
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: Task {
                    repositories: vec![
                        Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                    ],
                    ..task("T", "step", status(StatusCategory::Todo, "Todo"))
                },
                depends_on: vec![edge(("I_1", ItemKind::Task), ("I_gone", ItemKind::Task))],
            })
            .await
            .expect_err("a far end this board does not hold"),
    );
    assert!(message.contains("I_gone"), "{message}");
}

#[tokio::test]
async fn a_same_source_far_end_keeps_every_colon_its_own_native_id_holds() {
    // A qualified id is `<source>:<native>` split at its *first* colon, and a GitHub node id
    // is opaque, so the native half may hold colons of its own. Split at the last one, this
    // far end would be looked up as `2`, and an edge to an item this board holds would be
    // refused as missing.
    let fixture = board(vec![
        Item::issue("I_1", "one").status("Todo"),
        Item::issue("I:urn:2", "two").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: Task {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..task("T", "one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("I_1".to_owned()), ItemKind::Task),
                to: DependencyEndpoint::new("work:I:urn:2".to_owned(), ItemKind::Task).unwrap(),
                kind: DependencyKind::Blocks,
            }],
        })
        .await
        .expect("a same-source far end this board holds is written natively");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == "I:urn:2"),
        "the whole native id is the far end: {:?}",
        fixture.seen()
    );
    assert!(
        !fixture
            .item("I_1")
            .body
            .unwrap_or_default()
            .contains("work:I:urn:2"),
        "a far end this board holds is linked natively rather than recorded"
    );
}

#[tokio::test]
async fn dependencies_of_an_item_nothing_holds_are_refused_rather_than_empty() {
    let fixture = board(vec![]);
    let message = refusal(
        source(&fixture)
            .task_dependencies(
                &NativeId("I_missing".to_owned()),
                Direction::DependsOn,
                &page(10),
            )
            .await
            .expect_err("a dependency read is never silently empty"),
    );
    assert!(message.contains("I_missing"), "{message}");
}

#[tokio::test]
async fn tasks_projects_and_labels_page_to_exhaustion_in_a_stable_order() {
    let fixture = board(vec![
        Item::issue("I_p", "plan").sub_issues(2),
        Item::issue("I_1", "one")
            .parent("I_p")
            .labelled(&[("L_a", "alpha")]),
        Item::issue("I_2", "two")
            .parent("I_p")
            .labelled(&[("L_b", "beta")]),
    ]);
    let source = source(&fixture);
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let request = cursor.map_or_else(|| page(1), |cursor: Cursor| resume(&cursor.0, 1));
        let page = source
            .query_tasks(&TaskQuery::default(), &request)
            .await
            .unwrap();
        walked.extend(page.items.into_iter().map(|task| task.id.0));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, ["I_1", "I_2"]);

    let labels = source.labels(&page(10)).await.unwrap();
    assert_eq!(
        labels
            .items
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(1))
        .await
        .unwrap();
    assert_eq!(projects.items.len(), 1);
    assert!(projects.next.is_none(), "one project fits one page");
}

#[tokio::test]
async fn a_board_larger_than_one_page_is_walked_before_it_is_answered() {
    let fixture = board(
        (1..=5)
            .map(|n| Item::issue(&format!("I_{n}"), "step").status("Todo"))
            .collect(),
    );
    let source = source(&fixture);
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(100))
            .await
            .unwrap()
            .items
            .len(),
        5
    );
    assert!(
        source
            .get_task(&NativeId("I_5".to_owned()))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_zero_limit_and_a_nonsense_cursor_are_refused() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(0))
            .await
            .is_err()
    );
    assert!(source.labels(&page(0)).await.is_err());
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page(0))
            .await
            .is_err()
    );
    assert!(
        source
            .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(0))
            .await
            .is_err()
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &resume("not-a-number", 5))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn health_names_the_board_it_read_and_the_source_declares_what_it_applies() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let health = source.health().await.unwrap();
    assert!(health.reachable);
    assert!(health.detail.unwrap().contains("Roadmap"));
    assert_eq!(source.kind(), onetaskgraph_github_projects::KIND);
    assert_eq!(source.writes(), WriteSupport::Supported);
    // Every field, rather than a subset: this source applies every predicate a query
    // carries, in process, over a board it has already walked in full, and GitHub's
    // project-items connection has no filter argument to push any of them into.
    // `documents` is not one of those predicates — it says this board has documents, which
    // it does: an issue whose title begins with the design prefix is one.
    assert_eq!(
        source.capabilities(),
        Capabilities {
            projects: Support::Native,
            documents: Support::Native,
            comments: Support::Native,
            assets: Support::Native,
            priority: Support::Unsupported,
            filter_by_priority: Support::Native,
            filter_by_comment_activity: Support::Native,
            filter_by_metadata: Support::Native,
            filter_by_origin: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: onetaskgraph_github_projects::MAX_PAGE_SIZE,
        }
    );
    assert_eq!(
        onetaskgraph_github_projects::MAX_PAGE_SIZE,
        100,
        "the declared page size is GitHub's own connection maximum"
    );
}

#[test]
fn the_config_schema_is_strict_and_build_validates_every_input() {
    let schema = serde_json::to_value(Plugin.config_schema()).unwrap();
    assert_eq!(schema["additionalProperties"], json!(false));
    for key in ["owner", "project_number", "repository", "status_mapping"] {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }
    for invalid in [
        json!({"owner":"","project_number":7}),
        json!({"owner":"-bad","project_number":7}),
        json!({"owner":"octo","project_number":0}),
        json!({"owner":"octo","project_number":7,"token_env":"1BAD"}),
        json!({"owner":"octo","project_number":7,"endpoint":"not a url"}),
        json!({"owner":"octo","project_number":7,"endpoint":"http://example.invalid/graphql"}),
        json!({"owner":"octo","project_number":7,"repository":"nameless"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/a name"}),
        json!({"owner":"octo","project_number":7,"repository":"acme/.."}),
        json!({"owner":"octo","project_number":7,"repository":"acme/a/b"}),
        json!({"owner":"octo","project_number":7,
               "repository":format!("acme/{}", "n".repeat(101))}),
        json!({"owner":"octo","project_number":7,"unknown":true}),
    ] {
        assert!(
            Plugin
                .build(&SourceName::new("work").unwrap(), &invalid, &Secrets)
                .is_err(),
            "{invalid}"
        );
    }
    struct NoSecret;
    impl SecretResolver for NoSecret {
        fn get(&self, _: &str) -> Option<SecretString> {
            None
        }
    }
    let message = match Plugin.build(
        &SourceName::new("work").unwrap(),
        &json!({"owner":"octo","project_number":7}),
        &NoSecret,
    ) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("a missing credential is refused"),
    };
    assert!(message.contains("GH_PROJECTS_TOKEN"), "{message}");
    assert!(!message.contains("test-token"), "{message}");
}

#[tokio::test]
async fn transport_http_json_and_graphql_failures_each_reach_the_caller_intact() {
    let cases: Vec<(String, &str)> = vec![
        (
            raw_server_with_headers("429 Too Many Requests", "{}", "retry-after: 30\r\n"),
            "rate",
        ),
        (
            raw_server_with_headers("403 Forbidden", "{}", "x-ratelimit-remaining: 0\r\n"),
            "rate",
        ),
        (raw_server("401 Unauthorized", "{}"), "credential"),
        (raw_server("500 Internal Server Error", "{}"), "HTTP"),
        (raw_server("200 OK", "not json"), "invalid JSON"),
        (
            raw_server("200 OK", r#"{"errors":{"message":"x"}}"#),
            "not an array",
        ),
        (
            raw_server("200 OK", r#"{"errors":[{"message":"boom"}]}"#),
            "boom",
        ),
        (
            raw_server(
                "200 OK",
                r#"{"errors":[{"message":"Resource not accessible"}]}"#,
            ),
            "grant",
        ),
        (raw_server("200 OK", r#"{"errors":[]}"#), "no data object"),
        (
            raw_server("200 OK", r#"{"data":{"owner":null}}"#),
            "was not found",
        ),
        (
            raw_server(
                "200 OK",
                r#"{"data":{"owner":{"projectV2":{"title":"T","fields":{"nodes":[],"pageInfo":{"hasNextPage":false}},"items":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}}"#,
            ),
            "missing string field id",
        ),
    ];
    for (endpoint, expected) in cases {
        let message = refusal(
            configured(&endpoint, json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase()),
            "expected {expected} in {message}"
        );
    }
    assert!(
        configured("http://127.0.0.1:1/graphql", json!({}))
            .health()
            .await
            .is_err()
    );
    let untitled = raw_server(
        "200 OK",
        r#"{"data":{"owner":{"projectV2":{"id":"B","fields":{"nodes":[],"pageInfo":{"hasNextPage":false}},"items":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}}"#,
    );
    let message = refusal(
        configured(&untitled, json!({}))
            .health()
            .await
            .expect_err("a board with no title"),
    );
    assert!(message.contains("missing string field title"), "{message}");
}

#[tokio::test]
async fn malformed_board_shapes_are_named_rather_than_guessed_at() {
    let board = |items: Value, fields: Value| json!({"data":{"owner":{"projectV2":{"id":"B","title":"T","fields":fields,"items":items}}}});
    let complete = json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
    let cases = [
        (
            board(
                json!({"nodes":"no","pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "items.nodes is not an array",
        ),
        (board(json!({"nodes":[]}), complete.clone()), "no pageInfo"),
        (
            board(
                json!({"nodes":[{"id":"PVTI"}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "missing content",
        ),
        (
            board(
                json!({"nodes":[{"id":"PVTI","content":{"__typename":"Issue","id":"I","number":1043,"subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "missing fieldValues",
        ),
        (
            board(
                json!({"nodes":[{"id":"PVTI","fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":true}},
                                 "content":{"__typename":"Issue","id":"I","number":1043,"subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "exceeds the supported nested connection size",
        ),
        (
            board(
                json!({"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":""}}),
                complete.clone(),
            ),
            "did not advance",
        ),
    ];
    for (body, expected) in cases {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_mutation_that_answers_about_another_item_is_refused_as_malformed() {
    let complete = json!({"nodes":[{"__typename":"ProjectV2SingleSelectField","id":"FIELD_status",
                                    "name":"Status","options":[{"id":"OPT_todo","name":"Todo"}]}],
                          "pageInfo":{"hasNextPage":false}});
    // What a create reads first: the board's fields with the repository's id.
    let empty_board = with_repository(fields_json("B", complete));
    for (bodies, expected) in [
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":null}}}),
            ],
            "returned no issue",
        ),
        // Filed by hand once created, and a filing that answers with no item is refused too.
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":{"id":"I_new"}}}}),
                json!({"data":{"addProjectV2ItemById":{"item":null}}}),
            ],
            "returned no project item",
        ),
    ] {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

fn board_json(fields: Value, items: Value) -> Value {
    json!({"data":{"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap",
        "fields":fields,"items":items}}}})
}

fn complete(nodes: Value) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}})
}

fn usable_fields() -> Value {
    complete(json!([
        {"__typename":"ProjectV2SingleSelectField","id":"FIELD_status","name":"Status",
         "options":[{"id":"OPT_todo","name":"Todo"}]},
        {"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}
    ]))
}

fn issue_item(content: Value) -> Value {
    json!({"id":"PVTI_1","fieldValues":complete(json!([])),"content":content})
}

fn fields_json(id: &str, fields: Value) -> Value {
    json!({"data":{"boardFields":{"projectV2":{"id":id,"fields":fields}}}})
}
/// [`fields_json`] answered with the repository a create is for beside it, as the one read a
/// create makes of what it needs answers.
fn with_repository(mut fields: Value) -> Value {
    fields["data"]["repository"] = json!({"id":"R","nameWithOwner":"acme/work"});
    fields
}
/// One issue's own node read, placing it on the configured board and holding no field
/// values — so what a write needs of the board's fields comes from [`fields_json`].
fn held_issue(id: &str, title: &str, parent: Option<&str>) -> Value {
    json!({"data":{"node":{"__typename":"Issue","id":id,"number":1043,"title":title,"body":"",
        "state":"OPEN","stateReason":null,"repository":{"nameWithOwner":"acme/work"},
        "parent":parent.map(|parent| json!({"id":parent})),"subIssuesSummary":{"total":0},
        "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}},
        "projectItems":{"nodes":[{"id":format!("PVTI_{}", id.trim_start_matches("I_")),
                                  "project":{"id":"PVT_board","number":7},
                                  "fieldValues":complete(json!([]))}],
                        "pageInfo":{"hasNextPage":false,"endCursor":null}}}}})
}
fn plain_issue() -> Value {
    json!({"__typename":"Issue","id":"I_1","number":1043,"title":"one","body":"","state":"OPEN",
           "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
           "parent":null,"subIssuesSummary":{"total":0},
           "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}})
}

#[tokio::test]
async fn every_board_shape_this_source_will_not_guess_at_is_named() {
    let cases = [
        (
            board_json(
                usable_fields(),
                complete(
                    json!([{"id":"PVTI_1","fieldValues":{"nodes":"no","pageInfo":{"hasNextPage":false}},"content":plain_issue()}]),
                ),
            ),
            "fieldValues.nodes is not an array",
        ),
        (
            board_json(
                usable_fields(),
                complete(
                    json!([{"id":"PVTI_1","fieldValues":{"nodes":[]},"content":plain_issue()}]),
                ),
            ),
            "has no pageInfo",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":7})
                )])),
            ),
            "field body is not a string or null",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","number":1043,"title":"one","body":"",
                    "createdAt":"not-a-time","subIssuesSummary":{"total":0}})
                )])),
            ),
            "is not a timestamp",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","number":1043,"title":"one","body":"",
                    "subIssuesSummary":{"total":0},
                    "labels":{"nodes":"no","pageInfo":{"hasNextPage":false}}})
                )])),
            ),
            "content labels.nodes is not an array",
        ),
        (
            board_json(usable_fields(), json!({"nodes":[],"pageInfo":{}})),
            "missing boolean field hasNextPage",
        ),
        // An issue with no number. GitHub declares `Issue.number` as `Int!`, so this is a
        // shape that cannot come back from the real API — and reading it as *an issue with
        // no handle* would be guessing, which is what every case in this list exists to
        // refuse. Only a draft has no number, and a draft is decided on `__typename`
        // before this is ever read.
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":"",
                    "subIssuesSummary":{"total":0},
                    "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}})
                )])),
            ),
            "GitHub issue number is missing or is not an unsigned integer",
        ),
        // And one whose number is a string, which is the other half: present is not the
        // same as readable.
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","number":"1043","title":"one",
                    "body":"","subIssuesSummary":{"total":0},
                    "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}})
                )])),
            ),
            "GitHub issue number is missing or is not an unsigned integer",
        ),
    ];
    for (body, expected) in cases {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn an_item_whose_content_the_token_cannot_see_is_left_out_rather_than_guessed_at() {
    let body = board_json(
        usable_fields(),
        complete(json!([
            {"id":"PVTI_hidden","fieldValues":complete(json!([])),"content":null},
            issue_item(plain_issue()),
        ])),
    );
    let source = configured(&raw_server("200 OK", &body.to_string()), json!({}));
    assert_eq!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &resume("99", 10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "a cursor past the end is an empty last page rather than a panic"
    );
}

#[tokio::test]
async fn a_board_answered_in_two_pages_is_walked_before_it_is_answered() {
    let first = json!({"data":{"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap",
        "fields":usable_fields(),
        "items":{"nodes":[issue_item(plain_issue())],"pageInfo":{"hasNextPage":true,"endCursor":"1"}}}}}});
    let mut second_content = plain_issue();
    second_content["id"] = json!("I_2");
    let second = board_json(
        usable_fields(),
        complete(json!([issue_item(second_content)])),
    );
    let endpoint = sequence_server(vec![first, second]);
    assert_eq!(
        configured(&endpoint, json!({}))
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .map(|task| task.id.0.clone())
            .collect::<Vec<_>>(),
        ["I_1", "I_2"]
    );
}

#[tokio::test]
async fn a_graphql_error_with_nothing_to_say_still_says_something() {
    let message = refusal(
        configured(
            &raw_server("200 OK", r#"{"errors":[{"code":9}],"data":null}"#),
            json!({}),
        )
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("an error array with no message"),
    );
    assert!(message.contains("GraphQL errors"), "{message}");
}

#[tokio::test]
async fn a_status_or_origin_field_of_the_wrong_shape_is_refused_by_name() {
    for (fields, expected) in [
        (
            complete(json!([
                {"__typename":"ProjectV2Field","id":"FIELD_status","name":"Status"},
                {"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}
            ])),
            "not a single-select field",
        ),
        (
            complete(json!([
                {"__typename":"ProjectV2SingleSelectField","id":"FIELD_status","name":"Status",
                 "options":[{"id":"OPT_todo","name":"Todo"}]},
                {"__typename":"ProjectV2SingleSelectField","id":"FIELD_origin",
                 "name":"onetaskgraph.origin","options":[]}
            ])),
            "is not a text field",
        ),
        (
            json!({"nodes":"no","pageInfo":{"hasNextPage":false}}),
            "fields.nodes is not an array",
        ),
    ] {
        let endpoint = sequence_server(vec![
            with_repository(fields_json("PVT_board", fields)),
            json!({"data":{"createIssue":{"issue":{"id":"I_new"}}}}),
            json!({"data":{"addProjectV2ItemById":{"item":{"id":"PVTI_new"}}}}),
        ]);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_terminal_category_uses_its_configured_option_and_fixed_close_reason() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"cancelled":"Shipped"}}),
    );
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Cancelled, "Cancelled"),
        )))
        .await
        .unwrap();
    assert_eq!(fixture.item(&id.0).state, "CLOSED");
    assert_eq!(
        fixture.item(&id.0).state_reason.as_deref(),
        Some("NOT_PLANNED")
    );
}

#[tokio::test]
async fn a_terminal_update_whose_close_fails_leaves_the_option_that_landed_visible() {
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    // An existing write sends its Status option, and then the one updateIssue that writes its
    // content and closes it. Refuse that final call.
    fixture.refuse("updateIssue");
    let error = source(&fixture)
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: task("I_1", "one", status(StatusCategory::Done, "Done")),
            depends_on: vec![],
        })
        .await
        .expect_err("the close is refused");
    assert!(refusal(error).contains("updateIssue is refused"));
    let held = fixture.item("I_1");
    assert_eq!(held.status.as_deref(), Some("Done"));
    assert_eq!(held.state, "OPEN");
    assert_eq!(held.state_reason, None);
}

#[tokio::test]
async fn a_far_end_in_another_source_is_recorded_and_a_native_one_is_taken_back_out() {
    let fixture = board(vec![
        Item::issue("I_1", "one").status("Todo"),
        Item::issue("I_2", "two").status("Todo"),
    ]);
    let source = source(&fixture);
    let held = |edges: Vec<DependencyEdge>| ItemWrite {
        target: Some(NativeId("I_1".to_owned())),
        item: Task {
            repositories: vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()],
            ..task("T", "one", status(StatusCategory::Todo, "Todo"))
        },
        depends_on: edges,
    };
    source
        .write_task(&held(vec![
            edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task)),
            DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("I_1".to_owned()), ItemKind::Task),
                to: DependencyEndpoint::new("elsewhere:T-9".to_owned(), ItemKind::Task).unwrap(),
                kind: DependencyKind::Blocks,
            },
        ]))
        .await
        .unwrap();
    assert!(
        fixture.item("I_1").body.unwrap().contains("elsewhere:T-9"),
        "a far end no relationship here can name goes to the reserved key"
    );
    let walked = walk(
        source.as_ref(),
        "I_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .unwrap();
    assert_eq!(
        walked
            .iter()
            .map(|edge| edge.to.id().to_owned())
            .collect::<Vec<_>>(),
        ["I_2", "elsewhere:T-9"]
    );

    // Writing the item again without either edge takes the native one back out and clears
    // the recorded one.
    source.write_task(&held(vec![])).await.unwrap();
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "removeBlockedBy")
    );
    assert!(
        walk(
            source.as_ref(),
            "I_1",
            ItemKind::Task,
            Direction::DependsOn,
            10
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
async fn a_far_end_that_is_a_sub_issue_or_carries_a_broken_marker_is_read_as_it_is() {
    let node = |body: Value| {
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_far","title":"Far work","body":body,"parent":null,
                                   "subIssuesSummary":{"total":0}}],
                        "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}})
    };
    let sub_issue = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[{"id":"I_far","title":"Far work","body":null,"parent":{"id":"I_plan"},
                               "subIssuesSummary":{"total":4}}],
                    "pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let empty_board = board_json(usable_fields(), complete(json!([])));
    let endpoint = sequence_server(vec![sub_issue, empty_board.clone()]);
    let edges = configured(&endpoint, json!({}))
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
        .await
        .unwrap();
    assert_eq!(
        edges.items[0].to.kind,
        ItemKind::Task,
        "a sub-issue is a task however many sub-issues of its own it has"
    );

    let endpoint = sequence_server(vec![node(json!(
        "<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"epic\"}\n-->"
    ))]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
            .await
            .expect_err("a far end whose marker this contract cannot read"),
    );
    assert!(message.contains("I_far"), "{message}");
}

#[tokio::test]
async fn a_dependency_read_for_an_item_no_longer_on_the_board_has_no_recorded_tail() {
    let node = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let endpoint = sequence_server(vec![node, board_json(usable_fields(), complete(json!([])))]);
    let edges = configured(&endpoint, json!({}))
        .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
        .await
        .unwrap();
    assert!(edges.items.is_empty());
    assert!(edges.next.is_none());
}

#[tokio::test]
async fn malformed_dependency_connections_are_named_rather_than_read_as_empty() {
    for (body, expected) in [
        (
            json!({"data":{"node":{"__typename":"Issue"}}}),
            "missing its connection",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue",
                "blockedBy":{"nodes":"no","pageInfo":{"hasNextPage":false}}}}}),
            "nodes is not an array",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[]}}}}),
            "missing pageInfo",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue",
                "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":""}}}}}),
            "did not advance",
        ),
    ] {
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn every_write_mutation_that_answers_about_the_wrong_item_is_refused_as_malformed() {
    let fields = usable_fields();
    let held_one = held_issue("I_1", "one", Some("I_old"));
    let board_fields = fields_json("PVT_board", fields);
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let ok_removal = json!({"data":{"removeSubIssue":{"issue":{"id":"I_old"},
                                                      "subIssue":{"id":"I_1"}}}});
    let no_blockers = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    // An existing item's board fields go first, then its parent and its relationships, and
    // its content last.
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                ok_field.clone(),
                ok_removal.clone(),
                no_blockers.clone(),
                json!({"data":{"updateIssue":{}}}),
            ],
            "item update returned no item",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                ok_field.clone(),
                ok_removal.clone(),
                no_blockers.clone(),
                json!({"data":{"updateIssue":{"issue":{"id":"I_other"}}}}),
            ],
            "item update returned the wrong item",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{}}}),
            ],
            "field update returned no project item",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_other"}}}}),
            ],
            "field update returned the wrong project item",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"issue":{"id":"I_old"}}}}),
            ],
            "sub-issue update returned no sub-issue",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"subIssue":{"id":"I_1"}}}}),
            ],
            "sub-issue update returned no issue",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"issue":{"id":"I_wrong"},"subIssue":{"id":"I_1"}}}}),
            ],
            "sub-issue update returned the wrong issues",
        ),
    ];
    for (bodies, expected) in cases {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&ItemWrite {
                    target: Some(NativeId("I_1".to_owned())),
                    item: Task {
                        repositories: vec![
                            Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                        ],
                        ..task("T", "one", status(StatusCategory::Todo, "Todo"))
                    },
                    depends_on: vec![],
                })
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_malformed_dependency_mutation_or_reconciliation_read_is_refused() {
    let held_one = held_issue("I_1", "one", None);
    let board_fields = fields_json("PVT_board", usable_fields());
    let held_two = held_issue("I_2", "two", None);
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let held = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    // The item's own content is written last, after its relationship, so a refusal of the
    // relationship is reached with nothing of the content sent.
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                held_two.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue"}}}),
            ],
            "no blockedBy connection",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                held_two.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue",
                    "blockedBy":{"nodes":"no","pageInfo":{"hasNextPage":false}}}}}),
            ],
            "nodes is not an array",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                held_two.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"blockingIssue":{"id":"I_2"}}}}),
            ],
            "dependency update returned no issue",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                held_two.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"issue":{"id":"I_1"}}}}),
            ],
            "returned no blocking issue",
        ),
        (
            vec![
                held_one.clone(),
                board_fields.clone(),
                held_two.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_9"}}}}),
            ],
            "returned the wrong issues",
        ),
    ];
    for (bodies, expected) in cases {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .write_task(&ItemWrite {
                    target: Some(NativeId("I_1".to_owned())),
                    item: Task {
                        repositories: vec![
                            Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                        ],
                        ..task("T", "one", status(StatusCategory::Todo, "Todo"))
                    },
                    depends_on: vec![edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task))],
                })
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }
}

#[tokio::test]
async fn a_blocked_by_connection_answered_in_pages_is_walked_before_it_is_reconciled() {
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    // An item whose read carried no `blockedBy` — the held answer below is the bare issue —
    // has the relationship read on its own, every page of it, before anything is reconciled;
    // and the item's content is written last, once the relationship has landed.
    let endpoint = sequence_server(vec![
        held_issue("I_1", "one", None),
        fields_json("PVT_board", usable_fields()),
        ok_field,
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_a"}],"pageInfo":{"hasNextPage":true,"endCursor":"c1"}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_b"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_a"}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_b"}}}}),
        json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}}),
    ]);
    configured(&endpoint, json!({}))
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: Task {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..task("T", "one", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![],
        })
        .await
        .expect("both pages of held blockers are taken back out");
}

#[test]
fn the_plugin_names_the_kind_the_registry_knows_it_by() {
    assert_eq!(Plugin.kind(), onetaskgraph_github_projects::KIND);
}

#[tokio::test]
async fn a_terminal_write_selects_the_mapped_column_so_a_copy_settles() {
    // The close reason carries the category and the mapped option carries the name. The
    // caller's display name therefore cannot leave the issue in some other column and
    // make a copy report a change forever.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::Done, "Shipped"),
        )))
        .await
        .unwrap();
    assert_eq!(fixture.item(&id.0).state, "CLOSED");
    assert_eq!(fixture.item(&id.0).status.as_deref(), Some("Done"));
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().status,
        status(StatusCategory::Done, "Done")
    );
}

#[tokio::test]
async fn a_sub_issue_count_this_source_cannot_read_is_refused_rather_than_read_as_none() {
    // Reading an absent or non-integer `subIssuesSummary.total` as zero would classify a
    // project as a task — quietly, and in exactly the case the kind marker exists for.
    let mut without = plain_issue();
    without.as_object_mut().unwrap().remove("subIssuesSummary");
    let mut malformed = plain_issue();
    malformed["subIssuesSummary"] = json!({"total":"many"});
    for content in [without, malformed] {
        let body = board_json(usable_fields(), complete(json!([issue_item(content)])));
        let message = refusal(
            configured(&raw_server("200 OK", &body.to_string()), json!({}))
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .expect_err("a sub-issue count this source cannot read"),
        );
        assert!(message.contains("subIssuesSummary"), "{message}");
    }

    for far in [
        json!({"id":"I_far","title":"Far work","body":null,"parent":null}),
        json!({"id":"I_far","title":"Far work","body":null,"parent":null,"subIssuesSummary":{"total":-1}}),
    ] {
        let node = json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[far],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let message = refusal(
            configured(&raw_server("200 OK", &node.to_string()), json!({}))
                .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
                .await
                .expect_err("a far end whose sub-issue count this source cannot read"),
        );
        assert!(message.contains("subIssuesSummary"), "{message}");
    }
}

#[tokio::test]
async fn a_drafts_dependency_on_an_issue_is_recorded_rather_than_lost() {
    // A draft has neither `blockedBy` nor `blocking`, so an edge of one classified as
    // native would be written nowhere: a draft's native reconciliation never runs.
    let fixture = board(vec![
        Item::draft("D_1", "a draft").status("Todo"),
        Item::issue("I_2", "an issue").status("Todo"),
    ]);
    let source = source(&fixture);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("D_1".to_owned())),
            item: task("D", "a draft", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![edge(("D_1", ItemKind::Task), ("I_2", ItemKind::Task))],
        })
        .await
        .unwrap();
    assert_eq!(
        walk(
            source.as_ref(),
            "D_1",
            ItemKind::Task,
            Direction::DependsOn,
            10
        )
        .await
        .unwrap()
        .iter()
        .map(|edge| edge.to.id().to_owned())
        .collect::<Vec<_>>(),
        ["I_2"]
    );
}

#[tokio::test]
async fn an_origin_that_is_not_a_qualified_id_is_refused_before_anything_is_created() {
    // The board's origin field is text, so a value of another JSON type has nowhere to
    // go; storing it as no origin at all would leave the copy unable to find this item
    // again. The refusal comes before `createIssue`, like every other one this write owes.
    let fixture = board_with(vec![], true, true);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([(
        "onetaskgraph.origin".to_owned(),
        json!({"source":"notes","id":"T-1"}),
    )]);
    let said = refusal(
        source
            .write_task(&write(item))
            .await
            .expect_err("an origin of the wrong JSON type"),
    );
    assert!(
        said.contains("onetaskgraph.origin") && said.contains("qualified id"),
        "the key and what it holds are named: {said}"
    );
    assert!(
        fixture.seen().is_empty(),
        "nothing was written before the refusal: {:?}",
        fixture.seen()
    );
}

#[tokio::test]
async fn a_board_that_cannot_carry_the_origin_refuses_before_it_creates_anything() {
    // Refusing after `createIssue` would leave an issue behind that nothing asked for.
    let fixture = board_with(vec![], true, false);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    source
        .write_task(&write(item))
        .await
        .expect_err("a board with no origin field");
    assert!(
        fixture.seen().is_empty(),
        "nothing was written before the refusal: {:?}",
        fixture.seen()
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "and no issue was left on the board"
    );
}

#[tokio::test]
async fn a_write_that_fails_part_way_takes_back_only_the_item_it_created() {
    // Everything this source can refuse before the first mutation is refused there, so what
    // is left is GitHub failing part way — and the two halves of that are different. An item
    // this call created is taken back, because a retry would otherwise create a second. An
    // item that was already there is not: taking it back would destroy the very state the
    // engine's copy journal exists to write back, and nothing a user typed asked for a
    // delete.
    let created = board(vec![]);
    let maker = source(&created);
    created.refuse("updateProjectV2ItemFieldValue");
    let message = refusal(
        maker
            .write_task(&write(task(
                "T-1",
                "Publish",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("the board refused the field update"),
    );
    assert!(
        message.contains("updateProjectV2ItemFieldValue"),
        "the write's own failure is what the caller is told: {message}"
    );
    assert!(
        created.seen().iter().any(|call| call[0] == "deleteIssue"),
        "the issue this call created was left behind: {:?}",
        created.seen()
    );
    assert!(
        maker
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty(),
        "and the board holds nothing this failed write made"
    );

    let held = board(vec![Item::issue("I_1", "one").body("first").status("Todo")]);
    let holder = source(&held);
    held.refuse("updateProjectV2ItemFieldValue");
    let mut revised = task(
        "T-1",
        "one, revised",
        status(StatusCategory::InProgress, "In Progress"),
    );
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let message = refusal(
        holder
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: revised,
                depends_on: vec![],
            })
            .await
            .expect_err("the board refused the field update"),
    );
    assert!(
        message.contains("updateProjectV2ItemFieldValue"),
        "the write's own failure is what the caller is told: {message}"
    );
    assert!(
        held.holds("I_1"),
        "a write took back an item it did not create: {:?}",
        held.seen()
    );
    assert!(
        held.seen().iter().all(|call| call[0] != "deleteIssue"),
        "a write that did not create the item asked for it to be deleted: {:?}",
        held.seen()
    );
    assert_eq!(
        held.item("I_1").title,
        "one",
        "an existing item's content is written last, so a board field refused before it \
         leaves the content as it stood"
    );
}

/// A server which answers every request the same way, with headers of its own.
///
/// `raw_server_with_headers` cannot spell a status *and* a body a limiter needs together
/// with more than one header line, and reading a body under a non-success status is the
/// whole point here.
fn always(refusal: &Refusal) -> String {
    raw_server_with_headers(refusal.status, &refusal.body, &refusal.headers)
}

/// A source built against `endpoint` with pacing of the test's own choosing.
///
/// Every test that asserts on a wait or a gap uses this rather than [`source`], because
/// the shipped defaults are a minute's worth of backoff and would make each of them a
/// minute long. The two that assert on the *shipped* rate say so in their own names.
fn paced(endpoint: &str, pacing: Value) -> Box<dyn TaskSource> {
    configured(endpoint, json!({ "pacing": pacing }))
}

/// Pacing that neither spaces nor retries, so what a test sees is one request and the
/// answer to it — which is what every test asserting on a *classification* wants, rather
/// than the classification of whatever the last of several attempts got.
fn no_waiting() -> Value {
    json!({"min_mutation_interval_ms":0,"retry_budget_ms":0})
}

#[tokio::test]
async fn every_shape_a_rate_limit_arrives_in_is_classified_as_one_and_never_as_a_credential() {
    // Three shapes, because GitHub sends three and this source once read only the status:
    // a forbidden status, which it called a credential problem, and a *successful*
    // response, which it called an unexplained refusal.
    let secondary_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"type":"RATE_LIMITED",
                                "message":"You have exceeded a secondary rate limit. Please \
                                           wait a few minutes before you try again."}]})
        .to_string(),
    };
    let too_many = Refusal {
        status: "429 Too Many Requests",
        headers: "retry-after: 30\r\n".to_owned(),
        body: "{}".to_owned(),
    };
    for (what, shape, expected_hint, limiter) in [
        (
            "a too-many-requests status",
            too_many,
            Some(30),
            "primary API rate limit",
        ),
        (
            "a forbidden status naming it",
            Refusal::secondary_forbidden(),
            None,
            "secondary rate limit",
        ),
        (
            "a successful response naming it",
            secondary_in_a_success,
            None,
            "secondary rate limit",
        ),
    ] {
        let error = paced(&always(&shape), no_waiting())
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err(what);
        // The exact variant, not merely "not Auth": the kind is what a caller matches on,
        // and a rate limit reported under any other kind is a caller that cannot tell this
        // from a permission problem or a refusal it should not retry.
        let SourceError::RateLimited {
            retry_after_seconds,
            message: Some(said),
        } = &error
        else {
            panic!("{what} was not classified as a rate limit: {error:?}");
        };
        assert_eq!(
            *retry_after_seconds, expected_hint,
            "{what} did not carry the wait GitHub asked for"
        );
        assert!(
            said.contains(limiter),
            "{what} does not name which limiter refused: {said}"
        );
        assert!(
            said.contains("reading the board"),
            "{what} does not say what this source was doing: {said}"
        );
        assert!(
            !said.contains("Projects and Issues read/write"),
            "{what} still sends the operator to re-scope a token that is fine: {said}"
        );
        // And the whole of it reaches a caller that only renders the error.
        assert!(
            error.to_string().contains(said.as_str()),
            "the diagnostic is carried but not rendered: {error}"
        );
    }
}

#[tokio::test]
async fn a_credential_the_board_really_rejects_is_still_a_credential_problem() {
    // The other half of the same fix: a forbidden status carrying none of the limiter's
    // wording is a token that genuinely lacks the access, and the advice it earns is the
    // access it needs. A fix which simply stopped calling 403 a credential problem would
    // pass the test above and fail this one.
    for shape in [
        Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: json!({"message":"Resource not accessible by personal access token"}).to_string(),
        },
        Refusal {
            status: "401 Unauthorized",
            headers: String::new(),
            body: "{}".to_owned(),
        },
    ] {
        let error = paced(&always(&shape), no_waiting())
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a rejected credential");
        assert!(
            matches!(error, SourceError::Auth { .. }),
            "a rejected credential stopped being one: {error:?}"
        );
        let message = refusal(error);
        assert!(
            message.contains("Projects and Issues read/write")
                && message.contains("Pull requests read-only"),
            "the refusal no longer names the access the credential needs: {message}"
        );
    }
}

#[tokio::test]
async fn a_wait_hint_is_honoured_and_the_call_retried_rather_than_reported() {
    // The defect: a refusal carrying `retry-after` became an error with the hint attached
    // and no attempt made to honour it. A source still carrying it never sends the second
    // request, so `requests("board")` is 1 and the read fails.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![Refusal::secondary_forbidden().after(1)]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_budget_ms":10_000}),
    );
    let started = Instant::now();
    let page = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry after the hinted wait");
    let waited = started.elapsed();
    assert_eq!(
        page.items.len(),
        1,
        "the retry returned the board's own item"
    );
    assert!(
        waited >= Duration::from_secs(1),
        "the hinted second was not waited out: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        2,
        "the refused read was not retried"
    );
}

#[tokio::test]
async fn a_refusal_with_no_hint_is_retried_on_a_growing_schedule() {
    // A refusal carrying no hint had no schedule at all. The assertion that catches a
    // constant one: three waits of a flat 80 ms are 240 ms, and a doubling 80/160/320 is
    // 560 ms, so the floor below is above anything but growth.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![
        Refusal::secondary_forbidden(),
        Refusal::secondary_forbidden(),
        Refusal::secondary_forbidden(),
    ]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":80,"retry_budget_ms":10_000}),
    );
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the fourth attempt, once the board stopped refusing");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(80 + 160 + 320),
        "the schedule did not grow: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        4,
        "three refusals were not each retried"
    );
}

#[tokio::test]
async fn a_limiter_that_never_lets_up_ends_in_a_diagnostic_rather_than_an_unbounded_wait() {
    // Bounded, and what the bound reports. An operator who reads a secondary refusal as a
    // primary one polls `gh api rate_limit`, sees budget, and retries harder — which
    // extends the limit — so the diagnostic has to say which limiter, what this source was
    // doing, and that the endpoint reporting the primary budget does not report this one.
    let fixture = board(vec![]);
    fixture.refuse_every_mutation();
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":300}),
    );
    let started = Instant::now();
    let error = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect_err("a limiter that never lets up");
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(10),
        "the bounded schedule did not end: {waited:?}"
    );
    assert!(
        !matches!(error, SourceError::Auth { .. }),
        "an unrelenting limiter was reported as a credential problem: {error:?}"
    );
    let message = refusal(error);
    assert!(
        message.contains("secondary rate limit"),
        "the diagnostic does not name which limiter refused: {message}"
    );
    assert!(
        message.contains("creating an issue"),
        "the diagnostic does not say what this source was doing: {message}"
    );
    assert!(
        message.contains("gh api rate_limit") && message.contains("does not report this one"),
        "the diagnostic does not say the primary endpoint is silent about this: {message}"
    );
    assert!(
        message.contains("2 refusals"),
        "the diagnostic does not say a wait was taken at all: {message}"
    );
    assert!(
        !message.contains("Projects and Issues read/write"),
        "the diagnostic still sends the operator to re-scope a token: {message}"
    );
}

#[tokio::test]
async fn a_read_refused_past_the_budget_reports_it_rather_than_hanging() {
    // The same bound over a read, and against a limiter that answers with a *successful*
    // response — the shape that used to reach `Refused` carrying GitHub's own sentence and
    // nothing about what it meant.
    let secondary_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"message":"You have exceeded a secondary rate limit"}]})
            .to_string(),
    };
    let source = paced(
        &always(&secondary_in_a_success),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":40,"retry_budget_ms":200}),
    );
    let started = Instant::now();
    let message = refusal(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a limiter that never lets up"),
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the bounded schedule did not end"
    );
    assert!(
        message.contains("reading the board") && message.contains("secondary rate limit"),
        "{message}"
    );
}

#[tokio::test]
async fn six_related_writes_read_the_boards_fields_and_repository_once_for_the_command() {
    // Six items, so a source reading the board's fields per item written reads them six
    // times and the repository six — which is the burst this counts, not the writes
    // themselves. And none of the six lists the board: each item it names is read by id.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("the project issue");
    for step in 0..5 {
        let mut child = task(
            &format!("T-{step}"),
            &format!("step {step}"),
            status(StatusCategory::Todo, "Todo"),
        );
        child.project = Some(plan.clone());
        source.write_task(&write(child)).await.expect("a task");
    }
    assert_eq!(
        fixture.requests("boardFields"),
        1,
        "the board's fields were re-read per item written"
    );
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "a copy listed the board to write items it names by id"
    );
    // Read once, beside the board's fields, by the first issue created; never on its own.
    assert_eq!(
        fixture.requests("repository"),
        0,
        "the destination repository was re-resolved per issue created"
    );
    assert_eq!(
        fixture.documents()[0],
        onetaskgraph_github_projects::graphql::CREATION_CONTEXT
    );
    assert_eq!(
        fixture.item(&plan.0).sub_issues,
        5,
        "and the copy still filed every task under its project"
    );
}

/// One update of `I_1`, moving it to `In Progress`.
async fn move_to_in_progress(source: &dyn TaskSource) {
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: Task {
                repositories: vec![
                    Repository::try_from("github.com/acme/work".to_owned()).unwrap(),
                ],
                ..task(
                    "T",
                    "one",
                    status(StatusCategory::InProgress, "In Progress"),
                )
            },
            depends_on: vec![],
        })
        .await
        .expect("the update lands");
}

#[tokio::test]
async fn an_update_its_own_item_describes_is_written_without_reading_the_board() {
    // A copy naming one member out of many updates that one item, and reading every page of
    // the board for the board's id and fields was a read of the whole board per such write.
    // The item's own node read carries both — its board entry names the board, and each
    // field value names its field — so an update the item describes costs a read of it.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    move_to_in_progress(source(&fixture).as_ref()).await;
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "an update the item describes read the whole board"
    );
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
}

fn field_writes(fixture: &Fixture) -> Vec<(String, String)> {
    fixture
        .seen()
        .into_iter()
        .filter(|call| call[0] == "updateProjectV2ItemFieldValue")
        .map(|call| {
            (
                call[1]["projectId"].as_str().unwrap_or_default().to_owned(),
                call[1]["itemId"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn an_update_its_own_values_cannot_describe_takes_the_boards_fields_from_its_own_read() {
    // An item holding no Status value says nothing about whether the board has a Status
    // field, and one holding no origin value says nothing about the origin field — and an
    // update writes both. Its read by its own id carries the board's field definitions beside
    // its values, so neither field is guessed absent and the write refused, or guessed present
    // and written blind, and the board's fields are not read on their own. Only an item whose
    // read did not carry them — its entry for this board past the page of boards it came with
    // — has them read, the fields alone: whether the item is on this board is its own read's
    // answer, so an item GitHub's listings have not caught up with lands all the same.
    for (held, what, field_reads) in [
        (
            Item::issue("I_1", "before").unlisted(),
            "no Status value",
            0,
        ),
        (
            Item::issue("I_1", "before")
                .status("Todo")
                .holding_no_origin_value()
                .unlisted(),
            "no origin value",
            0,
        ),
        (
            Item::issue("I_1", "before")
                .also_on(&boards_ahead_of_this_one())
                .unlisted(),
            "its board's entry past the page of boards",
            1,
        ),
    ] {
        let fixture = board(vec![held]);
        move_to_in_progress(source(&fixture).as_ref()).await;
        let written = fixture.item("I_1");
        assert_eq!(written.status.as_deref(), Some("In Progress"), "{what}");
        assert_eq!(written.title, "one", "{what}");
        assert_eq!(written.origin.as_deref().unwrap_or(""), "", "{what}");
        assert_eq!(fixture.requests("boardFields"), field_reads, "{what}");
        assert_eq!(fixture.requests("board"), 0, "{what}");
        assert_eq!(
            fixture.board_item_reads(),
            Vec::<String>::new(),
            "an update of an item with {what} listed the board"
        );
    }
}

#[tokio::test]
async fn an_update_takes_its_fields_from_the_board_its_item_sits_on_and_not_a_namesake() {
    // A project number is unique within its owner only, so the read of an issue by its own id
    // can carry two boards numbered alike. The one whose fields a write uses is the one the
    // issue's own board item names by id: the other's field and option ids address nothing on
    // this board.
    let fixture = board(vec![
        Item::issue("I_1", "before")
            .status("Todo")
            .namesake_board_ahead(),
    ]);
    move_to_in_progress(source(&fixture).as_ref()).await;
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
    let sent = fixture
        .seen()
        .into_iter()
        .filter(|call| call[0] == "updateProjectV2ItemFieldValue")
        .collect::<Vec<_>>();
    assert!(!sent.is_empty(), "the update wrote no board field");
    for call in &sent {
        assert_eq!(call[1]["projectId"], "PVT_board", "{sent:#?}");
        assert!(
            !call[1].to_string().contains("elsewhere"),
            "a field write used another board's ids: {sent:#?}"
        );
    }
    // This board's entry was on the page the read carried, so its fields were not read again.
    assert_eq!(fixture.requests("boardFields"), 0);
}

#[tokio::test]
async fn an_update_whose_item_names_an_empty_board_id_takes_the_boards_id_from_its_fields() {
    // The board id an update writes its fields against comes from a third party's answer,
    // and an empty one addresses no board. Taken as given, every field write of the update
    // would go out against it; read as absent, the write takes the board's real id — and
    // the field definitions beside it — from the read of the board's fields, which lists
    // no item.
    let fixture = board(vec![
        Item::issue("I_1", "before")
            .status("Todo")
            .board_entry_names(""),
    ]);
    move_to_in_progress(source(&fixture).as_ref()).await;
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
    assert_eq!(fixture.item("I_1").title, "one");
    let writes = field_writes(&fixture);
    assert!(!writes.is_empty(), "the update wrote no board field");
    assert!(
        writes.iter().all(|(board, _)| board == "PVT_board"),
        "an update took an empty board id from its item: {writes:?}"
    );
    assert_eq!(fixture.requests("boardFields"), 1);
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "the board's id was taken from a listing of its items"
    );
}

#[tokio::test]
async fn a_child_is_filed_under_a_project_issue_no_listing_of_the_board_names_yet() {
    // The 842-item board's refusal: a project issue whose own `projectItems` names the board
    // and which `ProjectV2.items` does not list. It is this board's, so a child is filed
    // under it.
    let fixture = board(vec![
        Item::issue("I_plan", "the plan")
            .sub_issues(1)
            .status("Todo")
            .unlisted(),
    ]);
    let source = source(&fixture);
    let child = source
        .write_task(&task_under(Some("I_plan"), "a step", &[]))
        .await
        .expect("a project issue on this board takes a child");
    assert_eq!(fixture.item(&child.0).parent.as_deref(), Some("I_plan"));
    assert!(
        fixture.seen().iter().any(|call| call[0] == "addSubIssue"
            && call[1]["issueId"] == "I_plan"
            && call[1]["subIssueId"] == child.0.as_str()),
        "the child was not attached as the project's sub-issue: {:?}",
        fixture.seen()
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_child_is_refused_under_an_issue_whose_whole_membership_names_no_entry_for_this_board() {
    // The other half: an issue that really is not here, its memberships walked to
    // exhaustion — past the page that came with it — and none naming this board. Refused in
    // the words it always had, naming the id, before anything is created.
    for elsewhere in [
        Item::issue("I_elsewhere", "another plan").only_on(&[3]),
        Item::issue("I_elsewhere", "another plan").only_on(&boards_ahead_of_this_one()),
    ] {
        let fixture = board(vec![elsewhere]);
        let message = refusal(
            source(&fixture)
                .write_task(&task_under(Some("I_elsewhere"), "a step", &[]))
                .await
                .expect_err("an issue this board does not hold takes no child"),
        );
        assert!(
            message.contains("GitHub project issue I_elsewhere was not found on the board"),
            "{message}"
        );
        assert!(
            !fixture.seen().iter().any(|call| call[0] == "createIssue"),
            "an issue was created before the parent was refused: {:?}",
            fixture.seen()
        );
        assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
    }
}

#[tokio::test]
async fn an_update_of_a_destination_this_board_does_not_hold_is_refused_naming_it() {
    let fixture = board(vec![
        Item::issue("I_elsewhere", "another").only_on(&boards_ahead_of_this_one()),
    ]);
    for target in ["I_elsewhere", "I_gone"] {
        let message = refusal(
            source(&fixture)
                .write_task(&ItemWrite {
                    target: Some(id(target)),
                    item: task("T", "one", status(StatusCategory::Todo, "Todo")),
                    depends_on: vec![],
                })
                .await
                .expect_err("a destination this board does not hold"),
        );
        assert!(
            message.contains(&format!("GitHub destination item {target} was not found")),
            "{message}"
        );
    }
    assert!(fixture.seen().is_empty(), "{:?}", fixture.seen());
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_same_source_far_end_no_listing_names_yet_resolves_from_its_own_read() {
    // The near item holds no Status value, so the write takes the board's fields from their
    // own read — the path that once listed the board and looked the far end up in it.
    let fixture = board(vec![
        Item::issue("I_1", "step"),
        Item::issue("I_2", "the one waited on")
            .status("Todo")
            .unlisted(),
    ]);
    source(&fixture)
        .write_task(&ItemWrite {
            target: Some(id("I_1")),
            item: Task {
                repositories: vec![repo("acme/work")],
                ..task("T", "step", status(StatusCategory::Todo, "Todo"))
            },
            depends_on: vec![edge(("I_1", ItemKind::Task), ("I_2", ItemKind::Task))],
        })
        .await
        .expect("a far end this board holds resolves");
    assert_eq!(
        fixture.state.lock().unwrap().blocked_by.get("I_1").cloned(),
        Some(vec!["I_2".to_owned()])
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_status_set_on_an_item_no_listing_names_yet_lands() {
    // With a Status value, the item carries the field's definition; without one, the
    // board's fields are read for it. Neither lists the board.
    let fixture = board(vec![
        Item::issue("I_held", "has a column")
            .status("Todo")
            .unlisted(),
        Item::issue("I_bare", "has none").unlisted(),
    ]);
    let source = source(&fixture);
    for held in ["I_held", "I_bare"] {
        source
            .set_task_status(&id(held), StatusCategory::InProgress)
            .await
            .unwrap()
            .expect("a task of this board");
        assert_eq!(
            fixture.item(held).status.as_deref(),
            Some("In Progress"),
            "{held}"
        );
    }
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_drafts_recorded_dependencies_are_read_off_its_own_read_when_no_listing_names_it() {
    // A draft's dependency node carries no body, so its slot is read off the draft itself.
    let fixture = board(vec![
        Item::draft("D_1", "a draft")
            .status("Todo")
            .body(&slotted("", &json!({"onetaskgraph.depends_on":["I_2"]})))
            .unlisted(),
        Item::issue("I_2", "other").status("Todo"),
    ]);
    let edges = walk(
        source(&fixture).as_ref(),
        "D_1",
        ItemKind::Task,
        Direction::DependsOn,
        10,
    )
    .await
    .expect("a draft's recorded edges");
    assert_eq!(
        edges.iter().map(|edge| edge.to.id()).collect::<Vec<_>>(),
        vec!["I_2"]
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn deleting_an_item_no_listing_names_yet_deletes_it() {
    // Read as *already gone*, this is the item a copy that could not finish would leave
    // behind.
    let fixture = board(vec![Item::issue("I_made", "made by a copy").unlisted()]);
    source(&fixture)
        .delete_task(&id("I_made"))
        .await
        .expect("the item is taken back");
    assert!(!fixture.holds("I_made"), "the item is still on the board");
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_draft_is_read_by_its_own_id_and_never_found_by_listing_the_board() {
    let fixture = board(vec![
        Item::draft("D_1", "a draft").status("Todo").unlisted(),
        Item::draft("D_2", "someone else's").only_on(&[3]),
    ]);
    let source = source(&fixture);
    let draft = source
        .get_task(&id("D_1"))
        .await
        .unwrap()
        .expect("a draft of this board");
    assert_eq!(draft.title, "a draft");
    assert_eq!(draft.status, status(StatusCategory::Todo, "Todo"));
    assert_eq!(
        source
            .get_task(&id("D_2"))
            .await
            .unwrap()
            .map(|task| task.id),
        None,
        "a draft on another board was answered as this board's"
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_draft_read_or_a_fields_read_this_source_cannot_trust_is_refused_by_name() {
    // A draft is linked to one board item, so a page of them reporting more is malformed —
    // even when this board's entry is on it.
    let draft_entry = json!({"id":"PVTI_D","project":{"id":"PVT_board","number":7},
                             "fieldValues":complete(json!([]))});
    let endpoint = sequence_server(vec![
        json!({"data":{"node":{"__typename":"DraftIssue"}}}),
        json!({"data":{"node":{"__typename":"DraftIssue","id":"D_1","title":"a draft",
            "body":null,"createdAt":null,"updatedAt":null,
            "projectV2Items":{"nodes":[draft_entry.clone()],
                              "pageInfo":{"hasNextPage":true,"endCursor":"1"}}}}}),
    ]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .get_task(&id("D_1"))
            .await
            .expect_err("a draft page claiming a second board item"),
    );
    assert!(
        message.contains("D_1") && message.contains("reports more board items"),
        "{message}"
    );
    // The same for a complete page holding two, and for a second read that answers the
    // draft's id as something else.
    let second_entry = json!({"id":"PVTI_E","project":{"id":"PVT_9","number":9},
                              "fieldValues":complete(json!([]))});
    for (answer, expected) in [
        (
            json!({"data":{"node":{"__typename":"DraftIssue","id":"D_1","title":"a draft",
                "body":null,"createdAt":null,"updatedAt":null,
                "projectV2Items":{"nodes":[draft_entry.clone(), second_entry],
                                  "pageInfo":{"hasNextPage":false,"endCursor":"2"}}}}}),
            "reports more board items",
        ),
        (
            json!({"data":{"node":{"__typename":"Issue"}}}),
            "as a draft and then as something else",
        ),
        (
            json!({"data":{"node":{"__typename":"DraftIssue","id":"D_other","title":"a draft",
                "body":null,"createdAt":null,"updatedAt":null,
                "projectV2Items":{"nodes":[draft_entry.clone()],
                                  "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
            "answered a different draft",
        ),
    ] {
        let endpoint = sequence_server(vec![
            json!({"data":{"node":{"__typename":"DraftIssue"}}}),
            answer,
        ]);
        let message = refusal(
            configured(&endpoint, json!({}))
                .get_task(&id("D_1"))
                .await
                .expect_err(expected),
        );
        assert!(
            message.contains("D_1") && message.contains(expected),
            "{message}"
        );
    }

    // A blank board id addresses no board, so it is refused before anything is created.
    let endpoint = sequence_server(vec![fields_json(" ", usable_fields())]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
            .await
            .expect_err("a board read naming a blank id"),
    );
    assert!(message.contains("blank node id"), "{message}");
}

#[tokio::test]
async fn a_vanished_draft_is_absent_and_a_malformed_membership_is_refused() {
    let reached = json!({"data":{"node":{"__typename":"DraftIssue"}}});
    let vanished = configured(
        &sequence_server(vec![reached.clone(), json!({"data":{"node":null}})]),
        json!({}),
    );
    assert!(vanished.get_task(&id("D_1")).await.unwrap().is_none());

    for (membership, expected) in [
        (None, "missing projectV2Items"),
        (Some(json!({"nodes":"bad"})), "nodes is not an array"),
        (Some(json!({"nodes":[]})), "has no pageInfo"),
    ] {
        let mut draft = json!({"__typename":"DraftIssue","id":"D_1"});
        if let Some(membership) = membership {
            draft["projectV2Items"] = membership;
        }
        let endpoint = sequence_server(vec![reached.clone(), json!({"data":{"node":draft}})]);
        let message = refusal(
            configured(&endpoint, json!({}))
                .get_task(&id("D_1"))
                .await
                .unwrap_err(),
        );
        assert!(message.contains(expected), "{message}");
    }
}

#[tokio::test]
async fn a_draft_on_a_different_board_with_the_same_number_is_not_returned() {
    let endpoint = sequence_server(vec![
        json!({"data":{"node":{"__typename":"DraftIssue"}}}),
        json!({"data":{"node":{"__typename":"DraftIssue","id":"D_1","title":"elsewhere",
            "body":null,"createdAt":null,"updatedAt":null,
            "projectV2Items":{"nodes":[{"id":"PVTI_1",
                "project":{"id":"PVT_elsewhere","number":7},
                "fieldValues":complete(json!([]))}],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        fields_json("PVT_board", usable_fields()),
    ]);
    assert!(
        configured(&endpoint, json!({}))
            .get_task(&id("D_1"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_draft_membership_without_a_numeric_board_number_is_refused() {
    let endpoint = sequence_server(vec![
        json!({"data":{"node":{"__typename":"DraftIssue"}}}),
        json!({"data":{"node":{"__typename":"DraftIssue","id":"D_1",
            "projectV2Items":{"nodes":[{"id":"PVTI_1",
                "project":{"id":"PVT_board","number":"seven"}}],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
    ]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .get_task(&id("D_1"))
            .await
            .expect_err("a malformed membership"),
    );
    assert!(
        message.contains("D_1") && message.contains("numeric project number"),
        "{message}"
    );
}

#[tokio::test]
async fn a_fields_only_read_of_an_unavailable_board_refuses_before_creation() {
    let endpoint = sequence_server(vec![json!({"data":{"boardFields":{"projectV2":null}}})]);
    let message = refusal(
        configured(&endpoint, json!({}))
            .write_task(&write(task("T", "x", status(StatusCategory::Todo, "Todo"))))
            .await
            .expect_err("an unavailable board"),
    );
    assert!(
        message.contains("project") && message.contains("not found"),
        "{message}"
    );
}

#[tokio::test]
async fn a_listing_this_command_already_holds_does_not_decide_whether_an_item_is_on_the_board() {
    // The listing may still supply the board's fields; it may not refuse an item it omits.
    let fixture = board(vec![
        Item::issue("I_listed", "listed").status("Todo"),
        Item::issue("I_1", "before").status("Todo").unlisted(),
    ]);
    let source = source(&fixture);
    let listed = selected_tasks(source.as_ref(), &TaskQuery::default()).await;
    assert!(
        listed.contains(&"I_listed".to_owned()) && !listed.contains(&"I_1".to_owned()),
        "the listing was meant to omit I_1: {listed:?}"
    );
    let listings = fixture.board_item_reads().len();
    assert!(listings > 0, "the command did not list the board first");
    move_to_in_progress(source.as_ref()).await;
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
    assert_eq!(fixture.item("I_1").title, "one");
    assert_eq!(
        fixture.board_item_reads().len(),
        listings,
        "the update listed the board again"
    );
}

#[tokio::test]
async fn the_board_this_command_reads_holds_what_this_command_has_itself_written() {
    // The half a cache gets wrong. GitHub's project items are eventually consistent, so a
    // board read is completed from what this run created; a cache that served the snapshot
    // taken *before* those writes would break exactly what that completion exists to fix.
    //
    // Both halves are asserted: an item created after the cached read is depended on by a
    // later one and resolves, and an item that was already on the board reads back through
    // the source with what the second write gave it rather than what it had before.
    let fixture = board(vec![Item::issue("I_old", "already there").status("Todo")]);
    let source = source(&fixture);
    // The command lists the board first, so every write below lands on a listing it
    // already holds — which is the listing whose staleness this is about.
    assert_eq!(
        selected_tasks(source.as_ref(), &TaskQuery::default()).await,
        vec!["I_old".to_owned()]
    );
    let first = source
        .write_task(&write(task(
            "T-1",
            "the one depended on",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("the first task");
    let second = task(
        "T-2",
        "the one that depends",
        status(StatusCategory::Todo, "Todo"),
    );
    let landed = source
        .write_task(&ItemWrite {
            target: None,
            item: second,
            depends_on: vec![edge(("T-2", ItemKind::Task), (&first.0, ItemKind::Task))],
        })
        .await
        .expect("an item created earlier in this command resolves as a far end");
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "addBlockedBy" && call[1]["blockingIssueId"] == first.0.as_str()),
        "the dependency on the item this command created was not recorded: {:?}",
        fixture.seen()
    );

    let mut revised = task("T", "renamed", status(StatusCategory::Todo, "Todo"));
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_old".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("a second write of an item that was already on the board");
    assert_eq!(
        source
            .get_task(&NativeId("I_old".to_owned()))
            .await
            .unwrap()
            .expect("the item is still there")
            .title,
        "renamed",
        "this command's own view of the board went stale after it wrote to it"
    );
    let listed = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists again")
        .items;
    assert!(
        listed
            .iter()
            .any(|task| task.id.0 == "I_old" && task.title == "renamed")
            && listed.iter().any(|task| task.id == first)
            && listed.iter().any(|task| task.id == landed),
        "the listing this command holds went stale after it wrote: {:?}",
        listed
            .iter()
            .map(|task| (&task.id.0, &task.title))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "and it took one board read to answer all of that"
    );
    assert!(landed.0.starts_with("I_new"));
}

/// The title one source reports for a board item, read through the trait a caller holds.
///
/// A whole-board read rather than a read by id, and the difference is the subject of the
/// test below: an unconstrained task list is the question the board's own item connection
/// answers, and a read by id is not.
async fn title_of(source: &dyn TaskSource, id: &str) -> String {
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board answers a task query")
        .items
        .into_iter()
        .find(|task| task.id.0 == id)
        .expect("the board holds the item")
        .title
}

#[tokio::test]
async fn a_board_changed_by_something_else_is_seen_by_the_next_source_and_not_by_this_one() {
    // The other face of reading the board once, and the one no other test here states. A
    // source answers from that read for as long as it lives, so a change *nothing it did*
    // made is not visible to it. For every consumer this repository ships that is exactly
    // right — one invocation of the binary is one process, one source and one read, and
    // both SDKs drive that binary as a subprocess — and it is what stops a copy of a
    // project re-reading the whole board per item it writes.
    //
    // It is pinned here rather than left to prose because it is the observable a change to
    // the cache's scope moves, and prose does not fail. `docs/follow-ups.md` records what
    // a caller that links the crate and holds a source across several commands is owed,
    // and says that settling it means saying what this test should assert instead.
    let fixture = board(vec![Item::issue("I_1", "as it was").status("Todo")]);
    let held = source(&fixture);
    assert_eq!(title_of(held.as_ref(), "I_1").await, "as it was");

    fixture.retitled_by_something_else("I_1", "as somebody else left it");
    assert_eq!(
        title_of(held.as_ref(), "I_1").await,
        "as it was",
        "this source read the board a second time, which is the request its one read buys"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "two reads through one source cost two reads of the board"
    );

    // And a source built the way the next command builds one reads what is there now, so
    // the change is invisible for the life of one command rather than lost.
    let next = source(&fixture);
    assert_eq!(
        title_of(next.as_ref(), "I_1").await,
        "as somebody else left it",
        "a fresh source answered from a board read some earlier source had made"
    );
    assert_eq!(fixture.requests("board"), 2);

    // And the half of this that is no longer true, pinned on the same board so the two
    // cannot be confused. A read by id resolves that id — one request against the issue
    // itself rather than a walk of the board — so it is answered by what GitHub holds now,
    // by the same source, with no second board read bought.
    let before = fixture.requests("board");
    assert_eq!(
        held.get_task(&NativeId("I_1".to_owned()))
            .await
            .expect("the board answers a task read")
            .expect("the board holds the item")
            .title,
        "as somebody else left it",
        "a read by id resolves the id rather than answering from the board this source read"
    );
    assert_eq!(
        fixture.requests("board"),
        before,
        "and it did that without reading the board again"
    );
    assert_eq!(fixture.requests("issue"), 1);
}

/// The wait a live run reads its own fixture through, driven against a board that is behind.
///
/// `journey::settled_fixture` is the real one — the code the credentialed lane runs — and
/// what it is driven against is this file's own loopback board. No credential, no third
/// party, nothing outside this process.
///
/// The board is held one item behind what it really holds, which is what GitHub's
/// eventually-consistent item connection and its issue search both do to a run reading the
/// fixture it has just written. **It then catches up on its own, when it has answered the
/// first attempt's second read, rather than when the wait does anything** — which is what
/// makes this evidence about the wait rather than about the fixture. By the time the second
/// attempt runs the item is there and readable, so a wait that kept its first source would
/// be reading a board read taken while that item was still hidden, and would never see it
/// however long it went on. That is exactly what
/// `a_board_changed_by_something_else_is_seen_by_the_next_source_and_not_by_this_one` pins
/// about a source, and it is what failed the credentialed lane with "the board never
/// reported all three tasks".
#[tokio::test]
async fn the_fixture_wait_reads_through_a_source_built_after_the_board_caught_up() {
    // One run's five artifacts, titled the way a run titles them: two projects — an issue
    // with sub-issues and no parent — and three tasks. The one this board holds back is
    // last, because `read_behind` holds back what a board took most recently, and it is a
    // task on purpose: a task list is answered from the read a source keeps, so the
    // half held back is the half a kept source could never recover.
    let prefix = "onetaskgraph live cleanup 4242-909-";
    let titles = (1..=5)
        .map(|number| format!("{prefix}{number}"))
        .collect::<Vec<_>>();
    let fixture = board(vec![
        Item::issue("P_alpha", &titles[0])
            .status("Todo")
            .sub_issues(1),
        Item::issue("P_beta", &titles[1])
            .status("Todo")
            .sub_issues(1),
        Item::issue("T_first", &titles[2])
            .status("Todo")
            .parent("P_alpha"),
        Item::issue("T_second", &titles[3])
            .status("Todo")
            .parent("P_beta"),
        Item::issue("T_orphan", &titles[4]).status("Todo"),
    ]);
    fixture.read_behind(1);

    // The catch-up, from a thread of its own so that when it happens is this board's
    // business and not the wait's. One attempt is two searches — the task listing narrowed to
    // the run's prefix, then the board-scoped search that lists projects — and this board
    // records a search as it answers it, so two recorded searches mean the first attempt has
    // been answered whole, behind.
    let catching_up = Arc::clone(&fixture.state);
    let caught_up = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            {
                let mut state = catching_up.lock().unwrap();
                if state.searches.len() >= 2 {
                    state.lagging_reads = 0;
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    });

    let built = std::cell::Cell::new(0_usize);
    let rebuild = || {
        built.set(built.get() + 1);
        source(&fixture)
    };
    let settled = journey::settled_fixture(&rebuild, prefix, 3, 2)
        .await
        .expect("the wait settles once the board has caught up");

    assert!(
        caught_up.join().expect("the catch-up thread"),
        "this board never caught up, so nothing below would be about the wait"
    );
    assert_eq!(
        built.get(),
        2,
        "the wait answered from one source, so what it read was a board read taken while \
         the fixture was still behind"
    );
    assert_eq!(
        settled.tasks,
        vec![titles[2].clone(), titles[3].clone(), titles[4].clone()],
        "the task the board was holding back was not reported once it caught up"
    );
    assert_eq!(settled.projects, vec![titles[0].clone(), titles[1].clone()]);
    // Two attempts, each two searches and no board read: the task listing narrowed to the
    // run's prefix and the board-scoped search that lists projects. The second pair is the
    // request the rebuild buys, and it is the whole cost of being able to wait at all.
    assert_eq!(fixture.requests("board"), 0);
    assert_eq!(fixture.requests("search"), 4);
}

#[test]
fn the_shipped_pacing_defaults_are_githubs_published_limits() {
    // What GitHub publishes is `CONTENT_CREATION_PER_MINUTE`, and that is pinned and
    // gated against `fixtures/rate-limits.json` by the crate's drift check rather than
    // here. What this pins is the millisecond value that pacing actually runs at, so a
    // derivation that started rounding the wrong way would be caught on this side too.
    assert_eq!(onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS, 750);
    // The other two defaults, because the constant is where each one's reasoning is
    // written down and a value that drifts from it makes that reasoning a lie.
    assert_eq!(onetaskgraph_github_projects::RETRY_BACKOFF_MS, 1_000);
    let mut wait = onetaskgraph_github_projects::RETRY_BACKOFF_MS;
    let mut doublings = 0_u32;
    while wait < 60_000 {
        wait *= 2;
        doublings += 1;
    }
    assert_eq!(
        doublings, 6,
        "the shipped backoff no longer reaches a minute in six waits, which is what its \
         own reasoning claims for it"
    );
    assert_eq!(onetaskgraph_github_projects::RETRY_BUDGET_MS, 120_000);
    const {
        assert!(
            onetaskgraph_github_projects::RETRY_BUDGET_MS > 0
                && onetaskgraph_github_projects::RETRY_BUDGET_MS
                    <= onetaskgraph_github_projects::MAX_PACING_MS,
            "a shipped budget of zero would never wait and one past the cap could not be \
             configured, and the bound is what makes the wait a wait rather than a hang"
        );
    }
}

#[tokio::test]
async fn the_shipped_backoff_is_what_an_unhinted_refusal_waits_when_nothing_is_configured() {
    // Every other limiter test states its own backoff, so the path a board on github.com
    // actually takes — the one where the configuration says nothing and `Pacing::resolve`
    // falls back to `RETRY_BACKOFF_MS` — was the one path never driven against a refusal.
    // The window below is read from the constant on purpose: what this proves is the
    // *wiring*, that the defaulted path waits the shipped backoff and not some other
    // number, and the test above is what pins the number itself.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![Refusal::secondary_forbidden()]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    let shipped = Duration::from_millis(onetaskgraph_github_projects::RETRY_BACKOFF_MS);
    let started = Instant::now();
    let page = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry the shipped backoff schedules");
    let waited = started.elapsed();
    assert_eq!(
        page.items.len(),
        1,
        "the retry returned the board's own item"
    );
    assert_eq!(
        fixture.requests("board"),
        2,
        "the unhinted refusal was not retried at the shipped default"
    );
    assert!(
        waited >= shipped && waited < shipped * 2,
        "the defaulted path did not wait the shipped backoff of {shipped:?}: {waited:?}"
    );
}

#[tokio::test]
async fn the_shipped_budget_bounds_a_wait_no_configuration_asked_for() {
    // The other half of the defaulted path: a hint this source cannot afford. GitHub is
    // entitled to ask for longer than one call may spend waiting, and what bounds that at
    // the shipped defaults is `RETRY_BUDGET_MS` alone. A source defaulting to an unbounded
    // budget honours the hint instead and sits here for two minutes; so does one whose
    // default budget is longer than the hint. Neither reaches the assertions below.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    let past_the_budget = onetaskgraph_github_projects::RETRY_BUDGET_MS / 1_000 + 1;
    fixture.script(vec![Refusal::secondary_forbidden().after(past_the_budget)]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    let started = Instant::now();
    let error = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a hint past the shipped budget");
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(5),
        "the shipped budget did not bound a hint of {past_the_budget}s: {waited:?}"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "a wait it could not afford was taken anyway"
    );
    let message = refusal(error);
    assert!(
        message.contains(&format!(
            "{:.1}s one call may spend waiting",
            onetaskgraph_github_projects::RETRY_BUDGET_MS as f64 / 1_000.0
        )),
        "the diagnostic does not name the shipped budget that bounded it: {message}"
    );
    assert!(
        message.contains("secondary rate limit") && message.contains("reading the board"),
        "the bounded refusal stopped naming which limiter and what this source was doing: \
         {message}"
    );
}

#[tokio::test]
async fn content_creating_mutations_leave_this_source_no_faster_than_the_shipped_rate() {
    // Driven at the *shipped* default rather than a configured one, because the shipped
    // default is what a board on github.com meets. A source with no pacing at all sends
    // these three mutations inside a millisecond of each other, so every gap below fails.
    let fixture = board(vec![]);
    let source = configured(&fixture.endpoint, json!({"pacing": null}));
    source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("one task");
    let gaps = fixture.mutation_gaps();
    // `createIssue`, `addProjectV2ItemById` and its board fields.
    assert!(
        gaps.len() == 2,
        "a created task is three mutations, and this saw {}",
        gaps.len() + 1
    );
    let floor = Duration::from_millis(onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS);
    // Transit no longer eats into the gap a board sees — the interval is counted from the
    // last mutation's completion, which is after its arrival, so an arrival gap is at least
    // the interval by construction and
    // `the_interval_a_board_sees_is_the_full_one_however_long_a_request_is_in_transit`
    // asserts exactly that. What is left to allow for is the timer: a sleep is scheduled in
    // whole milliseconds and may fire a shade under its deadline, so the floor keeps a
    // millisecond-scale tolerance, which is still two orders of magnitude above the arrival
    // gap of a source that paces nothing.
    let tolerance = Duration::from_millis(10);
    assert!(
        gaps.iter().all(|gap| *gap >= floor - tolerance),
        "a mutation left this source faster than the shipped rate: {gaps:?}"
    );
    assert!(
        gaps.iter().sum::<Duration>() >= floor * u32::try_from(gaps.len()).unwrap() - tolerance,
        "the mutations of one write did not cost the shipped rate between them: {gaps:?}"
    );
    // And reads are not paced: pacing what the secondary limiter does not count would
    // charge every listing for a limit it cannot trip.
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a read");
    assert!(
        started.elapsed() < floor,
        "a read was paced as though it created content"
    );
}

#[tokio::test]
async fn the_interval_a_board_sees_is_the_full_one_however_long_a_request_is_in_transit() {
    // The board counts arrivals; the source can only choose departures. A source spacing
    // one departure from the last hands the board a gap of the interval *less* whatever the
    // previous request spent in transit — so it can pace correctly and still be seen going
    // too fast, which is how a copy paced at 60 ms against a board allowing 45 ms passed on
    // a quick machine and was refused on a slower one.
    //
    // This board holds each mutation's answer for 120 ms, which stands in for that transit
    // and is far longer than the 60 ms interval so the two behaviours cannot be confused. A
    // source spacing from the release moment finds every slot already in the past and sends
    // the moment the previous answer lands: arrival gaps of about 120 ms, the transit alone
    // and none of the interval. Spacing from completion adds the interval on top, so the
    // board sees at least 180 ms and would see at least the interval however slow transit
    // got.
    let transit = Duration::from_millis(120);
    let interval = Duration::from_millis(60);
    let fixture = board(vec![]);
    fixture.delay_mutation_responses(transit);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":60,"retry_budget_ms":0}),
    );
    source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("one task");
    let gaps = fixture.mutation_gaps();
    // `createIssue`, `addProjectV2ItemById` and its board fields.
    assert!(
        gaps.len() == 2,
        "a created task is three mutations, and this saw {}",
        gaps.len() + 1
    );
    // No tolerance is subtracted here and none is needed: the ordering that makes this
    // hold is causal rather than clocked. The previous request had arrived before its
    // answer was held, the answer was held for `transit`, and the next mutation waited
    // `interval` after receiving it, so `transit + interval` has elapsed on this board's
    // own clock between the two arrivals it recorded.
    assert!(
        gaps.iter().all(|gap| *gap >= transit + interval),
        "a mutation arrived without the full interval after the last one finished, so the \
         interval was measured from the release moment and transit was subtracted from it: \
         {gaps:?}"
    );
}

#[tokio::test]
async fn a_copy_of_a_project_of_many_tasks_is_not_refused_by_a_board_enforcing_that_rate() {
    // The whole point, end to end: a board which refuses any mutation arriving too soon
    // after the one before it, and a copy of a project holding many tasks which is never
    // refused by it. `retry_budget_ms: 0` is deliberate — nothing here may be rescued by a
    // retry, so what completes the copy is the pacing and only the pacing.
    //
    // The board's threshold sits a little under the source's own interval, and what makes
    // that safe is causal rather than statistical: the source counts its interval from the
    // moment the last mutation finished, which is after that mutation arrived here, so an
    // arrival gap is at least the full 60 ms however long a request spends in transit. It
    // once sat on the far weaker footing that loopback jitter would stay inside the margin,
    // and a Windows runner — where one round trip costs more than the margin — refused this
    // copy on its fifth task. An unpaced source arrives at roughly zero spacing and is
    // refused on its second mutation.
    let fixture = board(vec![]);
    fixture.rate_limit_mutations(Duration::from_millis(45));
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":60,"retry_budget_ms":0}),
    );
    let plan = source
        .write_project(&write(project(
            "P-1",
            "Published roadmap",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .expect("the project issue was not refused");
    for step in 0..8 {
        let mut child = task(
            &format!("T-{step}"),
            &format!("step {step}"),
            status(StatusCategory::Todo, "Todo"),
        );
        child.project = Some(plan.clone());
        source
            .write_task(&write(child))
            .await
            .unwrap_or_else(|error| panic!("task {step} was refused: {error}"));
    }
    assert_eq!(
        fixture.too_fast(),
        0,
        "the board refused mutations for arriving too fast"
    );
    assert_eq!(fixture.item(&plan.0).sub_issues, 8);
    assert!(
        fixture.mutation_gaps().len() >= 30,
        "a project of eight tasks is far more than a handful of mutations"
    );
}

#[test]
fn a_pacing_setting_that_would_not_pace_is_refused_when_the_source_is_built() {
    // A zero backoff with a budget to spend is a schedule of zero-length waits: it
    // consumes none of the budget, so the loop that ends when the budget runs out never
    // ends. And every setting is bounded, because a wait budget past that bound is the
    // unbounded wait this mechanism exists to replace.
    let refused = build_refusal(json!({"owner":"octo-org","project_number":7,
        "pacing":{"retry_backoff_ms":0,"retry_budget_ms":5000}}));
    assert!(
        refused.contains("retry_backoff_ms")
            && refused.contains("retry_budget_ms")
            && refused.contains("forever"),
        "{refused}"
    );
    for field in [
        "min_mutation_interval_ms",
        "retry_backoff_ms",
        "retry_budget_ms",
    ] {
        let refused = build_refusal(json!({"owner":"octo-org","project_number":7,
            "pacing":{field: onetaskgraph_github_projects::MAX_PACING_MS + 1}}));
        assert!(
            refused.contains(field) && refused.contains("an hour"),
            "{field}: {refused}"
        );
    }
    // And a zero backoff beside a zero budget is not a schedule at all — it reports the
    // first refusal, which is what every fixture-driven test here asks for.
    assert!(
        Plugin
            .build(
                &SourceName::new("work").unwrap(),
                &json!({"owner":"octo-org","project_number":7,
                        "pacing":{"retry_backoff_ms":0,"retry_budget_ms":0}}),
                &Secrets,
            )
            .is_ok()
    );
}

#[tokio::test]
async fn the_primary_budget_is_waited_out_and_then_reported_as_the_rate_limit_it_is() {
    // The other limiter. Both report as `SourceError::RateLimited`, because that is what
    // happened; what tells them apart is the message, and the primary one sends the
    // operator to the endpoint that really does report it. It reaches this source in two
    // shapes of its own: the `x-ratelimit-remaining: 0` header, and a successful response
    // whose GraphQL errors name it.
    let exhausted = Refusal {
        status: "403 Forbidden",
        headers: "x-ratelimit-remaining: 0\r\n".to_owned(),
        body: "{}".to_owned(),
    };
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.script(vec![exhausted.clone()]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":60,"retry_budget_ms":5_000}),
    );
    let started = Instant::now();
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the retry once the budget was no longer reported spent");
    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "no wait taken"
    );
    assert_eq!(fixture.requests("board"), 2, "the read was not retried");

    // Unrelenting, it reports as a rate limit carrying the wait GitHub asked for.
    let error = paced(
        &always(&Refusal {
            status: "429 Too Many Requests",
            headers: "retry-after: 30\r\n".to_owned(),
            body: "{}".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("an exhausted primary budget");
    let SourceError::RateLimited {
        retry_after_seconds: Some(30),
        message: Some(said),
    } = &error
    else {
        panic!("the primary budget reported as {error:?}");
    };
    assert!(
        said.contains("primary API rate limit") && said.contains("`gh api rate_limit` reports"),
        "the primary limit does not send the operator to the endpoint that reports it: {said}"
    );

    // And in the shape that arrives as a successful response.
    let named_in_a_success = Refusal {
        status: "200 OK",
        headers: String::new(),
        body: json!({"errors":[{"type":"RATE_LIMITED",
                                "message":"API rate limit exceeded for user ID 1."}]})
        .to_string(),
    };
    let error = paced(&always(&named_in_a_success), no_waiting())
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a primary rate limit named in a successful response");
    let SourceError::RateLimited {
        retry_after_seconds: None,
        message: Some(said),
    } = &error
    else {
        panic!("a primary rate limit inside a successful response reported as {error:?}");
    };
    assert!(said.contains("primary API rate limit"), "{said}");
}

#[tokio::test]
async fn a_budget_already_spent_is_a_rate_limit_carrying_the_reset_rather_than_a_refusal() {
    // What GraphQL answers a request made after the hour's budget is spent: an HTTP 200 whose
    // one error says "API rate limit already exceeded for user ID …" and carries no `type`.
    // Neither older wording is a substring of it, so it was answered as `Refused` — a limit
    // that lifts on its own, reported as one that never will.
    let reset = chrono::Utc::now().timestamp() + 120;
    let spent = Refusal {
        status: "200 OK",
        headers: format!("x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {reset}\r\n"),
        body:
            json!({"errors":[{"message":"API rate limit already exceeded for user ID 19440155."}]})
                .to_string(),
    };
    let error = paced(&always(&spent), no_waiting())
        .get_task(&NativeId("I_1".into()))
        .await
        .expect_err("a spent budget");
    let SourceError::RateLimited {
        retry_after_seconds: Some(wait),
        message: Some(said),
    } = &error
    else {
        panic!("a spent budget was reported as {error:?}");
    };
    assert!(
        (100..=120).contains(wait),
        "the wait is not the one GitHub's reset states: {wait}"
    );
    assert!(said.contains("primary API rate limit"), "{said}");

    // With no reset stated there is no wait to carry, and it is still a rate limit.
    let unstated = Refusal {
        headers: String::new(),
        ..spent
    };
    let error = paced(&always(&unstated), no_waiting())
        .get_task(&NativeId("I_1".into()))
        .await
        .expect_err("a spent budget");
    assert!(
        matches!(
            error,
            SourceError::RateLimited {
                retry_after_seconds: None,
                ..
            }
        ),
        "a spent budget with no reset was reported as {error:?}"
    );
}

#[tokio::test]
async fn a_wait_hint_of_nothing_is_still_a_wait_and_still_ends() {
    // GitHub really does answer `retry-after: 0`. Honoured literally it is a retry with no
    // wait at all, which spends none of the budget — so the schedule would never end, and
    // retrying at once is the one move that extends a secondary limit. A source honouring
    // it literally hangs here rather than failing.
    let unrelenting = Refusal::secondary_forbidden().after(0);
    let source = paced(
        &always(&unrelenting),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":200}),
    );
    let started = Instant::now();
    let message = refusal(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("a hint of nothing, from a limiter that never lets up"),
    );
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "it never ended: {took:?}");
    assert!(
        took >= Duration::from_millis(50),
        "the hint of nothing was honoured literally: {took:?}"
    );
    assert!(message.contains("secondary rate limit"), "{message}");
}

#[tokio::test]
async fn an_exhausted_budget_reports_when_it_comes_back_rather_than_burning_the_wait_on_it() {
    // `x-ratelimit-reset` is the primary budget's own hint, spelled as the moment it
    // refills rather than as a wait. A reset an hour out is past anything one command may
    // spend waiting, so this reports at once — carrying that wait — instead of sitting in
    // the schedule for a limit that will not lift inside it.
    let refills_in_an_hour = chrono::Utc::now().timestamp() + 3_600;
    let source = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: format!(
                "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {refills_in_an_hour}\r\n"
            ),
            body: "{}".to_owned(),
        }),
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let started = Instant::now();
    let error = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err("a budget that does not come back inside the wait");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it waited on a reset it could never reach"
    );
    let SourceError::RateLimited {
        retry_after_seconds: Some(seconds),
        ..
    } = error
    else {
        panic!("an exhausted primary budget reported as {error:?}");
    };
    assert!(
        (3_500..=3_600).contains(&seconds),
        "the reset was not reported as the wait it is: {seconds}"
    );
}

#[tokio::test]
async fn a_board_holding_work_about_rate_limits_is_not_read_as_a_rate_limit() {
    // A board is where people write about their own work, and this product's own board
    // holds tasks named after the very wordings a refusal carries. Matched across the raw
    // response text — which is where a forbidden status really does carry them — a
    // perfectly good answer would become a refusal this source then waited out and
    // reported. So classification reads what a response says about *itself* and never the
    // work it carries, and a source matching the whole body fails here on both counts.
    let fixture = board(vec![
        Item::issue("I_1", "You have exceeded a secondary rate limit").status("Todo"),
        Item::issue("I_2", "triage the abuse detection mechanism").body("API rate limit exceeded"),
    ]);
    let source = source(&fixture);
    let listed = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a board whose items are about rate limits is still a board");
    assert_eq!(listed.items.len(), 2);
    assert_eq!(
        fixture.requests("board"),
        1,
        "an answer was retried as though it were a refusal"
    );
    source
        .write_task(&write(task(
            "T-1",
            "You have exceeded a secondary rate limit",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("and writing one is not a refusal either");
}

#[tokio::test]
async fn a_mutation_refused_for_a_rate_limit_is_retried_and_lands() {
    // The recovery a copy actually needs: a refused mutation never ran, so replaying it is
    // safe, and the item it was creating ends up on the board rather than the copy ending
    // half done. A source that reports a refused mutation instead of retrying it leaves
    // the board empty here.
    let fixture = board(vec![]);
    fixture.script_for("createIssue", vec![Refusal::secondary_forbidden()]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":60,"retry_budget_ms":5_000}),
    );
    let landed = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("the write past a refusal it waited out");
    assert!(fixture.holds(&landed.0), "the retried write did not land");
    assert_eq!(
        fixture.item(&landed.0).title,
        "Publish",
        "and it landed with what it was given"
    );
    assert_eq!(
        fixture.requests("createIssue"),
        2,
        "the refused creation was not retried"
    );
    assert_eq!(
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "createIssue")
            .count(),
        1,
        "a refused call never ran, so exactly one creation reached the board"
    );
}

#[tokio::test]
async fn every_wording_github_refuses_a_burst_with_is_read_as_the_secondary_limiter() {
    // GitHub has renamed this limiter and reworded its refusal more than once, and the
    // older wordings still come back from some endpoints. Each is a separate arm of the
    // classification, so each is driven rather than one standing in for the rest — and a
    // wording GitHub sends that this source does not know is a credential problem again.
    for said in [
        "You have exceeded a secondary rate limit and have been temporarily blocked from \
         content creation.",
        "You have exceeded a secondary rate limit. Please wait a few minutes.",
        "You have triggered an abuse detection mechanism.",
        "You have exceeded a secondary rate limit for this endpoint.",
        "Your request was submitted too quickly. Please wait and try again.",
    ] {
        let error = paced(
            &always(&Refusal {
                status: "403 Forbidden",
                headers: String::new(),
                body: json!({ "message": said }).to_string(),
            }),
            no_waiting(),
        )
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect_err(said);
        let SourceError::RateLimited {
            message: Some(diagnostic),
            ..
        } = &error
        else {
            panic!("{said:?} was not read as a rate limit: {error:?}");
        };
        assert!(diagnostic.contains("secondary rate limit"), "{diagnostic}");
    }

    // A failing response that is not JSON at all — a proxy's own page, say. There is
    // nothing structured to read, so the text it did send is what classification has.
    let error = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: "<html><body>You have exceeded a secondary rate limit</body></html>".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("a secondary limit that did not arrive as JSON");
    assert!(
        matches!(error, SourceError::RateLimited { .. }),
        "a non-JSON refusal naming the limiter reported as {error:?}"
    );

    // And the same shape saying nothing about a limit is still the credential it is.
    let error = paced(
        &always(&Refusal {
            status: "403 Forbidden",
            headers: String::new(),
            body: "<html><body>Forbidden</body></html>".to_owned(),
        }),
        no_waiting(),
    )
    .query_tasks(&TaskQuery::default(), &page(10))
    .await
    .expect_err("a plain forbidden page");
    assert!(
        matches!(error, SourceError::Auth { .. }),
        "a forbidden response saying nothing about a limit reported as {error:?}"
    );
}

#[tokio::test]
async fn the_diagnostic_names_whichever_call_the_limiter_caught() {
    // "what the source was doing" is per operation, and a copy is many of them — the one
    // an operator needs named is whichever was refused, not whichever happens to come
    // first. Each is refused on its own, by name, against a board that answers the rest.
    let cases: Vec<(&str, &str)> = vec![
        (
            "boardFields",
            "reading the board's fields and the destination repository",
        ),
        ("createIssue", "creating an issue"),
        ("addProjectV2ItemById", "adding an issue to the board"),
        ("updateProjectV2ItemFieldValue", "writing a board field"),
        ("addSubIssue", "filing an issue under its project"),
        ("addBlockedBy", "recording a dependency"),
    ];
    for (operation, doing) in cases {
        let fixture = board(vec![Item::issue("I_far", "the far end").status("Todo")]);
        fixture.script_for(operation, vec![Refusal::secondary_forbidden()]);
        let source = paced(&fixture.endpoint, no_waiting());
        let plan = source
            .write_project(&write(project(
                "P-1",
                "Published roadmap",
                status(StatusCategory::InProgress, "In Progress"),
            )))
            .await;
        // A project write is the board's fields with the repository, `createIssue`,
        // `addProjectV2ItemById` and the board fields; the two dependency operations need an
        // item with a far end, so
        // those reach the refusal through the task written under the project instead.
        let error = match plan {
            Err(error) => error,
            Ok(plan) => {
                let mut child = task("T-1", "a step", status(StatusCategory::Todo, "Todo"));
                child.project = Some(plan);
                source
                    .write_task(&ItemWrite {
                        target: None,
                        item: child,
                        depends_on: vec![edge(("T-1", ItemKind::Task), ("I_far", ItemKind::Task))],
                    })
                    .await
                    .expect_err(operation)
            }
        };
        let SourceError::RateLimited {
            message: Some(said),
            ..
        } = &error
        else {
            panic!("{operation} refused for a rate limit reported as {error:?}");
        };
        assert!(
            said.contains(doing),
            "a limiter that caught {operation} says {said:?} rather than naming {doing:?}"
        );
    }
}

#[tokio::test]
async fn spending_the_last_of_the_budget_still_answers_rather_than_refusing() {
    // GitHub sets `x-ratelimit-remaining: 0` on the last request the budget allowed as
    // well as on the ones it then refuses. Reading the header alone made that successful
    // read a rate-limit failure — throwing away an answer it already had — and, once
    // refusals were retried, replayed a request that had already taken effect. A response
    // is a refusal because of its status or its own wording; a spent budget only explains
    // one.
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    fixture.spend_the_budget_on("board");
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let answered = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("a response that spent the last of the budget is still a response");
    assert_eq!(
        answered.items.len(),
        1,
        "the answer that response already carried was thrown away"
    );
    assert_eq!(
        fixture.requests("board"),
        1,
        "a request that had already taken effect was replayed"
    );

    // A write, where replaying is the half that costs something: the creation runs once
    // and exactly one issue ends up on the board.
    let fixture = board(vec![]);
    fixture.spend_the_budget_on("createIssue");
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":50,"retry_budget_ms":5_000}),
    );
    let landed = source
        .write_task(&write(task(
            "T-1",
            "Publish",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a write whose creation spent the last of the budget");
    assert_eq!(
        fixture.requests("createIssue"),
        1,
        "an issue creation that had already taken effect was sent a second time"
    );
    assert!(fixture.holds(&landed.0));
    assert_eq!(
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "createIssue")
            .count(),
        1,
        "and the board holds exactly the one issue that was asked for"
    );
}

/// A board holding a document of every shape that could be mistaken for something else.
///
/// `I_design` has sub-issues *and* the project marker, `I_loose` has neither, and
/// `I_filed` is a sub-issue of a project — so each of the three arms that decide between a
/// project and a task is present on a design issue, and each must lose to the prefix.
fn board_with_documents() -> Fixture {
    board(vec![
        design("I_design", "Alpha design")
            .body("the engine core, reviewed\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\",\"caller.flags\":[true,null]}\n-->")
            .sub_issues(2)
            .labelled(&[("L_1", "bug")]),
        design("I_loose", "Loose note").body("filed nowhere"),
        design("I_filed", "Runbook")
            .body("how to read the alpha design")
            .parent("I_plan")
            .labelled(&[("L_3", "core")]),
        Item::issue("I_plan", "Engine").sub_issues(1),
        Item::issue("I_task", "Alpha engine").parent("I_plan"),
    ])
}

#[tokio::test]
async fn an_issue_titled_with_the_design_prefix_is_a_document_and_no_other_issue_is() {
    let fixture = board_with_documents();
    let source = source(&fixture);

    assert_eq!(
        selected_documents(source.as_ref(), &DocumentQuery::default()).await,
        ["I_design", "I_loose", "I_filed"],
        "every design-titled issue is a document, whatever else it looks like"
    );
    // Each of these would be something else if the prefix were read after the rule that
    // separates a project from a task: the first has sub-issues and the kind marker, the
    // second has neither and would be an empty project's twin, the third is a sub-issue.
    assert_eq!(
        selected_projects(source.as_ref(), &ProjectQuery::default()).await,
        ["I_plan"],
        "a design issue is never a project, whatever sub-issues or marker it carries"
    );
    assert_eq!(
        selected_tasks(source.as_ref(), &TaskQuery::default()).await,
        ["I_task"],
        "and never a task, whichever project it is filed under"
    );
    assert!(
        source
            .get_task(&NativeId("I_loose".to_owned()))
            .await
            .unwrap()
            .is_none()
            && source
                .get_project(&NativeId("I_design".to_owned()))
                .await
                .unwrap()
                .is_none(),
        "a design issue is not found by a task read or by a project read either"
    );

    let shown = source
        .get_document(&NativeId("I_design".to_owned()))
        .await
        .unwrap()
        .expect("a design issue reads back as a document");
    assert_eq!(
        shown.title, "Alpha design",
        "the reported title is the one a person wrote, without the prefix"
    );
    assert_eq!(shown.content.as_deref(), Some("the engine core, reviewed"));
    assert_eq!(shown.metadata["caller.flags"], json!([true, null]));
    assert!(
        !shown.metadata.contains_key(ItemKind::METADATA_KEY),
        "the kind marker is this source's own encoding and never travels as metadata"
    );
    assert_eq!(
        shown
            .labels
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["bug"]
    );
    assert_eq!(
        shown.project, None,
        "a document under no project is in none, exactly as a task is"
    );
    assert_eq!(
        source
            .get_document(&NativeId("I_filed".to_owned()))
            .await
            .unwrap()
            .expect("the filed document")
            .project,
        Some(NativeId("I_plan".to_owned())),
        "and one filed under a project issue is in that project"
    );
    assert!(
        source
            .get_document(&NativeId("I_task".to_owned()))
            .await
            .unwrap()
            .is_none(),
        "an issue without the prefix is not a document"
    );
}

#[tokio::test]
async fn every_predicate_a_document_query_carries_is_applied_before_it_is_paged() {
    let fixture = board_with_documents();
    let source = source(&fixture);

    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(label_filter(&["bug"], &[], &[]), ProjectFilter::Any, None)
        )
        .await,
        ["I_design"]
    );
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(label_filter(&[], &[], &["bug"]), ProjectFilter::Any, None)
        )
        .await,
        ["I_loose", "I_filed"]
    );
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(
                LabelFilter::default(),
                ProjectFilter::Is(NativeId("I_plan".to_owned())),
                None
            )
        )
        .await,
        ["I_filed"]
    );
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(LabelFilter::default(), ProjectFilter::Orphans, None)
        )
        .await,
        ["I_design", "I_loose"]
    );
    // The reported title is what a title search reads, so the prefix is not searchable
    // text: a person searching for what they wrote finds it, and one searching for the
    // encoding finds nothing.
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(
                LabelFilter::default(),
                ProjectFilter::Any,
                text("alpha design", TextFields::Title)
            )
        )
        .await,
        ["I_design"]
    );
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(
                LabelFilter::default(),
                ProjectFilter::Any,
                text("alpha design", TextFields::Content)
            )
        )
        .await,
        ["I_filed"]
    );
    assert_eq!(
        selected_documents(
            source.as_ref(),
            &document_query(
                LabelFilter::default(),
                ProjectFilter::Any,
                text("alpha design", TextFields::TitleOrContent)
            )
        )
        .await,
        ["I_design", "I_filed"]
    );
    assert!(
        selected_documents(
            source.as_ref(),
            &document_query(
                LabelFilter::default(),
                ProjectFilter::Any,
                text(DESIGN_TITLE_PREFIX, TextFields::TitleOrContent)
            )
        )
        .await
        .is_empty(),
        "the prefix is this source's encoding, not text a person wrote"
    );

    // Filtered before paged: a page of a filtered result is a page of the survivors.
    let first = source
        .query_documents(
            &document_query(label_filter(&[], &[], &["bug"]), ProjectFilter::Any, None),
            &page(1),
        )
        .await
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|d| d.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_loose"]
    );
    let cursor = first.next.expect("a second page").0;
    let second = source
        .query_documents(
            &document_query(label_filter(&[], &[], &["bug"]), ProjectFilter::Any, None),
            &resume(&cursor, 1),
        )
        .await
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|d| d.id.0.as_str())
            .collect::<Vec<_>>(),
        ["I_filed"]
    );
    assert!(second.next.is_none(), "the walk reached the end");
}

#[tokio::test]
async fn a_task_reports_the_issue_number_alone_as_its_key_and_a_draft_reports_none() {
    // The short handle this backend shows people is the issue's **number alone**, as a
    // decimal string — `1043`, never `owner/repo#1043` — and the native id stays the
    // issue's GraphQL node id beside it. A draft has no number at all: `DraftIssue`
    // declares none, so it reports no handle rather than one of some other shape.
    let fixture = board(vec![
        Item::issue("I_task", "Alpha engine").number(1043),
        Item::issue("I_other", "Beta").number(7),
        Item::draft("D_1", "a draft"),
    ]);
    let source = source(&fixture);

    let task = source
        .get_task(&NativeId("I_task".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.key.as_deref(), Some("1043"));
    assert_eq!(
        task.id,
        NativeId("I_task".to_owned()),
        "the node id is what everything stores and matches on, and the key never displaces it"
    );

    // A second issue, so the handle is read off each item rather than being one constant
    // this board happens to answer everything with.
    assert_eq!(
        source
            .get_task(&NativeId("I_other".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .key
            .as_deref(),
        Some("7")
    );

    assert_eq!(
        source
            .get_task(&NativeId("D_1".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .key,
        None,
        "a draft is filed in no repository, so nothing numbered it"
    );

    // And a listing reports it too, so one verb cannot carry the handle while another
    // drops it: the board query and the node read compose the same fragment.
    let listed = source
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .unwrap()
        .items;
    let keys: Vec<(String, Option<String>)> = listed
        .iter()
        .map(|task| (task.id.0.clone(), task.key.clone()))
        .collect();
    assert!(
        keys.contains(&("I_task".to_owned(), Some("1043".to_owned())))
            && keys.contains(&("I_other".to_owned(), Some("7".to_owned())))
            && keys.contains(&("D_1".to_owned(), None)),
        "{keys:?}"
    );
}

#[tokio::test]
async fn an_issue_this_run_created_reports_its_key_before_the_board_read_catches_up() {
    // The creating mutation is the only place a run learns the new issue's number, because
    // GitHub's own board read is behind for a moment after a create — so an item this run
    // made and then read back answers out of what this source remembered, and an item
    // remembered without its number would report no handle for the rest of the run.
    let fixture = board(vec![Item::issue("I_plan", "Engine").sub_issues(0)]);
    let source = source(&fixture);

    let created = source
        .write_task(&write(task(
            "ignored",
            "Third step",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a task this board accepts");

    let read = source
        .get_task(&created)
        .await
        .expect("this source answers")
        .expect("the item it just created is there");
    assert_eq!(
        read.key.as_deref(),
        Some("2001"),
        "the number the creating mutation answered with is the handle this item reports"
    );
    assert_eq!(
        read.id, created,
        "and its node id is the one the write returned"
    );
}

#[tokio::test]
async fn an_issue_created_without_a_number_reports_no_key_rather_than_failing_the_write() {
    // GitHub declares `Issue.number` non-null, so this is a response that should not
    // happen — and a landed write is not worth failing over a member that came back
    // missing anyway. The item reports no handle until a read of the board catches up,
    // which is the same answer as a backend that has none, and the write still lands
    // whole: this is the lenient branch, driven rather than assumed.
    let fixture = board(vec![Item::issue("I_plan", "Engine").sub_issues(0)]);
    fixture.creation_reports_no_number();
    let source = source(&fixture);

    let created = source
        .write_task(&write(task(
            "ignored",
            "Third step",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a create whose answer omits the number still lands");

    let read = source
        .get_task(&created)
        .await
        .expect("this source answers")
        .expect("the item it just created is there");
    assert_eq!(
        read.key, None,
        "no handle, rather than a guess or a failure"
    );
    assert_eq!(
        read.title, "Third step",
        "and the rest of the write is exactly what it was"
    );
}

#[tokio::test]
async fn a_creation_answering_with_an_unreadable_number_is_refused_and_the_issue_taken_back() {
    // Unlike a missing number, an unreadable one is not guessed at as *no handle*; and the
    // issue already exists by then, so the refusal takes it back rather than leave it in the
    // repository on no board.
    let fixture = board(vec![Item::issue("I_plan", "Engine").sub_issues(0)]);
    fixture.creation_reports_an_unreadable_number();
    let source = source(&fixture);

    let message = refusal(
        source
            .write_task(&write(task(
                "ignored",
                "Third step",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a created issue whose number cannot be read is refused"),
    );
    assert!(
        message.contains("GitHub created issue number is not an unsigned integer"),
        "the refusal names what it could not read: {message}"
    );
    assert!(
        fixture.seen().iter().any(|call| call[0] == "deleteIssue"),
        "the issue this call created was left behind: {:?}",
        fixture.seen()
    );
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .iter()
            .all(|held| held.title != "Third step"),
        "and the board holds nothing this failed write made"
    );
}

#[tokio::test]
async fn a_board_that_will_not_take_the_issue_back_still_reports_why_the_write_failed() {
    // The take-back is best effort on purpose: the caller is told why the *write* failed,
    // not why the tidy-up did, because a cleanup failure reported in place of the cause
    // would name the wrong problem — and the residue a refused cleanup leaves is real, so
    // the attempt is made and its outcome is what this pins. Nothing else in this suite
    // reaches a delete that fails, so the discarded result is unproved without it.
    let stuck = board(vec![Item::issue("I_plan", "Engine").sub_issues(0)]);
    stuck.creation_reports_an_unreadable_number();
    stuck.refuse("deleteIssue");
    let source = source(&stuck);

    let message = refusal(
        source
            .write_task(&write(task(
                "ignored",
                "Third step",
                status(StatusCategory::Todo, "Todo"),
            )))
            .await
            .expect_err("a created issue whose number cannot be read is still refused"),
    );
    assert!(
        message.contains("GitHub created issue number is not an unsigned integer"),
        "the cause survives the failed cleanup rather than being replaced by it: {message}"
    );
    assert!(
        !message.contains("deleteIssue"),
        "and the cleanup's own failure is not what the caller is told: {message}"
    );
    assert!(
        stuck.seen().iter().any(|call| call[0] == "deleteIssue"),
        "the take-back was attempted even though this board refuses it: {:?}",
        stuck.seen()
    );
}

#[tokio::test]
async fn an_update_keeps_the_destinations_own_key_whatever_the_incoming_task_carried() {
    // A key is read-only, so an `ItemWrite` arriving with one is an item read somewhere
    // that has a handle — a Linear issue's `ENG-123` — on its way into an item this board
    // already numbered. The handle stays the destination's own.
    //
    // A list either side of the write is what makes that a real question rather than one
    // the board answers for free: a write records what it wrote over this source's own
    // view of the board, so the second list answers out of the record the update built
    // rather than by asking GitHub again. An update that carried no number into that
    // record would have the item report no handle for the rest of the run, while a direct
    // read of the issue went on reporting one.
    let fixture = board(vec![Item::issue("I_1", "one").number(4242).status("Todo")]);
    let source = source(&fixture);

    let before = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("this board lists")
        .items;
    assert_eq!(
        before
            .iter()
            .map(|task| (task.id.0.as_str(), task.key.as_deref()))
            .collect::<Vec<_>>(),
        vec![("I_1", Some("4242"))]
    );

    let mut revised = task(
        "ignored",
        "one, revised",
        status(StatusCategory::Todo, "Todo"),
    );
    revised.key = Some("ENG-999".to_owned());
    revised.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let written = source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("this board takes the update");
    assert_eq!(
        written,
        NativeId("I_1".to_owned()),
        "the update addressed the item that was already there"
    );

    let after = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("this board lists")
        .items;
    assert_eq!(
        after
            .iter()
            .map(|task| (task.id.0.as_str(), task.title.as_str(), task.key.as_deref()))
            .collect::<Vec<_>>(),
        vec![("I_1", "one, revised", Some("4242"))],
        "the destination's own number, not the handle the write carried, and the rest of \
         the update landed with it"
    );

    // And a direct read of the issue says the same, so the two halves cannot disagree.
    assert_eq!(
        source
            .get_task(&written)
            .await
            .expect("this source answers")
            .expect("the item it just updated is there")
            .key
            .as_deref(),
        Some("4242")
    );
}

#[tokio::test]
async fn an_issue_says_where_it_is_as_a_link_and_a_draft_says_nothing_at_all() {
    // A draft has no web address of its own, so this source does not say where it is —
    // which is not the same as saying it is nowhere. Read first, before the binding below
    // shadows the constructor.
    let drafts = board(vec![Item::draft("D_1", "a draft")]);
    assert_eq!(
        source(&drafts)
            .get_task(&NativeId("D_1".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .location,
        None
    );

    let fixture = board_with_documents();
    let source = source(&fixture);
    let link = |id: &str| Some(Location::Url(format!("https://github.example/{id}")));

    assert_eq!(
        source
            .get_document(&NativeId("I_loose".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .location,
        link("I_loose")
    );
    let task = source
        .get_task(&NativeId("I_task".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.location, link("I_task"));
    assert_eq!(
        task.url.as_deref(),
        Some("https://github.example/I_task"),
        "the location says what the url field already reported, and does not replace it"
    );
    assert_eq!(
        source
            .get_project(&NativeId("I_plan".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .location,
        link("I_plan")
    );
}

#[tokio::test]
async fn a_document_written_to_this_board_puts_the_prefix_back_and_round_trips_intact() {
    let fixture = board(vec![Item::issue("I_plan", "Engine").sub_issues(0)]);
    let source = source(&fixture);
    let mut item = document("D-1", "Alpha design");
    item.content = Some("the engine core, reviewed".to_owned());
    item.project = Some(NativeId("I_plan".to_owned()));
    item.metadata = BTreeMap::from([
        ("caller.flags".to_owned(), json!([true, null])),
        ("caller.shape".to_owned(), json!({"nested": 3.5})),
        ("onetaskgraph.origin".to_owned(), json!("notes:D-1")),
    ]);

    let created = source
        .write_document(&write(item.clone()))
        .await
        .expect("a document copies onto this board");
    assert_eq!(
        fixture.item(&created.0).title,
        format!("{DESIGN_TITLE_PREFIX}Alpha design"),
        "the issue on the board carries the prefix, so the board reads as one too"
    );

    let read = source
        .get_document(&created)
        .await
        .unwrap()
        .expect("the created document reads back");
    assert_eq!(
        read.title, "Alpha design",
        "and the title that comes back out is the title that went in"
    );
    assert_eq!(read.content.as_deref(), Some("the engine core, reviewed"));
    assert_eq!(read.project, Some(NativeId("I_plan".to_owned())));
    assert_eq!(read.metadata["caller.flags"], json!([true, null]));
    assert_eq!(read.metadata["caller.shape"], json!({"nested": 3.5}));
    assert_eq!(read.metadata["onetaskgraph.origin"], json!("notes:D-1"));
    assert!(
        !read.metadata.contains_key(ItemKind::METADATA_KEY),
        "a document is told by its title, so nothing marks it as a kind of work"
    );
    assert!(
        source.get_task(&created).await.unwrap().is_none()
            && source.get_project(&created).await.unwrap().is_none(),
        "what this write created is a document and nothing else"
    );

    // A second copy of the same document updates the one already there.
    let mut revised = item.clone();
    revised.title = "Alpha design, revised".to_owned();
    let again = source
        .write_document(&ItemWrite {
            target: Some(created.clone()),
            item: revised,
            depends_on: vec![],
        })
        .await
        .expect("the second copy lands on the item the first one wrote");
    assert_eq!(again, created);
    assert_eq!(
        selected_documents(source.as_ref(), &DocumentQuery::default()).await,
        std::slice::from_ref(&created.0),
        "exactly one where there was one before"
    );
    assert_eq!(
        source.get_document(&created).await.unwrap().unwrap().title,
        "Alpha design, revised"
    );

    // And the undo a copy that cannot finish performs takes it back off the board.
    source
        .delete_document(&created)
        .await
        .expect("a document this run created is removable");
    assert!(!fixture.holds(&created.0));
    assert!(
        selected_documents(source.as_ref(), &DocumentQuery::default())
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn an_item_this_run_created_reads_back_whole_while_the_board_is_still_behind() {
    // GitHub's board read is eventually consistent, so a read that follows a write closely
    // enough is answered out of this source's own record of what it created — and until
    // now that record held the composed body, metadata slot and all, and no web address at
    // all. The live lane caught it: a document written and read back in one run came back
    // with its content carrying the encoding and nowhere a reader could open. Held one item
    // behind here so the record is what answers, which is the only state it is visible in.
    let fixture = board(vec![Item::issue("I_plan", "Engine").sub_issues(1)]);
    let source = source(&fixture);

    let mut design = document("D-1", "Alpha design");
    design.content = Some("the engine core, reviewed".to_owned());
    design.project = Some(NativeId("I_plan".to_owned()));
    design.metadata = BTreeMap::from([("caller.flags".to_owned(), json!([true, null]))]);
    let created = source
        .write_document(&write(design))
        .await
        .expect("a document copies onto this board");
    // Held behind once the document exists, so it is the document the board has not
    // caught up on: held behind before the write, the board would have hidden the project
    // it is filed under instead, which a write refuses rather than files blind.
    fixture.read_behind(1);

    let read = source
        .get_document(&created)
        .await
        .unwrap()
        .expect("a board read that has not caught up still holds what this run created");
    assert_eq!(read.title, "Alpha design");
    assert_eq!(
        read.content.as_deref(),
        Some("the engine core, reviewed"),
        "the content a person wrote, not the body this source composed around it"
    );
    assert_eq!(read.metadata["caller.flags"], json!([true, null]));
    assert_eq!(
        read.url.as_deref(),
        Some(format!("https://github.example/{}", created.0).as_str())
    );
    assert_eq!(
        read.location,
        Some(Location::Url(format!(
            "https://github.example/{}",
            created.0
        ))),
        "an issue this run created is somewhere a reader can open from the moment it exists"
    );

    // The same of the work on the same board: this is one record for all three kinds.
    let mut work = task("T-1", "Ship it", status(StatusCategory::Todo, "Todo"));
    work.content = Some("the plan, written out".to_owned());
    work.metadata = BTreeMap::from([("caller.shape".to_owned(), json!({"nested": 3.5}))]);
    let written = source
        .write_task(&write(work))
        .await
        .expect("a task copies onto this board");
    let held = source
        .get_task(&written)
        .await
        .unwrap()
        .expect("and reads back the same way");
    assert_eq!(held.content.as_deref(), Some("the plan, written out"));
    assert_eq!(held.metadata["caller.shape"], json!({"nested": 3.5}));
    assert_eq!(
        held.location,
        Some(Location::Url(format!(
            "https://github.example/{}",
            written.0
        )))
    );
}

#[tokio::test]
async fn a_document_write_names_every_field_and_target_this_board_cannot_carry() {
    let fixture = board(vec![Item::issue("I_1", "held").labelled(&[("L_1", "bug")])]);
    let source = source(&fixture);

    let stale = refusal(
        source
            .write_document(&ItemWrite {
                target: Some(NativeId("I_missing".to_owned())),
                item: document("D-1", "Alpha design"),
                depends_on: vec![],
            })
            .await
            .expect_err("a target this board does not hold"),
    );
    assert!(stale.contains("I_missing"), "{stale}");

    let mut labelled = document("D-1", "Alpha design");
    labelled.labels = vec![Label {
        id: NativeId("L_1".to_owned()),
        name: "bug".to_owned(),
        color: None,
    }];
    let labels = refusal(
        source
            .write_document(&write(labelled))
            .await
            .expect_err("a label this destination cannot create"),
    );
    assert!(labels.contains("labels"), "{labels}");

    // A document takes part in no dependency graph, so a caller naming one is told so
    // rather than having it recorded under the reserved key, where a later read would
    // report an edge the contract says cannot exist.
    let depending = refusal(
        source
            .write_document(&ItemWrite {
                target: None,
                item: document("D-1", "Alpha design"),
                depends_on: vec![DependencyEdge {
                    from: DependencyEndpoint::from_native(
                        NativeId("D-1".to_owned()),
                        ItemKind::Task,
                    ),
                    to: DependencyEndpoint::from_native(NativeId("I_1".to_owned()), ItemKind::Task),
                    kind: DependencyKind::Blocks,
                }],
            })
            .await
            .expect_err("a dependency on a document"),
    );
    assert!(depending.contains("no dependency graph"), "{depending}");
    assert_eq!(
        fixture.state.lock().unwrap().items.len(),
        1,
        "every one of those refusals happens before anything is created"
    );
}

#[tokio::test]
async fn a_task_or_project_titled_the_way_this_board_spells_a_document_is_refused_by_name() {
    // Written, it would land as an issue this same source reads back as a document — so
    // the field this destination cannot carry is named rather than silently reclassified.
    let fixture = board(vec![]);
    let source = source(&fixture);
    let title = format!("{DESIGN_TITLE_PREFIX}Alpha design");

    for message in [
        refusal(
            source
                .write_task(&write(task(
                    "T-1",
                    &title,
                    status(StatusCategory::Todo, "Todo"),
                )))
                .await
                .expect_err("a task titled as a document"),
        ),
        refusal(
            source
                .write_project(&write(project(
                    "P-1",
                    &title,
                    status(StatusCategory::Todo, "Todo"),
                )))
                .await
                .expect_err("a project titled as a document"),
        ),
    ] {
        assert!(message.contains(DESIGN_TITLE_PREFIX), "{message}");
        assert!(message.contains("retitle it"), "{message}");
    }
    assert!(
        fixture.state.lock().unwrap().items.is_empty(),
        "the refusal comes before anything is created"
    );
}

#[tokio::test]
async fn a_dependency_far_end_this_board_holds_as_a_document_is_refused_by_name() {
    // Nothing may point at a document, and `ItemKind` has no variant for one, so neither
    // answer a read could give would be true: reporting it as a task names an id no task
    // read of this source can find, and reporting it as a project names one no project
    // read can.
    let fixture = board(vec![
        Item::issue("I_1", "Alpha engine"),
        design("I_design", "Alpha design"),
    ]);
    let source = source(&fixture);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_1".to_owned())),
            item: task("I_1", "Alpha engine", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![],
        })
        .await
        .expect("a write that changes nothing");
    fixture
        .state
        .lock()
        .unwrap()
        .blocked_by
        .insert("I_1".to_owned(), vec!["I_design".to_owned()]);

    let message = refusal(
        source
            .task_dependencies(&NativeId("I_1".to_owned()), Direction::DependsOn, &page(10))
            .await
            .expect_err("a far end this board holds as a document"),
    );
    assert!(message.contains("I_design"), "{message}");
    assert!(message.contains("is a document"), "{message}");

    // And the write side settles it in the same place, in the sentence a disagreeing kind
    // already had: no caller can name a document's kind correctly.
    let named = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("I_1".to_owned())),
                item: task("I_1", "Alpha engine", status(StatusCategory::Todo, "Todo")),
                depends_on: vec![DependencyEdge {
                    from: DependencyEndpoint::from_native(
                        NativeId("I_1".to_owned()),
                        ItemKind::Task,
                    ),
                    to: DependencyEndpoint::from_native(
                        NativeId("I_design".to_owned()),
                        ItemKind::Task,
                    ),
                    kind: DependencyKind::Blocks,
                }],
            })
            .await
            .expect_err("a dependency on a document"),
    );
    assert!(
        named.contains("I_design") && named.contains("document"),
        "{named}"
    );
}

/// One session's worth of reads and one write, driven the way a caller drives them.
///
/// Shared by the accounting tests below so each of them measures the same session and the
/// figures they assert on are comparable between them — which is the property a report is
/// for.
async fn drive_a_session(source: &dyn TaskSource) {
    source
        .query_tasks(&TaskQuery::default(), &page(50))
        .await
        .expect("a task read");
    source
        .query_projects(&ProjectQuery::default(), &page(50))
        .await
        .expect("a project read");
    source
        .get_task(&NativeId("I_step".to_owned()))
        .await
        .expect("one task by id");
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_step".to_owned())),
            item: task(
                "I_step",
                "step renamed",
                status(StatusCategory::Todo, "Todo"),
            ),
            depends_on: vec![],
        })
        .await
        .expect("a write");
}

fn accounted_board() -> Fixture {
    board(vec![
        Item::issue("I_plan", "plan").sub_issues(1),
        Item::issue("I_step", "step").parent("I_plan"),
    ])
}

fn budget_of(session: &Session) -> BudgetReport {
    session
        .budgets()
        .into_iter()
        .find(|report| report.budget() == Budget::Graphql)
        .expect("the session drew on the GraphQL budget")
}

/// Every request the board served is one the accounting recorded, and the report adds them
/// up the way a person compares two runs.
///
/// The count is compared against what the *fixture* served rather than against a number
/// written here: a request path that sent something without recording it fails this, which
/// is the only way completeness is provable rather than asserted.
#[tokio::test]
async fn the_session_report_counts_every_request_the_board_served_and_what_each_cost() {
    let fixture = accounted_board();
    let ledger = Arc::new(Accounting::new());
    let source = recording(&fixture.endpoint, &ledger);
    drive_a_session(source.as_ref()).await;

    let session = ledger.snapshot();
    let served = fixture.documents().len();
    assert_eq!(
        session.total_requests(),
        served,
        "the board served {served} requests and the accounting recorded {}",
        session.total_requests()
    );

    // Every GraphQL record is named out of the inventory rather than out of a second list,
    // and carries the node count the same offline calculation computes.
    let described = graphql::DOCUMENTS
        .iter()
        .map(|(_, doing)| *doing)
        .collect::<Vec<_>>();
    for request in session.requests() {
        assert!(
            described.contains(&request.name()),
            "{} is not one of this source's documents",
            request.name()
        );
        assert_eq!(request.budget(), Budget::Graphql);
        assert_eq!(request.outcome(), Outcome::Answered);
        assert!(
            request.node_count().is_some(),
            "{} carries no node count",
            request.name()
        );
    }
    assert!(
        session
            .requests()
            .iter()
            .any(|request| request.mode() == Mode::Write),
        "the write was recorded as a read"
    );
    assert_eq!(
        session.total_node_count(),
        session
            .requests()
            .iter()
            .filter_map(Request::node_count)
            .sum::<u64>()
    );

    // The count is the offline calculation's own answer for the document that was sent,
    // under the page sizes that request bound — this source walks a board at its full page
    // size, so that is the worst case for this one.
    let board_read = session
        .requests()
        .iter()
        .find(|request| request.name() == "reading the board")
        .expect("the board was read");
    assert_eq!(
        board_read.node_count(),
        onetaskgraph_github_projects::worst_case_node_count(graphql::BOARD).ok()
    );
    // And it really is the bindings that decide it rather than the document alone: the same
    // document over a tenth of the page costs a tenth of the nodes.
    let narrower = Request::graphql(graphql::BOARD, &json!({"first":10}), None, None)
        .answered(RateLimit::default());
    assert!(
        narrower.node_count() < board_read.node_count(),
        "a smaller page bound {:?} should cost fewer nodes than {:?}",
        narrower.node_count(),
        board_read.node_count()
    );

    let report = session.report();
    assert!(
        report.contains(&format!("requests {}", session.total_requests())),
        "{report}"
    );
    assert!(report.contains("reading the board"), "{report}");
    assert!(report.contains("reading one issue"), "{report}");
    assert!(
        report.contains(&format!("node count {}", session.total_node_count())),
        "{report}"
    );
    assert!(
        report.contains("budget graphql, metered in points"),
        "{report}"
    );
    // No credential, no token, no issue body and no board content.
    assert!(!report.contains("test-token"), "{report}");
    assert!(!report.contains("step renamed"), "{report}");
    assert!(!report.contains("octo-org"), "{report}");
}

/// A source nobody handed a ledger to still accounts for itself, and hands back a value.
///
/// [`GitHubProjectsSource::accounting`] is the read every caller that did **not** supply an
/// accounting has — which is every source the registry builds, because
/// `SourcePlugin::build` gives one an accounting of its own. So what proves it is a source
/// built that way, driven the same way, and asked afterwards what it cost. It also proves
/// the snapshot is a value rather than a live borrow: one taken early is compared with one
/// taken later, and the early one has not grown.
#[tokio::test]
async fn a_source_that_was_handed_no_ledger_still_accounts_for_itself() {
    let fixture = accounted_board();
    let config = serde_json::from_value(fixture_config(&fixture.endpoint, &json!({})))
        .expect("the fixture configuration deserializes");
    let source = onetaskgraph_github_projects::GitHubProjectsSource::new(
        &SourceName::new("work").unwrap(),
        config,
        &Secrets,
    )
    .expect("a usable source");

    source
        .query_tasks(&TaskQuery::default(), &page(50))
        .await
        .expect("a task read");
    let early = source.accounting();
    let early_count = early.total_requests();
    assert_eq!(early_count, fixture.documents().len());
    assert!(early_count > 0);

    drive_a_session(&source).await;
    let whole = source.accounting();
    assert_eq!(
        whole.total_requests(),
        fixture.documents().len(),
        "the board served {} requests and this source's own accounting recorded {}",
        fixture.documents().len(),
        whole.total_requests()
    );
    assert!(whole.total_requests() > early_count);
    assert_eq!(
        early.total_requests(),
        early_count,
        "the earlier snapshot grew with the source after it was taken, so it is a live \
         borrow rather than a value two runs could be compared with"
    );
    assert_eq!(budget_of(&whole).limit(), Some(FIXTURE_BUDGET_LIMIT));
    assert!(
        whole.report().contains("reading the board"),
        "{}",
        whole.report()
    );
}

/// The budget figures come out of the headers the board's own answers carried.
///
/// A report that could only fill these in against the real API would be an instrument
/// nobody could check, so the fixture answers with GitHub's own rate-limit headers and this
/// holds the report to what they said.
#[tokio::test]
async fn the_reported_budget_figures_are_the_ones_the_responses_own_headers_carried() {
    let fixture = accounted_board();
    let ledger = Arc::new(Accounting::new());
    let source = recording(&fixture.endpoint, &ledger);
    drive_a_session(source.as_ref()).await;

    let session = ledger.snapshot();
    let graphql = budget_of(&session);
    let used = fixture.budget_used();
    assert_eq!(graphql.limit(), Some(FIXTURE_BUDGET_LIMIT));
    assert_eq!(graphql.used_by_the_account(), Some(used));
    assert_eq!(
        graphql.remaining_last_seen(),
        Some(FIXTURE_BUDGET_LIMIT - used)
    );
    // Two readings from two responses rather than one reading twice: this board reports the
    // account one point poorer with every answer, so the first response this session got and
    // the last one carried different figures and the report keeps them apart.
    assert_eq!(
        graphql.remaining_first_seen(),
        Some(FIXTURE_BUDGET_LIMIT - 1)
    );
    assert!(graphql.remaining_first_seen() > graphql.remaining_last_seen());
    // The session's own spend is attributed per call rather than read off the account.
    assert_eq!(graphql.attributed(), session.total_requests() as u64);
    assert_eq!(graphql.modelled(), graphql.attributed());
    assert_eq!(graphql.reported(), 0);

    let report = session.report();
    assert!(
        report.contains(&format!(
            "limit {FIXTURE_BUDGET_LIMIT}, {used} used, {} remaining",
            FIXTURE_BUDGET_LIMIT - used
        )),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "is attributed {} points — measured only where GitHub reported it: 0 reported by GitHub",
            graphql.attributed()
        )),
        "{report}"
    );
}

/// A shared account falling faster than this session spends does not move what this session
/// is reported to have spent.
///
/// The same drive is run twice against two boards that differ only in how fast something
/// *else* is spending the same budget. A report built by subtracting a remaining allowance
/// at the end from one at the start would give two different answers; one attributed per
/// call gives the same answer twice, and says on its face that the movement it also shows
/// is the account's.
#[tokio::test]
async fn a_budget_something_else_is_spending_does_not_move_this_sessions_reported_spend() {
    let alone = accounted_board();
    let alone_ledger = Arc::new(Accounting::new());
    drive_a_session(recording(&alone.endpoint, &alone_ledger).as_ref()).await;
    let alone_session = alone_ledger.snapshot();

    let shared = accounted_board();
    // Nine points of somebody else's work between each of this session's own requests.
    shared.other_traffic(9);
    let shared_ledger = Arc::new(Accounting::new());
    drive_a_session(recording(&shared.endpoint, &shared_ledger).as_ref()).await;
    let shared_session = shared_ledger.snapshot();

    let alone_budget = budget_of(&alone_session);
    let shared_budget = budget_of(&shared_session);
    assert_eq!(
        alone_session.total_requests(),
        shared_session.total_requests()
    );
    assert_eq!(
        shared_budget.attributed(),
        alone_budget.attributed(),
        "the session's spend moved with the account's allowance"
    );
    assert!(
        shared_budget.account_allowance_fall() > alone_budget.account_allowance_fall(),
        "the shared board's allowance was supposed to fall faster"
    );
    assert!(
        shared_budget.account_allowance_fall().unwrap() > shared_budget.attributed(),
        "the account's allowance fell by {:?} and this session is attributed {}",
        shared_budget.account_allowance_fall(),
        shared_budget.attributed()
    );
    let report = shared_session.report();
    assert!(
        report.contains(
            "that is the account's own consumption and not this session's spend, because \
             other work draws on the same budget in the same window"
        ),
        "{report}"
    );
}

/// An answer, a refusal and a rate-limited refusal are three outcomes, and only the third
/// is attributed nothing.
#[tokio::test]
async fn a_refusal_and_a_rate_limited_refusal_are_told_apart_and_only_one_of_them_spends() {
    let refusing = accounted_board();
    refusing.refuse("updateIssue");
    let refusing_ledger = Arc::new(Accounting::new());
    let source = recording(&refusing.endpoint, &refusing_ledger);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_step".to_owned())),
            item: task("I_step", "refused", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![],
        })
        .await
        .expect_err("this board refuses that mutation");
    let refused = refusing_ledger.snapshot();
    let refused_record = refused
        .requests()
        .iter()
        .find(|request| request.outcome() == Outcome::Refused)
        .expect("a refusal was recorded");
    assert_eq!(refused_record.name(), "updating an issue");
    assert_eq!(refused_record.mode(), Mode::Write);
    assert_eq!(refused_record.spend().basis(), Basis::Modelled);
    assert_eq!(refused_record.spend().amount(), 1);

    let limited = accounted_board();
    limited.refuse_every_mutation();
    let limited_ledger = Arc::new(Accounting::new());
    let source = recording(&limited.endpoint, &limited_ledger);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_step".to_owned())),
            item: task("I_step", "limited", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![],
        })
        .await
        .expect_err("this board refuses every mutation for a rate limit");
    let session = limited_ledger.snapshot();
    let refusal = session
        .requests()
        .iter()
        .find(|request| request.outcome() == Outcome::RateLimited)
        .expect("a rate-limited refusal was recorded");
    assert_eq!(refusal.spend().basis(), Basis::NotRun);
    assert_eq!(refusal.spend().amount(), 0);
    assert!(
        session.report().contains("1 rate-limited"),
        "{}",
        session.report()
    );

    // A refusal for a spent primary budget is the response whose headers say most about that
    // budget's state, so a request attributed nothing still contributes what its own response
    // reported. Counting the refusal and reading its headers are two different things, and a
    // report that stopped reading them here would say the allowance was never reported on the
    // one run where it mattered.
    let exhausted = raw_server_with_headers(
        "403 Forbidden",
        "{}",
        "x-ratelimit-limit: 5000\r\nx-ratelimit-used: 5000\r\nx-ratelimit-remaining: 0\r\n",
    );
    let exhausted_ledger = Arc::new(Accounting::new());
    recording(&exhausted, &exhausted_ledger)
        .query_tasks(&TaskQuery::default(), &page(50))
        .await
        .expect_err("that board has nothing left of its budget");
    let session = exhausted_ledger.snapshot();
    let budget = budget_of(&session);
    assert_eq!(budget.requests(), 1);
    assert_eq!(budget.not_run(), 1);
    assert_eq!(
        budget.attributed(),
        0,
        "a request that never ran is attributed nothing"
    );
    assert_eq!(budget.limit(), Some(5000));
    assert_eq!(budget.used_by_the_account(), Some(5000));
    assert_eq!(budget.remaining_last_seen(), Some(0));
    assert!(
        budget
            .render()
            .contains("limit 5000, 5000 used, 0 remaining"),
        "{}",
        budget.render()
    );
}

/// A caller's own calls are recorded into the same session as the source's.
///
/// This is what the credentialed lane does with its schema verification, its board and
/// field lookups, its residue sweep and its cleanup — GraphQL and REST alike — so the
/// session total accounts for the whole session. Both budgets are kept apart, because
/// GitHub meters them apart.
#[tokio::test]
async fn a_callers_own_graphql_and_rest_calls_join_the_sources_in_one_session() {
    let fixture = accounted_board();
    let ledger = Arc::new(Accounting::new());
    let source = recording(&fixture.endpoint, &ledger);
    drive_a_session(source.as_ref()).await;
    let from_the_source = ledger.snapshot().total_requests();

    // A caller's REST call, named by the endpoint it addressed rather than by the URL it
    // built, with the rate-limit headers that response carried.
    let rest_headers = BTreeMap::from([
        // The board's own figures again rather than GitHub's published REST allowance, for
        // the reason `FIXTURE_BUDGET_LIMIT` gives: a number the report could have known
        // without reading a header proves nothing about whether it read one.
        ("x-ratelimit-limit".to_owned(), "1234".to_owned()),
        ("x-ratelimit-remaining".to_owned(), "1221".to_owned()),
        ("x-ratelimit-used".to_owned(), "13".to_owned()),
        ("x-ratelimit-resource".to_owned(), "core".to_owned()),
    ]);
    ledger.record(
        Request::rest(
            Endpoint::parse(Method::Get, "/repos/{owner}/{repo}/labels")
                .expect("a documented endpoint"),
        )
        .answered(RateLimit::read(|name| rest_headers.get(name).cloned())),
    );
    // And a caller's own GraphQL document, which the inventory does not name and which was
    // shaped so GitHub reported what it cost.
    ledger.record(
        Request::graphql(
            "query MutationContract { __typename }",
            &json!({}),
            Some("mutation contract introspection"),
            Some(37),
        )
        .answered(RateLimit::default()),
    );

    let session = ledger.snapshot();
    assert_eq!(session.total_requests(), from_the_source + 2);
    assert_eq!(session.attributed(Budget::Rest), 1);
    assert_eq!(
        session.attributed(Budget::Graphql),
        from_the_source as u64 + 37,
        "GitHub's own reported cost is what a call that reports one is attributed"
    );
    let rest = session
        .budgets()
        .into_iter()
        .find(|report| report.budget() == Budget::Rest)
        .expect("the session drew on the REST budget");
    assert_eq!(rest.counted(), 1);
    assert_eq!(rest.limit(), Some(1234));
    assert_eq!(rest.remaining_last_seen(), Some(1221));
    assert_eq!(budget_of(&session).reported(), 37);

    let report = session.report();
    assert!(
        report.contains("GET /repos/{owner}/{repo}/labels"),
        "{report}"
    );
    assert!(
        report.contains("mutation contract introspection"),
        "{report}"
    );
    assert!(
        report.contains("budget rest, metered in requests"),
        "{report}"
    );
    assert!(
        report.contains("budget graphql, metered in points"),
        "{report}"
    );
}

/// The public helpers a caller outside this crate records its own calls with.
///
/// The credentialed lane is that caller, and it skips wherever no credential was given, so
/// what makes these correct has to be provable without one: they read GitHub's refusal wordings through the
/// same limiter this source's own requests go through, so a secondary rate limit under a
/// forbidden status is a rate limit here exactly as it is there.
#[tokio::test]
async fn a_caller_tells_an_answer_from_a_refusal_from_a_rate_limit_the_way_this_source_does() {
    assert_eq!(
        Outcome::of_response(StatusCode::OK, false, "{}"),
        Outcome::Answered
    );
    assert_eq!(
        Outcome::of_response(StatusCode::NOT_FOUND, false, r#"{"message":"Not Found"}"#),
        Outcome::Refused
    );
    assert_eq!(
        Outcome::of_response(
            StatusCode::FORBIDDEN,
            false,
            r#"{"message":"You have exceeded a secondary rate limit."}"#
        ),
        Outcome::RateLimited
    );
    // A spent budget explains a failing response; it never turns a good answer into one.
    assert_eq!(
        Outcome::of_response(StatusCode::FORBIDDEN, true, r#"{"message":"Forbidden"}"#),
        Outcome::RateLimited
    );
    assert_eq!(
        Outcome::of_response(StatusCode::OK, true, "{}"),
        Outcome::Answered
    );

    let spent = RateLimit::read(|name| (name == "x-ratelimit-remaining").then(|| "0".to_owned()));
    assert!(spent.exhausted());
    assert!(!RateLimit::default().exhausted());
    // The one header here that is not a number is the one that could carry a third party's
    // arbitrary bytes, so a value not spelled like a resource name is dropped.
    let named = |value: &'static str| {
        RateLimit::read(move |name| (name == "x-ratelimit-resource").then(|| value.to_owned()))
    };
    assert_eq!(named("  graphql  ").resource(), Some("graphql"));
    assert_eq!(
        named("integration_manifest").resource(),
        Some("integration_manifest")
    );
    assert_eq!(named("<script>alert(1)</script>").resource(), None);
    assert_eq!(named("").resource(), None);
    // The five figures are read back through the accessors that are the only way to have
    // them, so a caller cannot assemble a set of headers no response ever carried.
    let observed = RateLimit::read(|name| {
        BTreeMap::from([
            ("x-ratelimit-limit", "4321"),
            ("x-ratelimit-remaining", "4300"),
            ("x-ratelimit-used", "21"),
            ("x-ratelimit-reset", "1788000000"),
        ])
        .get(name)
        .map(|value| (*value).to_owned())
    });
    assert_eq!(observed.limit(), Some(4321));
    assert_eq!(observed.remaining(), Some(4300));
    assert_eq!(observed.used_by_the_account(), Some(21));
    assert_eq!(observed.reset(), Some(1_788_000_000));
    assert_eq!(observed.resource(), None);
    assert_eq!(RateLimit::default().limit(), None);

    // The three allowance figures are one fact, so a response whose figures cannot all be
    // true carries no budget state at all: more left of a budget than the whole of it, or
    // more of it used than it holds, is not something to report as an observation. They go
    // together — which of the three is the wrong one is not knowable from a response — while
    // `reset` and `resource`, which are independent of them, survive.
    let impossible = |remaining: &'static str, used: &'static str| {
        RateLimit::read(move |name| {
            BTreeMap::from([
                ("x-ratelimit-limit", "4321"),
                ("x-ratelimit-remaining", remaining),
                ("x-ratelimit-used", used),
                ("x-ratelimit-reset", "1788000000"),
                ("x-ratelimit-resource", "graphql"),
            ])
            .get(name)
            .map(|value| (*value).to_owned())
        })
    };
    // The third of them is the one neither figure shows on its own: `used` and `remaining`
    // partition the allowance, so 4,000 spent and 4,000 left of 4,321 is impossible although
    // each figure is inside the limit.
    for (remaining, used) in [("4322", "21"), ("4300", "4322"), ("4000", "4000")] {
        let unreadable = impossible(remaining, used);
        assert_eq!(
            (
                unreadable.limit(),
                unreadable.remaining(),
                unreadable.used_by_the_account()
            ),
            (None, None, None),
            "remaining {remaining}, used {used} of a whole 4321 cannot all be true"
        );
        assert_eq!(unreadable.reset(), Some(1_788_000_000));
        assert_eq!(unreadable.resource(), Some("graphql"));
        // And nothing downstream reads a budget as exhausted off figures nobody observed.
        assert!(!unreadable.exhausted());
    }
    // The boundary itself is possible: an untouched allowance leaves the whole of it, and a
    // partly spent one has the two figures summing to exactly the whole.
    let whole = impossible("4321", "0");
    assert_eq!(whole.remaining(), Some(4321));
    assert_eq!(whole.limit(), Some(4321));
    let partly_spent = impossible("300", "4021");
    assert_eq!(partly_spent.remaining(), Some(300));
    assert_eq!(partly_spent.used_by_the_account(), Some(4021));
    // A pair falling short of the whole is kept rather than dropped, which is the deliberate
    // half of that rule: it accounts for less of the budget than exists rather than for more
    // than could, and reporting the budget as unknown would be the worse answer. The stand-in
    // board answers exactly this way where it stands in for an allowance somebody else spent.
    let short_of_the_whole = impossible("0", "21");
    assert_eq!(short_of_the_whole.limit(), Some(4321));
    assert_eq!(short_of_the_whole.remaining(), Some(0));
    assert_eq!(short_of_the_whole.used_by_the_account(), Some(21));
    assert!(short_of_the_whole.exhausted());

    for (spelled, method, mode) in [
        ("get", Method::Get, Mode::Read),
        ("HEAD", Method::Head, Mode::Read),
        (" post ", Method::Post, Mode::Write),
        ("put", Method::Put, Mode::Write),
        ("patch", Method::Patch, Mode::Write),
        ("delete", Method::Delete, Mode::Write),
    ] {
        assert_eq!(Method::parse(spelled), Some(method), "{spelled}");
        assert_eq!(method.mode(), mode, "{}", method.name());
        assert_eq!(Method::parse(method.name()), Some(method));
    }
    // A method HTTP has no verb for is refused where it is spelled rather than counted as a
    // write somewhere further on.
    assert_eq!(Method::parse("GTE"), None);
    assert_eq!(Method::parse(""), None);

    // A REST record: named by its endpoint, no node count, and one request against a budget
    // metered in requests however it ended — except a rate limiter's refusal, which never
    // ran.
    let endpoint = |method, path| Endpoint::parse(method, path).expect("a documented endpoint");
    let labels = endpoint(Method::Get, "/repos/{owner}/{repo}/labels");
    assert_eq!(labels.method(), Method::Get);
    assert_eq!(labels.path(), "/repos/{owner}/{repo}/labels");
    assert_eq!(labels.name(), "GET /repos/{owner}/{repo}/labels");
    let listed = Request::rest(labels.clone()).answered(RateLimit::default());
    assert_eq!(
        listed.call(),
        &onetaskgraph_github_projects::accounting::Call::Endpoint { endpoint: labels }
    );
    assert_eq!(listed.name(), "GET /repos/{owner}/{repo}/labels");
    assert_eq!(listed.node_count(), None);
    assert_eq!(listed.spend().basis(), Basis::Counted);
    assert_eq!(listed.spend().amount(), 1);
    let refused = Request::rest(endpoint(
        Method::Delete,
        "/repos/{owner}/{repo}/labels/{name}",
    ))
    .finished(Outcome::Refused, RateLimit::default());
    assert_eq!(refused.mode(), Mode::Write);
    assert_eq!(refused.spend().basis(), Basis::Counted);
    assert_eq!(refused.spend().amount(), 1);
    let limited = Request::rest(endpoint(Method::Post, "/repos/{owner}/{repo}/labels"))
        .finished(Outcome::RateLimited, RateLimit::default());
    assert_eq!(limited.spend().basis(), Basis::NotRun);
    assert_eq!(limited.spend().amount(), 0);
    // What a record may hold is settled where the endpoint is written: a URL a run built,
    // and everything else it could carry into a report that promises to carry none of it,
    // is refused there rather than named in the report.
    for refused in [
        "https://api.github.com/repos/octo-org/board/labels",
        "/repos/{owner}/{repo}/labels?per_page=100&token=secret",
        "/repos/{owner}/{repo}/labels#fragment",
        "repos/{owner}/{repo}/labels",
        "/repos//labels",
        "/repos/{owner}/{repo}/labels/a label",
        "/repos/{}/labels",
        "",
    ] {
        assert_eq!(
            Endpoint::parse(Method::Get, refused),
            None,
            "{refused} is not spelled like an endpoint template"
        );
    }

    // A document the calculation cannot rule on carries no node count, which is a defect in
    // the document rather than a cost of nothing.
    let uncountable = Request::graphql("this is not a GraphQL document", &json!({}), None, None)
        .answered(RateLimit::default());
    assert_eq!(uncountable.node_count(), None);
    assert_eq!(uncountable.name(), "talking to GitHub");
    let session = {
        let ledger = Accounting::new();
        ledger.record(uncountable);
        ledger.snapshot()
    };
    assert_eq!(session.total_node_count(), 0);
    assert!(
        session
            .report()
            .contains("node count 0 over 0 GraphQL requests that have one"),
        "{}",
        session.report()
    );
}

/// A request that never got an answer is recorded too, and carries no budget figures.
///
/// Both halves matter. A send that failed and a body that could not be read are the two ways
/// this source can end a request with nothing to read, and a session that quietly stopped
/// counting at either would report a cheap run where there was an expensive one. Neither
/// response says anything about the budget, so both records say so rather than guessing.
#[tokio::test]
async fn a_request_that_never_answered_is_recorded_as_refused_with_no_rate_limit_facts() {
    // Nothing is listening on port 1, so the send itself fails.
    let ledger = Arc::new(Accounting::new());
    let unreachable = recording("http://127.0.0.1:1/graphql", &ledger);
    let failure = refusal(
        unreachable
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("nothing is listening there"),
    );
    assert!(failure.contains("request failed"), "{failure}");
    let session = ledger.snapshot();
    assert_eq!(session.total_requests(), 1);
    let record = &session.requests()[0];
    assert_eq!(record.outcome(), Outcome::Refused);
    assert_eq!(record.rate_limit(), &RateLimit::default());
    assert_eq!(session.attributed(Budget::Graphql), 1);

    // And a response that promises more body than it sends, which fails while being read.
    let ledger = Arc::new(Accounting::new());
    let truncated = recording(&truncating_server(), &ledger);
    let failure = refusal(
        truncated
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .expect_err("that response cannot be read"),
    );
    // Named, so this proves the body-read path rather than the send path above it.
    assert!(failure.contains("response could not be read"), "{failure}");
    let session = ledger.snapshot();
    assert_eq!(session.total_requests(), 1);
    assert_eq!(session.requests()[0].outcome(), Outcome::Refused);
    assert_eq!(session.requests()[0].name(), "reading the board");
}

/// A server that promises a body far longer than the one it sends, then hangs up.
///
/// That is the one way to make reading a response's body fail without failing the send: the
/// status and the headers arrive, and the read of what they promised does not.
fn truncating_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{}",
            );
        }
    });
    format!("http://{address}/graphql")
}

/// The repository half of this fixture board, over REST.
///
/// The label lifecycle is the one thing the journey does over REST rather than GraphQL, and
/// a session's cost is metered against a different budget for it — so a board that answered
/// only GraphQL would leave a whole budget of the session report empty. It shares the board's
/// own state, so a label created here is a label the GraphQL side can attach.
fn label_endpoints(state: &Arc<Mutex<State>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    let served = Arc::clone(state);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("fixture connection");
            let (method, path, body) = board::read_http_request(&mut stream);
            let (status, payload) = answer_a_label_call(&served, &method, &path, body.as_ref());
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 x-ratelimit-limit: {FIXTURE_BUDGET_LIMIT}\r\n\
                 x-ratelimit-used: 1\r\n\
                 x-ratelimit-remaining: {}\r\n\
                 x-ratelimit-resource: core\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                FIXTURE_BUDGET_LIMIT - 1,
                payload.len()
            );
            stream.write_all(response.as_bytes()).expect("a response");
        }
    });
    host
}

/// GitHub's three label endpoints, as far as one repository of this board needs them.
fn answer_a_label_call(
    state: &Arc<Mutex<State>>,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> (&'static str, String) {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    // The allowance read the budget precondition makes before anything else: GitHub answers
    // it off this same host, one object per budget, and this board reports an account with
    // room so the journey below is the thing under test rather than the gate.
    if (method, path) == ("GET", "/rate_limit") {
        return ("200 OK", ample_allowance().to_string());
    }
    let rest = path
        .strip_prefix("/repos/acme/work/labels")
        .unwrap_or_else(|| panic!("the fixture repository received an unknown path: {path}"));
    let mut state = state.lock().unwrap();
    match (method, rest) {
        ("POST", "") => {
            let name = body.expect("a label body")["name"]
                .as_str()
                .expect("a label name")
                .to_owned();
            let node_id = format!("LA_{}", state.labels.len() + 1);
            state.seen.push(json!(["createLiveLabel", {"name":name}]));
            state.labels.push((name.clone(), node_id.clone()));
            (
                "201 Created",
                json!({"name":name,"node_id":node_id}).to_string(),
            )
        }
        ("GET", "") => {
            // Everything this repository holds is on the first page; a later page is empty,
            // which is how GitHub's own pagination ends.
            let held = if query.contains("page=1") {
                state
                    .labels
                    .iter()
                    .map(|(name, node_id)| json!({"name":name,"node_id":node_id}))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            ("200 OK", Value::Array(held).to_string())
        }
        ("DELETE", named) => {
            let name = named.trim_start_matches('/').to_owned();
            state.labels.retain(|(held, _)| *held != name);
            ("204 No Content", String::new())
        }
        _ => panic!("the fixture repository received an unknown call: {method} {path}"),
    }
}

/// What one whole session of the live journey costs, against this board rather than GitHub.
///
/// The credentialed target drives this same journey against GitHub with a credential; this
/// drives it against the fixture board above with none, and counts what it cost. Two numbers
/// come out of it and they are the two this crate can know offline about a whole session:
/// **how many requests the session makes**, and **the worst-case node count** of the
/// documents it sends. Neither is rate-limit points. What is pinned offline in points is a
/// **per-document** price, one document at a time, in `tests/point_cost.rs`; what a whole
/// session consumes of the hourly allowance is observed only by the accounting's own
/// per-budget figures, filled from the `x-ratelimit-*` headers a credentialed session's
/// responses carry, which the required check's live run prints.
///
/// The introspection batch stays under GitHub's cap, and stays complete.
///
/// Only a credentialed run meets that cap — the loopback board answers whatever it is
/// asked — and going over costs the whole request rather than part of the answer, so the
/// shape is held here. Complete matters as much: a batch that stayed under the cap by
/// dropping a type would verify less while looking cheaper. The cap itself comes from
/// [`journey::INTROSPECTION_FIELD_LIMIT`].
#[test]
fn no_introspection_document_selects_a_capped_field_more_often_than_github_allows() {
    let documents = journey::contract_schema_documents();
    for (index, document) in documents.iter().enumerate() {
        for capped in ["fields{", "inputFields{"] {
            let used = document.matches(capped).count();
            assert!(
                used <= journey::INTROSPECTION_FIELD_LIMIT,
                "introspection document {index} selects `{capped}` {used} times; GitHub refuses the whole request above {}:\n{document}",
                journey::INTROSPECTION_FIELD_LIMIT
            );
        }
    }

    let selected = documents.concat();
    for (type_name, input, _) in journey::MUTATION_TYPES {
        let selection = if input { "inputFields" } else { "fields" };
        let root = format!("{type_name}:__type(name:\"{type_name}\"){{{selection}");
        assert_eq!(
            selected.matches(&root).count(),
            1,
            "{type_name} must be asked about exactly once across the batch"
        );
    }
    assert_eq!(
        selected
            .matches("Mutation:__type(name:\"Mutation\"){fields")
            .count(),
        1,
        "and the Mutation root exactly once too"
    );
}

/// The two delete mutations the pinned schema carries are exactly what the credentialed lane
/// introspects from GitHub on every run, so the janitor's documents, which
/// `crates/onetaskgraph-live-janitor/tests/schema.rs` validates against that pinned copy, are
/// held to GitHub's own answer rather than to a copy nothing re-reads.
#[test]
fn the_pinned_delete_mutations_are_the_ones_the_live_lane_introspects() {
    use graphql_parser::schema::{Definition, TypeDefinition, parse_schema};
    let pinned = parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let fields = |name: &str| -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = pinned
            .definitions
            .iter()
            .find_map(|definition| match definition {
                Definition::TypeDefinition(TypeDefinition::Object(object))
                    if object.name == name =>
                {
                    Some(
                        object
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.field_type.to_string()))
                            .collect(),
                    )
                }
                Definition::TypeDefinition(TypeDefinition::InputObject(input))
                    if input.name == name =>
                {
                    Some(
                        input
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.value_type.to_string()))
                            .collect(),
                    )
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the pinned schema declares no {name}"));
        found.sort();
        found
    };
    let mutation = pinned
        .definitions
        .iter()
        .find_map(|definition| match definition {
            Definition::TypeDefinition(TypeDefinition::Object(object))
                if object.name == "Mutation" =>
            {
                Some(object)
            }
            _ => None,
        })
        .unwrap();
    for name in ["deleteIssue", "deleteProjectV2Item"] {
        let (_, input, payload) = journey::MUTATION_CONTRACT
            .iter()
            .find(|(mutation, _, _)| *mutation == name)
            .unwrap_or_else(|| panic!("the live lane does not introspect {name}"));
        let field = mutation.fields.iter().find(|f| f.name == name).unwrap();
        assert_eq!(field.field_type.to_string(), *payload, "{name}'s payload");
        assert_eq!(
            field
                .arguments
                .iter()
                .map(|a| (a.name.as_str(), a.value_type.to_string()))
                .collect::<Vec<_>>(),
            [("input", format!("{input}!"))],
            "{name}'s argument"
        );
        for type_name in [*input, *payload] {
            let mut live = journey::mutation_field_types(type_name)
                .iter()
                .map(|(field, kind)| ((*field).to_owned(), (*kind).to_owned()))
                .collect::<Vec<_>>();
            live.sort();
            assert_eq!(fields(type_name), live, "{type_name}");
        }
    }
}

/// The record beside it is a golden: a change to the journey or to what the source asks for
/// moves these numbers, and this fails naming both so the move is a decision rather than a
/// drift. `crates/onetaskgraph-github-projects/session-cost.md` is where the before and after
/// of the reduction this record came out of are written down.
#[tokio::test]
async fn a_whole_session_of_the_live_journey_costs_what_the_record_beside_it_says() {
    let fixture = board_with(vec![], true, false);
    // The board GitHub really is, so the journey below — the same body of code the
    // credentialed lane drives — can only converge through the union in
    // `GitHubProjectsSource::board`.
    fixture.items_connection_falls_behind();
    let labels = label_endpoints(&fixture.state);
    journey::against(journey::Endpoints {
        graphql: fixture.endpoint.clone(),
        rest_host: labels,
        // Pacing off: it spaces this source's own content-creating mutations in time and
        // changes no count, so paying for it here would only make the measurement slower.
        source: Some(json!({"endpoint":fixture.endpoint,
            "pacing":{"min_mutation_interval_ms":0,"retry_budget_ms":0}})),
    });
    journey::run(journey::Nomination {
        token: "test-token".to_owned(),
        owner: "octo-org".to_owned(),
        project_number: 7,
        repository: "acme/work".to_owned(),
        writer: match lane::admit(&|name| match name {
            "GH_PROJECTS_TOKEN" => Some("test-token".to_owned()),
            "GH_PROJECTS_OWNER" => Some("octo-org".to_owned()),
            "GH_PROJECTS_NUMBER" => Some("7".to_owned()),
            "GH_PROJECTS_REPOSITORY" => Some("acme/work".to_owned()),
            "GITHUB_ACTIONS" => Some("true".to_owned()),
            "GITHUB_RUN_ID" => Some("37616803489".to_owned()),
            "GITHUB_RUN_ATTEMPT" => Some("2".to_owned()),
            _ => None,
        })
        .unwrap()
        {
            lane::Admission::Run { writer, .. } => writer,
            lane::Admission::Skip(reason) => panic!("fixture lane skipped: {reason}"),
        },
    })
    .await;
    let sent = fixture.seen();
    let issues: Vec<_> = sent
        .iter()
        .filter(|call| call[0] == "createIssue")
        .collect();
    assert!(!issues.is_empty(), "the journey wrote its issues");
    for issue in issues {
        let title = issue[1]["title"].as_str().unwrap();
        let suffix = onetaskgraph_github_live::titled_stamp(
            title,
            onetaskgraph_github_projects::DESIGN_TITLE_PREFIX,
        )
        .unwrap();
        let stamp = onetaskgraph_live::artifact::CiStamp::read(suffix).unwrap();
        assert_eq!(stamp.run().run_id(), 37616803489);
        assert_eq!(stamp.run().attempt(), 2);
    }
    let label = sent
        .iter()
        .find(|call| call[0] == "createLiveLabel")
        .expect("journey created its label");
    let stamp = onetaskgraph_live::artifact::CiStamp::read(
        onetaskgraph_github_live::labelled_stamp(label[1]["name"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(stamp.run().run_id(), 37616803489);
    assert_eq!(stamp.run().attempt(), 2);
    let measured = session_cost(&journey::SESSION.snapshot());
    let recorded = include_str!("fixtures/session-cost.txt");
    assert_eq!(
        measured.trim(),
        recorded.trim(),
        "this session no longer costs what tests/fixtures/session-cost.txt records; if the \
         change is deliberate, put the measurement above into that file and say in \
         session-cost.md what moved it"
    );
}

/// The session's cost in the two quantities this crate can count without a credential.
///
/// Per call, so a reader can see *where* a session spends rather than only how much: the
/// name is the document's own or the endpoint's template, never anything a board held.
fn session_cost(session: &Session) -> String {
    let mut calls: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for request in session.requests() {
        let entry = calls.entry(request.name()).or_default();
        entry.0 += 1;
        entry.1 += match request.call() {
            onetaskgraph_github_projects::accounting::Call::Document { node_count, .. } => {
                node_count.unwrap_or_default()
            }
            onetaskgraph_github_projects::accounting::Call::Endpoint { .. } => 0,
        };
    }
    let mut rendered = format!(
        "requests {}\nnode count {}\n\nrequests per call\n",
        session.total_requests(),
        session.total_node_count()
    );
    for (name, (requests, nodes)) in calls {
        rendered.push_str(&format!("  {requests:>3}  {nodes:>6}  {name}\n"));
    }
    rendered
}

/// A board holding every kind of thing an id handed to a comment verb might name.
///
/// A task under a project, a task nobody has commented on, another task with comments of its
/// own, the project itself, a document and a draft — so each case below can tell the item it
/// is about from every item it must not touch.
fn comment_board() -> Fixture {
    board(vec![
        Item::issue("I_plan", "the plan").sub_issues(1),
        Item::issue("I_task", "a step")
            .parent("I_plan")
            .status("Todo"),
        Item::issue("I_quiet", "a step nobody has commented on").status("Todo"),
        Item::issue("I_other", "another step").status("Todo"),
        design("I_doc", "the design"),
        Item::draft("DI_sketch", "a sketch").status("Todo"),
    ])
}

fn native(id: &str) -> NativeId {
    NativeId(id.to_owned())
}

fn comment_body(text: &str) -> CommentBody {
    CommentBody::new(text).expect("a non-empty comment body")
}

fn commenting(text: &str) -> NewComment {
    NewComment {
        body: comment_body(text),
        author: None,
    }
}

fn comment_ids(comments: &[Comment]) -> Vec<String> {
    comments
        .iter()
        .map(|comment| comment.id.0.clone())
        .collect()
}

#[tokio::test]
async fn a_tasks_comments_are_its_issues_own_oldest_first_walked_in_pages_to_exhaustion() {
    let fixture = comment_board();
    let first = fixture.commented("I_task", Some("octocat"), "Seen on main.\n");
    let elsewhere = fixture.commented("I_other", Some("octocat"), "not this task's");
    let second = fixture.commented("I_task", None, "from an account since deleted");
    let third = fixture.commented("I_task", Some("hubot"), "fixed by #12");
    let source = source(&fixture);
    let task = native("I_task");

    let opening = source
        .task_comments(&task, &page(2))
        .await
        .unwrap()
        .expect("a task this board holds");
    assert_eq!(comment_ids(&opening.items), [first.clone(), second.clone()]);
    let cursor = opening
        .next
        .clone()
        .expect("a third comment is still to come");
    let rest = source
        .task_comments(&task, &resume(&cursor.0, 2))
        .await
        .unwrap()
        .expect("the same task");
    assert_eq!(comment_ids(&rest.items), [third]);
    assert!(rest.next.is_none(), "the walk ends at the last comment");
    assert!(
        !comment_ids(&opening.items).contains(&elsewhere)
            && !comment_ids(&rest.items).contains(&elsewhere),
        "another issue's comment was reported on this task"
    );
    // One request per page, the read of the issue included: the caller's limit is the page
    // GitHub is asked for, rather than every comment read and then cut.
    assert_eq!(fixture.requests("issueDetail"), 2);
    assert_eq!(fixture.operations(), ["issueDetail", "issueDetail"]);

    // Every member is read off what GitHub said, the author's login and a deleted account's
    // `null` alike, and the body keeps its trailing newline.
    let written = &opening.items[0];
    assert_eq!(written.author.as_deref(), Some("octocat"));
    assert_eq!(written.body, "Seen on main.\n");
    assert_eq!(
        written.created_at,
        Some("2026-09-01T00:00:01Z".parse().unwrap())
    );
    assert_eq!(written.updated_at, written.created_at);
    assert_eq!(
        written.url.as_deref(),
        Some(format!("https://github.example/I_task#{first}").as_str())
    );
    assert_eq!(opening.items[1].author, None);
    assert_eq!(rest.items[0].author.as_deref(), Some("hubot"));

    // A task nobody has commented on is one empty last page: neither no such task nor a
    // refusal.
    let quiet = source
        .task_comments(&native("I_quiet"), &page(50))
        .await
        .unwrap()
        .expect("a task this board holds");
    assert!(quiet.items.is_empty() && quiet.next.is_none());
}

#[tokio::test]
async fn adding_a_comment_keeps_its_body_byte_for_byte_and_reports_the_account_github_signed_it_with()
 {
    let fixture = comment_board();
    let source = source(&fixture);
    let text = "Seen again on main.\n\n```\npanicked at src/lib.rs:1\n```\n";

    let added = source
        .add_comment(&native("I_task"), &commenting(text))
        .await
        .unwrap()
        .expect("a task this board holds");
    assert_eq!(added.body, text);
    assert_eq!(added.author.as_deref(), Some(COMMENTER));
    assert!(added.created_at.is_some());
    assert_eq!(added.updated_at, added.created_at);
    assert!(
        added
            .url
            .as_deref()
            .is_some_and(|url| url.contains(&added.id.0)),
        "{added:?}"
    );

    // What GitHub was sent is the task's own issue and the body exactly, and what it now
    // holds is that one comment.
    assert!(
        fixture.seen().iter().any(|call| call[0] == "addComment"
            && call[1]["subjectId"] == "I_task"
            && call[1]["body"] == text),
        "{:?}",
        fixture.seen()
    );
    let held = fixture.comments_on("I_task");
    assert_eq!(held.len(), 1);
    assert_eq!(
        (held[0].id.as_str(), held[0].body.as_str()),
        (added.id.0.as_str(), text)
    );

    // And a read of the task reports it exactly as the write answered it.
    let listed = source
        .task_comments(&native("I_task"), &page(50))
        .await
        .unwrap()
        .expect("a task this board holds");
    assert_eq!(listed.items, vec![added]);
}

#[tokio::test]
async fn editing_a_comment_moves_its_body_and_when_it_last_changed_and_nothing_else() {
    let fixture = comment_board();
    let id = fixture.commented("I_task", Some("octocat"), "first thoughts");
    let source = source(&fixture);
    let before = source
        .task_comments(&native("I_task"), &page(50))
        .await
        .unwrap()
        .expect("a task this board holds")
        .items
        .remove(0);

    let edited = source
        .edit_comment(
            &native("I_task"),
            &native(&id),
            &comment_body("second thoughts\n"),
        )
        .await
        .unwrap()
        .expect("a comment of that task");
    assert_eq!(edited.body, "second thoughts\n");
    assert!(
        edited.updated_at > before.updated_at,
        "{edited:?} against {before:?}"
    );
    assert_eq!(
        Comment {
            body: before.body.clone(),
            updated_at: before.updated_at,
            ..edited.clone()
        },
        before,
        "an edit moved something other than the body and when it last changed"
    );
    assert_eq!(fixture.comments_on("I_task")[0].body, "second thoughts\n");

    // Which issue the comment is on was read before the edit was sent, not after.
    let documents = fixture.documents();
    let asked = documents
        .iter()
        .position(|document| document == graphql::COMMENT_ISSUE)
        .expect("the comment's issue was read");
    let changed = documents
        .iter()
        .position(|document| document == graphql::UPDATE_COMMENT)
        .expect("the edit was sent");
    assert!(
        asked < changed,
        "the edit was sent before its issue was read"
    );
}

#[tokio::test]
async fn deleting_a_comment_removes_that_comment_alone_and_answers_its_id() {
    let fixture = comment_board();
    let kept = fixture.commented("I_task", Some("octocat"), "keep me");
    let gone = fixture.commented("I_task", Some("octocat"), "remove me");
    let source = source(&fixture);

    let removed = source
        .delete_comment(&native("I_task"), &native(&gone))
        .await
        .unwrap();
    assert_eq!(removed, Some(native(&gone)));
    assert_eq!(
        fixture
            .comments_on("I_task")
            .iter()
            .map(|held| held.id.clone())
            .collect::<Vec<_>>(),
        [kept]
    );
    assert!(
        fixture
            .seen()
            .iter()
            .any(|call| call[0] == "deleteIssueComment" && call[1]["id"] == gone.as_str())
    );
}

#[tokio::test]
async fn a_comment_call_naming_no_task_of_this_board_answers_none_and_changes_nothing() {
    let fixture = comment_board();
    let on_plan = fixture.commented("I_plan", Some("octocat"), "on the project's issue");
    let on_doc = fixture.commented("I_doc", Some("octocat"), "on the document's issue");
    let source = source(&fixture);
    // A project and a document are issues with comments of their own on GitHub, and still no
    // task of this board — which is what `get_task` answers about them too.
    for (named, comment, what) in [
        ("I_missing", on_plan.as_str(), "an id naming nothing"),
        ("I_plan", on_plan.as_str(), "a project"),
        ("I_doc", on_doc.as_str(), "a document"),
    ] {
        let task = native(named);
        assert!(source.get_task(&task).await.unwrap().is_none(), "{what}");
        assert!(
            source
                .task_comments(&task, &page(50))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
        assert!(
            source
                .add_comment(&task, &commenting("hello"))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
        assert!(
            source
                .edit_comment(&task, &native(comment), &comment_body("changed"))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
        assert!(
            source
                .delete_comment(&task, &native(comment))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
    }
    for operation in [
        "issueComments",
        "comment",
        "addComment",
        "updateIssueComment",
        "deleteIssueComment",
    ] {
        assert_eq!(
            fixture.requests(operation),
            0,
            "{operation} was sent for no task"
        );
    }
    // The project and the document were answered from their reads above; the id naming
    // nothing was asked once, by the one read a listing is.
    assert_eq!(fixture.requests("issueDetail"), 1);
    assert_eq!(
        fixture.comments_on("I_plan")[0].body,
        "on the project's issue"
    );
    assert_eq!(
        fixture.comments_on("I_doc")[0].body,
        "on the document's issue"
    );
}

#[tokio::test]
async fn a_comment_that_is_not_on_this_task_answers_none_and_no_mutation_is_sent() {
    let fixture = comment_board();
    let theirs = fixture.commented("I_other", Some("octocat"), "on another issue");
    let source = source(&fixture);
    let task = native("I_task");
    for (comment, what) in [
        (theirs.as_str(), "a comment on another issue"),
        ("IC_nothing", "an id GitHub cannot resolve"),
        ("I_other", "an id naming an issue rather than a comment"),
    ] {
        assert!(
            source
                .edit_comment(&task, &native(comment), &comment_body("hijacked"))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
        assert!(
            source
                .delete_comment(&task, &native(comment))
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
    }
    assert_eq!(fixture.requests("updateIssueComment"), 0);
    assert_eq!(fixture.requests("deleteIssueComment"), 0);
    assert_eq!(
        fixture.requests("comment"),
        6,
        "each call asked which issue its comment is on before deciding"
    );
    let held = fixture.comments_on("I_other");
    assert_eq!((held.len(), held[0].body.as_str()), (1, "on another issue"));
}

#[tokio::test]
async fn every_comment_call_on_a_draft_item_is_refused_naming_what_to_do_instead() {
    let fixture = comment_board();
    let source = source(&fixture);
    let draft = native("DI_sketch");
    // A draft is a task of this board, so the refusal is about the draft rather than about
    // an id this board does not hold.
    assert!(source.get_task(&draft).await.unwrap().is_some());
    let outcomes = [
        source.task_comments(&draft, &page(50)).await.map(|_| ()),
        source
            .add_comment(&draft, &commenting("hello"))
            .await
            .map(|_| ()),
        source
            .edit_comment(&draft, &native("IC_1"), &comment_body("changed"))
            .await
            .map(|_| ()),
        source
            .delete_comment(&draft, &native("IC_1"))
            .await
            .map(|_| ()),
    ];
    for outcome in outcomes {
        let Err(SourceError::Refused { message }) = &outcome else {
            panic!("a comment call on a draft answered {outcome:?}");
        };
        assert!(
            message.contains("DI_sketch") && message.contains("draft"),
            "{message}"
        );
        assert!(
            message.contains("next: convert the draft to an issue"),
            "{message}"
        );
    }
    for operation in [
        "issueComments",
        "comment",
        "addComment",
        "updateIssueComment",
        "deleteIssueComment",
    ] {
        assert_eq!(
            fixture.requests(operation),
            0,
            "{operation} was sent for a draft"
        );
    }
}

#[tokio::test]
async fn a_comment_handed_an_author_is_refused_before_anything_is_sent() {
    let fixture = comment_board();
    let source = source(&fixture);
    let error = source
        .add_comment(
            &native("I_task"),
            &NewComment {
                body: comment_body("hello"),
                author: Some("Ada".to_owned()),
            },
        )
        .await
        .expect_err("GitHub signs every comment as the token's own account");
    let SourceError::Refused { message } = &error else {
        panic!("an author was answered with {error:?}");
    };
    assert!(message.contains("\"Ada\""), "{message}");
    assert!(
        message.contains("GitHub records the account the token signs in as the author"),
        "{message}"
    );
    assert!(message.contains("next: leave --author out"), "{message}");
    assert!(
        fixture.documents().is_empty(),
        "a request was sent for a comment already refused: {:?}",
        fixture.documents()
    );
    assert!(fixture.comments_on("I_task").is_empty());
}

#[tokio::test]
async fn a_graphql_error_on_any_comment_call_reaches_the_caller_as_the_refusal_github_sent() {
    for operation in [
        "issueDetail",
        "comment",
        "addComment",
        "updateIssueComment",
        "deleteIssueComment",
    ] {
        let fixture = comment_board();
        let id = fixture.commented("I_task", Some("octocat"), "already here");
        fixture.refuse(operation);
        let source = source(&fixture);
        let task = native("I_task");
        let outcome = match operation {
            "issueDetail" => source.task_comments(&task, &page(50)).await.map(|_| ()),
            "addComment" => source
                .add_comment(&task, &commenting("hello"))
                .await
                .map(|_| ()),
            "deleteIssueComment" => source.delete_comment(&task, &native(&id)).await.map(|_| ()),
            // `comment` is the read an edit makes first, so an edit reaches either refusal.
            _ => source
                .edit_comment(&task, &native(&id), &comment_body("changed"))
                .await
                .map(|_| ()),
        };
        let Err(SourceError::Refused { message }) = &outcome else {
            panic!("{operation} refused by GitHub answered {outcome:?}");
        };
        assert!(
            message.contains(&format!("{operation} is refused by this board")),
            "{message}"
        );
        assert_eq!(fixture.comments_on("I_task")[0].body, "already here");
        assert_eq!(fixture.comments_on("I_task").len(), 1);
    }
}

/// What one issue read answers for the task `id`, for a stand-in that answers in sequence.
fn issue_read(id: &str) -> Value {
    let asked = Asked {
        path: "issue",
        board_items: 3,
        stuck_cursor: None,
    };
    json!({"data":{"node":Item::issue(id, "a step").as_issue(&json!([]), asked)}})
}

/// One issue answered the way a read of it with its comments answers, carrying `comments`.
fn issue_with_comments(id: &str, comments: Value) -> Value {
    let mut read = issue_read(id);
    read["data"]["node"]["comments"] = comments;
    read
}

/// Which comment verb a malformed-answer case drives.
enum CommentCall {
    List,
    ListFrom(&'static str),
    Add,
    Edit,
    Delete,
}

#[tokio::test]
async fn a_comment_answer_this_source_cannot_read_is_refused_as_malformed_rather_than_guessed_at() {
    let comment = |id: &str| {
        json!({"id":id,"author":{"login":"octocat"},"createdAt":"2026-09-01T00:00:01Z",
               "updatedAt":"2026-09-01T00:00:01Z","body":"said","url":null})
    };
    let owned = json!({"data":{"node":{"__typename":"IssueComment","id":"IC_1",
                                       "issue":{"id":"I_1"}}}});
    let cases: Vec<(Vec<Value>, CommentCall, &str)> = vec![
        // A listing is one read of the issue with its comments, so each of these is the issue
        // answered with a comment connection this source cannot read.
        (
            vec![issue_read("I_1")],
            CommentCall::List,
            "answered with no comments connection",
        ),
        (
            vec![issue_with_comments(
                "I_1",
                json!({"nodes":[{"id":"IC_1","body":"said","createdAt":"yesterday"}],
                       "pageInfo":{"hasNextPage":false}}),
            )],
            CommentCall::List,
            "createdAt is not a timestamp",
        ),
        (
            vec![issue_with_comments(
                "I_1",
                json!({"nodes":[comment("IC_1")],
                       "pageInfo":{"hasNextPage":true,"endCursor":"C1"}}),
            )],
            CommentCall::ListFrom("C1"),
            "cursor is empty or did not advance",
        ),
        (
            vec![
                issue_read("I_1"),
                json!({"data":{"addComment":{"commentEdge":{"node":comment("IC_1")}}}}),
            ],
            CommentCall::Add,
            "comment addition returned no subject",
        ),
        (
            vec![
                issue_read("I_1"),
                json!({"data":{"addComment":{"subject":{"id":"I_else"},
                                             "commentEdge":{"node":comment("IC_1")}}}}),
            ],
            CommentCall::Add,
            "comment addition answered about another issue",
        ),
        (
            vec![
                issue_read("I_1"),
                json!({"data":{"addComment":{"subject":{"id":"I_1"},"commentEdge":{"node":null}}}}),
            ],
            CommentCall::Add,
            "comment addition returned no comment",
        ),
        (
            vec![
                issue_read("I_1"),
                json!({"data":{"node":{"__typename":"IssueComment","id":"IC_1"}}}),
            ],
            CommentCall::Edit,
            "issue comment IC_1 names no issue",
        ),
        (
            vec![
                issue_read("I_1"),
                owned.clone(),
                json!({"data":{"updateIssueComment":{"issueComment":null}}}),
            ],
            CommentCall::Edit,
            "comment update returned no comment",
        ),
        (
            vec![
                issue_read("I_1"),
                owned.clone(),
                json!({"data":{"updateIssueComment":{"issueComment":comment("IC_else")}}}),
            ],
            CommentCall::Edit,
            "comment update returned the wrong comment",
        ),
        (
            vec![
                issue_read("I_1"),
                owned.clone(),
                json!({"data":{"deleteIssueComment":null}}),
            ],
            CommentCall::Delete,
            "comment deletion returned no payload",
        ),
    ];
    for (bodies, call, expected) in cases {
        let source = configured(&sequence_server(bodies), json!({}));
        let task = native("I_1");
        let outcome = match call {
            CommentCall::List => source.task_comments(&task, &page(50)).await.map(|_| ()),
            CommentCall::ListFrom(cursor) => source
                .task_comments(&task, &resume(cursor, 50))
                .await
                .map(|_| ()),
            CommentCall::Add => source
                .add_comment(&task, &commenting("hello"))
                .await
                .map(|_| ()),
            CommentCall::Edit => source
                .edit_comment(&task, &native("IC_1"), &comment_body("changed"))
                .await
                .map(|_| ()),
            CommentCall::Delete => source
                .delete_comment(&task, &native("IC_1"))
                .await
                .map(|_| ()),
        };
        let Err(SourceError::Malformed { message }) = &outcome else {
            panic!("expected a malformed answer naming {expected:?}, got {outcome:?}");
        };
        assert!(
            message.contains(expected),
            "expected {expected} in {message}"
        );
    }

    // An issue that is not there when its comments are read, and a comment whose node is
    // gone by the time its issue is read, are no such task and no such comment.
    let gone = configured(
        &sequence_server(vec![json!({"data":{"node":null}})]),
        json!({}),
    );
    assert!(
        gone.task_comments(&native("I_1"), &page(50))
            .await
            .unwrap()
            .is_none()
    );
    let gone = configured(
        &sequence_server(vec![issue_read("I_1"), json!({"data":{"node":null}})]),
        json!({}),
    );
    assert!(
        gone.edit_comment(&native("I_1"), &native("IC_1"), &comment_body("changed"))
            .await
            .unwrap()
            .is_none()
    );
    let zero = configured(&sequence_server(vec![]), json!({}));
    assert!(matches!(
        zero.task_comments(&native("I_1"), &page(0)).await,
        Err(SourceError::Config { .. })
    ));
}

#[tokio::test]
async fn comment_calls_are_paced_waited_out_and_named_like_every_other_call() {
    // A limiter catching any comment call names that call, rather than whatever came first.
    for (operation, doing) in [
        ("issueDetail", "reading one issue with its comments"),
        ("comment", "reading which issue a comment is on"),
        ("addComment", "adding a comment"),
        ("updateIssueComment", "editing a comment"),
        ("deleteIssueComment", "deleting a comment"),
    ] {
        let fixture = comment_board();
        let id = fixture.commented("I_task", Some("octocat"), "already here");
        fixture.script_for(operation, vec![Refusal::secondary_forbidden()]);
        let source = paced(&fixture.endpoint, no_waiting());
        let task = native("I_task");
        let outcome = match operation {
            "issueDetail" => source.task_comments(&task, &page(50)).await.map(|_| ()),
            "addComment" => source
                .add_comment(&task, &commenting("hello"))
                .await
                .map(|_| ()),
            "deleteIssueComment" => source.delete_comment(&task, &native(&id)).await.map(|_| ()),
            _ => source
                .edit_comment(&task, &native(&id), &comment_body("changed"))
                .await
                .map(|_| ()),
        };
        let Err(SourceError::RateLimited {
            message: Some(said),
            ..
        }) = &outcome
        else {
            panic!("{operation} refused for a rate limit answered {outcome:?}");
        };
        assert!(said.contains(doing), "{said:?} does not name {doing:?}");
    }

    // A refused comment is waited out and sent again, and lands once: a request GitHub
    // refused for a rate limit did not run.
    let fixture = comment_board();
    fixture.script_for("addComment", vec![Refusal::secondary_forbidden().after(0)]);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":0,"retry_backoff_ms":1,"retry_budget_ms":1000}),
    );
    source
        .add_comment(&native("I_task"), &commenting("once"))
        .await
        .unwrap()
        .expect("a task this board holds");
    assert_eq!(fixture.requests("addComment"), 2);
    assert_eq!(fixture.comments_on("I_task").len(), 1);

    // And the three comment mutations leave this source no faster than its interval, which
    // is measured from each one's completion like every other mutation's.
    let fixture = comment_board();
    let interval = Duration::from_millis(150);
    let source = paced(
        &fixture.endpoint,
        json!({"min_mutation_interval_ms":150,"retry_budget_ms":0}),
    );
    let task = native("I_task");
    let added = source
        .add_comment(&task, &commenting("paced"))
        .await
        .unwrap()
        .expect("a task this board holds");
    source
        .edit_comment(&task, &added.id, &comment_body("still paced"))
        .await
        .unwrap()
        .expect("the comment just added");
    source
        .delete_comment(&task, &added.id)
        .await
        .unwrap()
        .expect("the comment just edited");
    let gaps = fixture.mutation_gaps();
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    assert!(gaps.iter().all(|gap| *gap >= interval), "{gaps:?}");
}

/// A settlement-shaped update: a status, three keys set and one removed.
fn settlement(category: StatusCategory, name: &str) -> TaskUpdate {
    TaskUpdate {
        status: Some(status(category, name)),
        metadata_set: BTreeMap::from([
            (key("team.settlement"), json!({"outcome": "landed"})),
            (key("team.landing"), json!("merged")),
            (key("team.change"), json!("https://example.test/pull/1")),
        ]),
        metadata_remove: BTreeSet::from([key("team.claim")]),
        ..TaskUpdate::default()
    }
}

/// One task holding a slot of its own, a label and a status, on a board of three.
fn update_board(item: Item) -> Fixture {
    board(vec![
        item,
        Item::issue("I_2", "a blocker").status("Todo"),
        Item::issue("I_3", "another blocker").status("Todo"),
    ])
}

fn settled_task() -> Item {
    Item::issue("I_1", "a task")
        .body(
            "The prose.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.delivers\":[\"I_9\"],\
             \"onetaskgraph.item_kind\":\"task\",\"team.claim\":\"r-1\",\"team.kept\":[1]}\n-->",
        )
        .labelled(&[("L_1", "bug")])
        .status("Todo")
}

/// Every request the board answered, by the name `operation_name` reads off it.
fn every_request(fixture: &Fixture) -> Vec<String> {
    fixture
        .documents()
        .iter()
        .map(|document| operation_name(document).to_owned())
        .collect()
}

#[tokio::test]
async fn a_targeted_update_is_one_read_one_body_update_and_one_status_write() {
    let fixture = update_board(settled_task());
    let before = source_of(&fixture)
        .get_task(&id("I_1"))
        .await
        .unwrap()
        .unwrap();
    let requests_before = fixture.documents().len();
    let source = source(&fixture);
    let outcome = source
        .update_task(
            &id("I_1"),
            &settlement(StatusCategory::InProgress, "in-progress"),
        )
        .await
        .expect("the update lands")
        .expect("a task of this board");

    let body = "The prose.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.delivers\":[\"I_9\"],\
                \"onetaskgraph.item_kind\":\"task\",\"team.change\":\"https://example.test/pull/1\",\
                \"team.kept\":[1],\"team.landing\":\"merged\",\
                \"team.settlement\":{\"outcome\":\"landed\"}}\n-->";
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board","itemId":"PVTI_I_1",
                   "fieldId":"FIELD_status","value":{"singleSelectOptionId":"OPT_doing"}}]),
            json!(["updateIssue", {"id":"I_1","body":body}]),
        ],
        "the option, then the slot and the visible body in one update, last; no origin, no \
         title, no state, no dependency request"
    );
    assert_eq!(
        every_request(&fixture)[requests_before..],
        ["issue", "updateProjectV2ItemFieldValue", "updateIssue"],
        "one read of the item and nothing else read"
    );
    assert_eq!(
        outcome.written,
        BTreeSet::from([UpdatedField::Status, UpdatedField::Metadata])
    );
    assert_eq!(outcome.delivers_before, refs(&["I_9"]));

    let mut expected = before;
    expected.status = status(StatusCategory::InProgress, "In Progress");
    expected.metadata.remove("team.claim");
    for (key, value) in [
        ("team.settlement", json!({"outcome": "landed"})),
        ("team.landing", json!("merged")),
        ("team.change", json!("https://example.test/pull/1")),
    ] {
        expected.metadata.insert(key.to_owned(), value);
    }
    assert_eq!(outcome.task, expected, "every field not named is as it was");
    assert_eq!(
        source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .unwrap(),
        expected,
        "the answer is what a fresh read of the task reports"
    );
}

#[tokio::test]
async fn an_item_holding_no_status_costs_its_update_no_read_of_the_boards_fields() {
    // GitHub leaves an empty single-select out of an item's field values, so an item in no
    // Status column does not say which options the board has from its values. Its read by
    // its own id carries the board's field definitions beside them, so the update reads
    // nothing more: not the board's fields, not its items — and it sends only the status
    // write.
    let fixture = update_board(Item::issue("I_1", "a task").body("The prose."));
    let requests_before = fixture.documents().len();
    let outcome = source(&fixture)
        .update_task(
            &id("I_1"),
            &TaskUpdate {
                status: Some(status(StatusCategory::InProgress, "in-progress")),
                ..TaskUpdate::default()
            },
        )
        .await
        .expect("the update lands")
        .expect("a task of this board");
    assert_eq!(
        every_request(&fixture)[requests_before..],
        ["issue", "updateProjectV2ItemFieldValue"],
        "one read of the item, and the status write"
    );
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
    assert_eq!(outcome.written, BTreeSet::from([UpdatedField::Status]));
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
}

#[tokio::test]
async fn a_terminal_status_selects_its_option_then_closes_in_the_one_body_update() {
    let fixture = update_board(settled_task());
    let source = source(&fixture);
    let outcome = source
        .update_task(&id("I_1"), &settlement(StatusCategory::Done, "done"))
        .await
        .expect("the update lands")
        .expect("a task of this board");
    let seen = fixture.seen();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    assert_eq!(
        seen[0],
        json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board","itemId":"PVTI_I_1",
               "fieldId":"FIELD_status","value":{"singleSelectOptionId":"OPT_done"}}])
    );
    assert_eq!(seen[1][0], "updateIssue");
    assert_eq!(
        seen[1][1]["stateInput"],
        json!({"value":"CLOSED","stateReason":"COMPLETED"})
    );
    assert!(seen[1][1]["body"].is_string(), "{seen:#?}");
    assert!(seen[1][1].get("title").is_none(), "{seen:#?}");
    assert_eq!(outcome.task.status, status(StatusCategory::Done, "Done"));
    assert_eq!(fixture.item("I_1").state, "CLOSED");
}

#[tokio::test]
async fn an_open_status_selects_its_option_then_reopens_and_retitles_in_one_update() {
    let fixture = update_board(settled_task().status("Done").closed(Some("COMPLETED")));
    let source = source(&fixture);
    let update = TaskUpdate {
        title: Some("a task, again".to_owned()),
        status: Some(status(StatusCategory::Todo, "todo")),
        ..TaskUpdate::default()
    };
    let outcome = source
        .update_task(&id("I_1"), &update)
        .await
        .expect("the update lands")
        .expect("a task of this board");
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["updateProjectV2ItemFieldValue", {"projectId":"PVT_board","itemId":"PVTI_I_1",
                   "fieldId":"FIELD_status","value":{"singleSelectOptionId":"OPT_todo"}}]),
            json!(["updateIssue", {"id":"I_1","title":"a task, again",
                   "stateInput":{"value":"OPEN"}}]),
        ]
    );
    assert_eq!(
        outcome.written,
        BTreeSet::from([UpdatedField::Title, UpdatedField::Status])
    );
    assert_eq!(fixture.item("I_1").state, "OPEN");
}

#[tokio::test]
async fn an_update_naming_only_what_the_item_holds_sends_one_read_and_no_mutation() {
    let fixture = update_board(settled_task());
    let before = source_of(&fixture)
        .get_task(&id("I_1"))
        .await
        .unwrap()
        .unwrap();
    let requests_before = fixture.documents().len();
    let update = TaskUpdate {
        title: Some("a task".to_owned()),
        content: Some("The prose.".to_owned()),
        status: Some(status(StatusCategory::Todo, "todo")),
        metadata_set: BTreeMap::from([(key("team.kept"), json!([1]))]),
        metadata_remove: BTreeSet::from([key("team.absent")]),
        delivers: Some(refs(&["I_9"])),
        ..TaskUpdate::default()
    };
    let outcome = source(&fixture)
        .update_task(&id("I_1"), &update)
        .await
        .expect("an answer")
        .expect("a task of this board");
    assert!(fixture.seen().is_empty(), "{:#?}", fixture.seen());
    assert_eq!(every_request(&fixture)[requests_before..], ["issue"]);
    assert!(outcome.written.is_empty(), "{:?}", outcome.written);
    assert_eq!(outcome.task, before);
}

#[tokio::test]
async fn a_status_named_by_a_word_of_its_own_lands_on_the_mapped_option() {
    let fixture = update_board(settled_task());
    let update = TaskUpdate {
        status: Some(status(StatusCategory::Cancelled, "failed")),
        ..TaskUpdate::default()
    };
    let outcome = source(&fixture)
        .update_task(&id("I_1"), &update)
        .await
        .expect("the update lands")
        .expect("a task of this board");
    assert_eq!(
        outcome.task.status,
        status(StatusCategory::Cancelled, "Cancelled")
    );
    assert_eq!(
        source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .unwrap()
            .status,
        status(StatusCategory::Cancelled, "Cancelled")
    );
}

#[tokio::test]
async fn named_edges_send_only_the_blocked_by_difference() {
    let fixture = update_board(settled_task());
    fixture
        .state
        .lock()
        .unwrap()
        .blocked_by
        .insert("I_1".to_owned(), vec!["I_2".to_owned()]);
    let update = TaskUpdate {
        depends_on: Some(vec![DependencyEdge {
            from: DependencyEndpoint::from_native(id("I_1"), ItemKind::Task),
            to: DependencyEndpoint::from_native(id("I_3"), ItemKind::Task),
            kind: DependencyKind::Blocks,
        }]),
        ..TaskUpdate::default()
    };
    let outcome = source(&fixture)
        .update_task(&id("I_1"), &update)
        .await
        .expect("the update lands")
        .expect("a task of this board");
    assert_eq!(
        fixture.seen(),
        vec![
            json!(["removeBlockedBy", {"issueId":"I_1","blockingIssueId":"I_2"}]),
            json!(["addBlockedBy", {"issueId":"I_1","blockingIssueId":"I_3"}]),
        ],
        "no body, title, state or field request for an edge change"
    );
    assert_eq!(outcome.written, BTreeSet::from([UpdatedField::DependsOn]));

    // The same set again sends nothing.
    let again = source(&fixture)
        .update_task(&id("I_1"), &update)
        .await
        .expect("an answer")
        .expect("a task of this board");
    assert!(again.written.is_empty());
    assert_eq!(fixture.seen().len(), 2);
}

#[tokio::test]
async fn a_contradictory_update_or_an_absent_task_writes_nothing() {
    let fixture = update_board(settled_task());
    let contradiction = TaskUpdate {
        metadata_set: BTreeMap::from([(key("team.claim"), json!("r-2"))]),
        metadata_remove: BTreeSet::from([key("team.claim")]),
        ..TaskUpdate::default()
    };
    let error = source(&fixture)
        .update_task(&id("I_1"), &contradiction)
        .await
        .expect_err("a contradiction");
    assert!(matches!(error, SourceError::Refused { .. }), "{error:?}");
    assert!(
        fixture.documents().is_empty(),
        "refused before anything is read"
    );
    let absent = source(&fixture)
        .update_task(&id("I_404"), &settlement(StatusCategory::Done, "done"))
        .await
        .expect("an answer");
    assert_eq!(absent, None);
    assert!(fixture.seen().is_empty());
}

#[tokio::test]
async fn an_update_this_board_cannot_carry_is_refused_by_name_and_writes_nothing() {
    // A document's title and a priority on a board with no priority_mapping are refused
    // before the item is read. Closing a draft item, which has no open or closed state, is
    // refused after the one read that shows it is a draft, and sends no mutation.
    let fixture = update_board(settled_task());
    for (update, names) in [
        (
            TaskUpdate {
                title: Some(format!("{DESIGN_TITLE_PREFIX}a plan")),
                ..TaskUpdate::default()
            },
            "retitle it",
        ),
        (
            TaskUpdate {
                priority: Some(Priority::High),
                ..TaskUpdate::default()
            },
            "priority_mapping",
        ),
    ] {
        let error = source(&fixture)
            .update_task(&id("I_1"), &update)
            .await
            .expect_err("a field this board cannot carry");
        let SourceError::Refused { message } = &error else {
            panic!("answered as {error:?}");
        };
        assert!(message.contains(names), "{message}");
    }
    assert!(
        fixture.documents().is_empty(),
        "refused before anything is read"
    );

    let fixture = update_board(Item::draft("I_1", "a draft").status("Todo"));
    let error = source(&fixture)
        .update_task(&id("I_1"), &settlement(StatusCategory::Done, "done"))
        .await
        .expect_err("a draft has no closed state");
    let SourceError::Refused { message } = &error else {
        panic!("answered as {error:?}");
    };
    assert!(message.contains("draft items"), "{message}");
    assert!(fixture.seen().is_empty(), "{:#?}", fixture.seen());
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("Todo"));
}

#[tokio::test]
async fn a_slot_spelled_another_way_is_kept_byte_for_byte_by_an_update_that_changes_nothing() {
    // A person, or another tool, spelled this slot with its own whitespace. It holds exactly
    // what the update names, so nothing is sent — and when the update changes only the
    // status, the body is not touched either.
    let body = "The prose.\n\n<!-- onetaskgraph.metadata\n{ \"team.kept\" : [1], \
                \"onetaskgraph.item_kind\": \"task\" }\n-->";
    let fixture = update_board(Item::issue("I_1", "a task").body(body).status("Todo"));
    let unchanged = TaskUpdate {
        metadata_set: BTreeMap::from([(key("team.kept"), json!([1]))]),
        metadata_remove: BTreeSet::from([key("team.absent")]),
        content: Some("The prose.".to_owned()),
        ..TaskUpdate::default()
    };
    let outcome = source(&fixture)
        .update_task(&id("I_1"), &unchanged)
        .await
        .expect("an answer")
        .expect("a task of this board");
    assert!(outcome.written.is_empty(), "{:?}", outcome.written);
    assert!(fixture.seen().is_empty(), "{:#?}", fixture.seen());

    let moved = TaskUpdate {
        status: Some(status(StatusCategory::InProgress, "in-progress")),
        ..unchanged
    };
    source(&fixture)
        .update_task(&id("I_1"), &moved)
        .await
        .expect("the update lands")
        .expect("a task of this board");
    assert_eq!(
        fixture
            .seen()
            .iter()
            .map(|entry| entry[0].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>(),
        ["updateProjectV2ItemFieldValue"],
        "a status alone moved"
    );
    assert_eq!(fixture.item("I_1").body.as_deref(), Some(body));
}

#[tokio::test]
async fn a_slot_holding_steps_keeps_them_byte_for_byte_through_an_update_of_other_fields() {
    // This source keeps metadata as one compact JSON object, which escapes every line break,
    // so a value has no layout for an edit to move, and `-->` alone cannot close the slot.
    let task = "## What\nWork.\n\n## Acceptance criteria\n\n- x <!-- a note -->\n\n";
    let steps = json!([
        {"id": "build", "persona": "engineer", "task": task},
        {"id": "check", "persona": "reviewer", "deps": ["build"], "task": task},
    ]);
    let slot = BTreeMap::from([
        ("onetaskgraph.item_kind".to_owned(), json!("task")),
        ("onepipeline.steps".to_owned(), steps.clone()),
        ("team.claim".to_owned(), json!("r-1")),
    ]);
    let encoded_steps = serde_json::to_string(&steps).unwrap();
    let body = format!(
        "The prose.\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        serde_json::to_string(&slot).unwrap()
    );
    let updates = [
        TaskUpdate {
            metadata_set: BTreeMap::from([(key("onepipeline.node"), json!("x"))]),
            ..TaskUpdate::default()
        },
        TaskUpdate {
            metadata_remove: BTreeSet::from([key("team.claim")]),
            ..TaskUpdate::default()
        },
        TaskUpdate {
            title: Some("renamed".to_owned()),
            content: Some("Other prose.\n".to_owned()),
            ..TaskUpdate::default()
        },
        TaskUpdate {
            status: Some(status(StatusCategory::Done, "done")),
            ..TaskUpdate::default()
        },
    ];
    for update in updates {
        let fixture = update_board(Item::issue("I_1", "a task").body(&body).status("Todo"));
        let before = source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.metadata["onepipeline.steps"], steps);
        let outcome = source(&fixture)
            .update_task(&id("I_1"), &update)
            .await
            .unwrap_or_else(|error| panic!("{update:?}: refused: {error:?}"))
            .expect("a task of this board");
        assert!(!outcome.written.is_empty(), "{update:?}");
        let written = fixture.item("I_1").body.clone().unwrap_or_default();
        assert!(
            written.contains(&format!("\"onepipeline.steps\":{encoded_steps}")),
            "{update:?}: the steps are the bytes they were: {written}"
        );
        let after = source_of(&fixture)
            .get_task(&id("I_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, outcome.task, "the answer is the read");
        assert_eq!(after.metadata["onepipeline.steps"], steps, "{update:?}");
        let mut wanted = update.applied_to(&before);
        // The board reads a status back as its option's name.
        if update.status.is_some() {
            assert_eq!(after.status.category, StatusCategory::Done);
            wanted.status = after.status.clone();
        }
        assert_eq!(after, wanted, "{update:?}: nothing unnamed moved");
    }
}

/// The three endings a content write must keep, on content with interior structure a trim
/// would also leave alone.
const CONTENT_ENDINGS: [&str; 3] = [
    "# Goal\n\n- two\n  lines",
    "# Goal\n\n- two\n  lines\n",
    "# Goal\n\n- two\n  lines\n\n",
];

/// What the board stores for `content_id` before its metadata slot: the issue body less the
/// slot and the one separator the source puts in front of it.
fn stored_content(fixture: &Fixture, content_id: &str) -> String {
    let body = fixture.item(content_id).body.unwrap_or_default();
    body.rfind("\n\n<!-- onetaskgraph.metadata\n")
        .map_or(body.clone(), |at| body[..at].to_owned())
}

/// The entry a rendering write carries. A plugin stores it as it stores any metadata value and
/// never reads its shape, which is the engine's, so an opaque value is all a write here needs.
fn rendered_provenance() -> Value {
    json!({"rendered": "by this test"})
}

#[tokio::test]
async fn every_task_content_write_stores_the_bytes_exactly_and_reads_them_back() {
    let answers = BTreeMap::from([("goal".to_owned(), json!("Ship it"))]);
    for content in CONTENT_ENDINGS {
        let fixture = board(vec![]);
        let source = source(&fixture);
        let mut plain = task("T-1", "Plain", status(StatusCategory::Todo, "Todo"));
        plain.content = Some(content.to_owned());
        let mut rendered = plain.clone();
        rendered.title = "Rendered".to_owned();
        rendered.metadata =
            BTreeMap::from([(MetadataKey::TEMPLATE_KEY.to_owned(), rendered_provenance())]);
        let held = |id: NativeId| {
            let source = &source;
            async move { source.get_task(&id).await.unwrap().unwrap().content }
        };

        let created = source.write_task(&write(plain.clone())).await.unwrap();
        assert_eq!(stored_content(&fixture, &created.0), content, "create");
        assert_eq!(held(created.clone()).await.as_deref(), Some(content));
        let from_template = source
            .write_task_rendered(&write(rendered.clone()), &answers)
            .await
            .unwrap();
        assert_eq!(
            stored_content(&fixture, &from_template.0),
            content,
            "rendered create"
        );
        assert_eq!(held(from_template.clone()).await.as_deref(), Some(content));

        for again in CONTENT_ENDINGS {
            // The write a copy makes over the item it made before.
            plain.content = Some(again.to_owned());
            source
                .write_task(&ItemWrite {
                    target: Some(created.clone()),
                    item: plain.clone(),
                    depends_on: vec![],
                })
                .await
                .unwrap();
            assert_eq!(stored_content(&fixture, &created.0), again, "update");
            assert_eq!(held(created.clone()).await.as_deref(), Some(again));

            rendered.content = Some(again.to_owned());
            source
                .write_task_rendered(
                    &ItemWrite {
                        target: Some(from_template.clone()),
                        item: rendered.clone(),
                        depends_on: vec![],
                    },
                    &answers,
                )
                .await
                .unwrap();
            assert_eq!(
                stored_content(&fixture, &from_template.0),
                again,
                "rendered update"
            );
            assert_eq!(held(from_template.clone()).await.as_deref(), Some(again));

            source
                .set_task_rendering(&from_template, again, &rendered_provenance(), &answers)
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(
                stored_content(&fixture, &from_template.0),
                again,
                "rendering"
            );
            let read = source.get_task(&from_template).await.unwrap().unwrap();
            assert_eq!(read.content.as_deref(), Some(again));
            assert_eq!(
                read.metadata[MetadataKey::TEMPLATE_KEY],
                rendered_provenance()
            );

            source
                .set_task_content(&created, again)
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(stored_content(&fixture, &created.0), again, "content set");
            assert_eq!(held(created.clone()).await.as_deref(), Some(again));
        }
    }
}

#[tokio::test]
async fn every_document_and_project_content_write_stores_the_bytes_exactly() {
    let answers = BTreeMap::from([("goal".to_owned(), json!("Ship it"))]);
    for content in CONTENT_ENDINGS {
        let fixture = board(vec![]);
        let source = source(&fixture);
        let mut plain = document("D-1", "Design");
        plain.content = Some(content.to_owned());
        let mut rendered = plain.clone();
        rendered.metadata =
            BTreeMap::from([(MetadataKey::TEMPLATE_KEY.to_owned(), rendered_provenance())]);
        let mut launch = project("P-1", "Launch", status(StatusCategory::Todo, "Todo"));
        launch.content = Some(content.to_owned());
        let document_content = |id: NativeId| {
            let source = &source;
            async move { source.get_document(&id).await.unwrap().unwrap().content }
        };

        let created = source.write_document(&write(plain.clone())).await.unwrap();
        assert_eq!(stored_content(&fixture, &created.0), content, "create");
        assert_eq!(
            document_content(created.clone()).await.as_deref(),
            Some(content)
        );
        let from_template = source
            .write_document_rendered(&write(rendered.clone()), &answers)
            .await
            .unwrap();
        assert_eq!(
            stored_content(&fixture, &from_template.0),
            content,
            "rendered create"
        );
        assert_eq!(
            document_content(from_template.clone()).await.as_deref(),
            Some(content)
        );
        let project_id = source.write_project(&write(launch.clone())).await.unwrap();
        assert_eq!(stored_content(&fixture, &project_id.0), content, "project");

        for again in CONTENT_ENDINGS {
            plain.content = Some(again.to_owned());
            source
                .write_document(&ItemWrite {
                    target: Some(created.clone()),
                    item: plain.clone(),
                    depends_on: vec![],
                })
                .await
                .unwrap();
            assert_eq!(stored_content(&fixture, &created.0), again, "update");
            assert_eq!(
                document_content(created.clone()).await.as_deref(),
                Some(again)
            );

            rendered.content = Some(again.to_owned());
            source
                .write_document_rendered(
                    &ItemWrite {
                        target: Some(from_template.clone()),
                        item: rendered.clone(),
                        depends_on: vec![],
                    },
                    &answers,
                )
                .await
                .unwrap();
            assert_eq!(
                stored_content(&fixture, &from_template.0),
                again,
                "rendered update"
            );
            source
                .set_document_rendering(&from_template, again, &rendered_provenance(), &answers)
                .await
                .unwrap()
                .expect("the document");
            assert_eq!(
                stored_content(&fixture, &from_template.0),
                again,
                "rendering"
            );
            assert_eq!(
                document_content(from_template.clone()).await.as_deref(),
                Some(again)
            );

            launch.content = Some(again.to_owned());
            source
                .write_project(&ItemWrite {
                    target: Some(project_id.clone()),
                    item: launch.clone(),
                    depends_on: vec![],
                })
                .await
                .unwrap();
            assert_eq!(
                stored_content(&fixture, &project_id.0),
                again,
                "project update"
            );
            assert_eq!(
                source
                    .get_project(&project_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .content
                    .as_deref(),
                Some(again)
            );
        }
    }
}

/// The instant the comment-activity cases below ask about.
const COMMENTED_SINCE: &str = "2026-09-20T12:00:00Z";

/// Long before [`COMMENTED_SINCE`]: when every issue below was created and first commented on.
const LONG_BEFORE: &str = "2026-06-01T09:00:00Z";

/// A board whose items live in two owners' repositories, and one issue that is not on it.
///
/// - `I_fresh` — commented on after the instant, in the board owner's repository.
/// - `I_foreign` — in **another owner's** repository; created and commented on long before
///   the instant, and its only activity since is an edit to one of those old comments, which
///   moved the issue's `updatedAt` as GitHub does.
/// - `I_stale` — in the board owner's repository, every comment before the instant, and not
///   updated since.
/// - `I_retitled` — updated after the instant for another reason, with only old comments.
/// - `I_elsewhere` — an edited comment after the instant, on an issue this board does not
///   hold.
fn comment_activity_board() -> Fixture {
    let fixture = board(vec![
        Item::issue("I_fresh", "Fresh")
            .status("Todo")
            .updated("2026-09-21T09:00:00Z"),
        Item::issue("I_foreign", "Foreign")
            .status("In Progress")
            .in_repository("another-owner/elsewhere")
            .updated("2026-09-25T09:00:00Z"),
        Item::issue("I_stale", "Stale")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_retitled", "Retitled")
            .status("Todo")
            .updated("2026-09-26T09:00:00Z"),
        Item::issue("I_elsewhere", "Elsewhere")
            .only_on(&[3])
            .updated("2026-09-27T09:00:00Z"),
    ]);
    fixture.commented_at("I_fresh", "2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z");
    fixture.commented_at("I_foreign", LONG_BEFORE, LONG_BEFORE);
    fixture.commented_at("I_foreign", LONG_BEFORE, "2026-09-25T09:00:00Z");
    fixture.commented_at("I_stale", LONG_BEFORE, "2026-06-02T09:00:00Z");
    fixture.commented_at("I_retitled", LONG_BEFORE, LONG_BEFORE);
    fixture.commented_at("I_elsewhere", LONG_BEFORE, "2026-09-27T09:00:00Z");
    fixture
}

fn commented_since(statuses: Vec<StatusCategory>) -> TaskQuery {
    TaskQuery {
        statuses,
        commented_since: Some(COMMENTED_SINCE.parse().expect("an RFC 3339 instant")),
        ..TaskQuery::default()
    }
}

#[tokio::test]
async fn comment_activity_is_answered_by_the_board_scoped_search_and_each_candidates_comments() {
    let fixture = comment_activity_board();
    let source = source(&fixture);
    assert_eq!(
        source.capabilities().filter_by_comment_activity,
        Support::Native
    );

    assert_eq!(
        selected_tasks(source.as_ref(), &commented_since(Vec::new())).await,
        ["I_fresh", "I_foreign"],
        "the new comment and the cross-owner edited one; not the stale issue, not the one \
         updated for another reason, not the one on another board"
    );
    assert_eq!(
        fixture.searches(),
        ["project:octo-org/7 is:issue updated:>=2026-09-20T12:00:00+00:00"],
        "one search, scoped by the board alone, with the updated qualifier"
    );
    for search in fixture.searches() {
        for narrowing in ["repo:", "user:", "org:", "owner:"] {
            assert!(
                !search.contains(narrowing),
                "the search narrows by {narrowing}: {search}"
            );
        }
    }
    assert_eq!(
        fixture.board_item_reads(),
        Vec::<String>::new(),
        "the board's own item connection was read"
    );
    assert_eq!(
        fixture.comment_reads(),
        ["I_fresh", "I_foreign", "I_retitled"],
        "comments read for the search's candidates on this board, and for no other issue"
    );
}

#[tokio::test]
async fn comment_activity_combined_with_a_status_filter_is_the_intersection() {
    let fixture = comment_activity_board();
    let source = source(&fixture);
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &commented_since(vec![StatusCategory::InProgress])
        )
        .await,
        ["I_foreign"]
    );
    assert_eq!(
        fixture.comment_reads(),
        ["I_foreign"],
        "a candidate the status filter drops has no comments read"
    );
}

#[tokio::test]
async fn comment_activity_within_one_project_asks_that_project_and_reads_only_updated_candidates() {
    let fixture = board(vec![
        Item::issue("I_plan", "the plan").sub_issues(2),
        Item::issue("I_lively", "lively")
            .parent("I_plan")
            .status("Todo")
            .updated("2026-09-23T09:00:00Z"),
        Item::issue("I_quiet", "quiet")
            .parent("I_plan")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_loose", "elsewhere on the board")
            .status("Todo")
            .updated("2026-09-24T09:00:00Z"),
    ]);
    fixture.commented_at("I_lively", "2026-09-23T09:00:00Z", "2026-09-23T09:00:00Z");
    fixture.commented_at("I_quiet", LONG_BEFORE, "2026-06-02T09:00:00Z");
    fixture.commented_at("I_loose", "2026-09-24T09:00:00Z", "2026-09-24T09:00:00Z");
    let source = source(&fixture);

    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Is(NativeId("I_plan".to_owned())),
                ..commented_since(Vec::new())
            },
        )
        .await,
        ["I_lively"],
        "the project's own task commented on since, and no other project's or the board's"
    );
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
    assert_eq!(
        fixture.comment_reads(),
        ["I_lively"],
        "a task whose issue was not updated since has no comments read"
    );
}

/// The credentialed journey's failure, on a board whose issue search has not caught up with
/// any of the comment writes below: GitHub's index lags a write, and a comment-activity read
/// taken in that window used to rule out an issue this process had just commented on.
///
/// - `I_edited` — written by this source, with a comment from long before the instant that
///   this source then edits.
/// - `I_new` — written by this source, then newly commented on by it.
/// - `I_theirs` — never written by this source, newly commented on by it.
/// - `I_quiet` — nobody touches it.
///
/// What this source wrote leaves its own record of each item holding the `updatedAt` the item
/// had before, and the search still answers that same instant, so the only evidence of the
/// comment activity is that this process wrote it. Each read is asked twice — once by comment
/// activity alone and once narrowed by title too, as the journey asks it.
#[tokio::test]
async fn an_issue_this_source_commented_on_is_selected_before_the_search_index_catches_up() {
    let fixture = board(vec![
        Item::issue("I_edited", "widget edited")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_new", "widget new")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_theirs", "widget theirs")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_quiet", "widget quiet")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
    ]);
    let old = fixture.commented_at("I_edited", LONG_BEFORE, "2026-06-02T09:00:00Z");
    let source = source(&fixture);
    for id in ["I_edited", "I_new"] {
        source
            .write_task(&ItemWrite {
                target: Some(native(id)),
                item: task(
                    id,
                    &format!("widget {id}"),
                    status(StatusCategory::Todo, "Todo"),
                ),
                depends_on: vec![],
            })
            .await
            .unwrap();
    }
    for id in ["I_edited", "I_new", "I_theirs", "I_quiet"] {
        fixture.indexes_behind(id);
    }
    // Every comment write below is stamped by this board's clock, which reads 2026-09-01.
    let since: chrono::DateTime<chrono::Utc> = "2026-08-01T00:00:00Z".parse().unwrap();
    let commented_since = |text: Option<TextQuery>| TaskQuery {
        text,
        commented_since: Some(since),
        ..TaskQuery::default()
    };
    let by_title = || text("widget", TextFields::Title);
    for query in [commented_since(None), commented_since(by_title())] {
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            Vec::<String>::new(),
            "before any comment activity after the instant"
        );
    }

    source
        .edit_comment(&native("I_edited"), &native(&old), &comment_body("edited"))
        .await
        .unwrap()
        .expect("a comment of that task");
    source
        .add_comment(&native("I_new"), &commenting("new"))
        .await
        .unwrap()
        .expect("a task this board holds");
    source
        .add_comment(&native("I_theirs"), &commenting("new"))
        .await
        .unwrap()
        .expect("a task this board holds");

    for query in [commented_since(None), commented_since(by_title())] {
        let mut selected = selected_tasks(source.as_ref(), &query).await;
        selected.sort();
        assert_eq!(
            selected,
            ["I_edited", "I_new", "I_theirs"],
            "the edited comment's issue and both newly commented on, though the search \
             still answers each as it was before; never the quiet one ({query:?})"
        );
    }
    assert!(
        !fixture.comment_reads().iter().any(|read| read == "I_quiet"),
        "an issue nobody commented on is still ruled out by its own updatedAt: {:?}",
        fixture.comment_reads()
    );

    // Once the index has caught up with `I_theirs`, the search names it and its copy there is
    // what the read confirms: it is not read again by its own node.
    fixture.index_catches_up("I_theirs");
    let node_reads = fixture.requests("issue");
    let mut selected = selected_tasks(source.as_ref(), &commented_since(None)).await;
    selected.sort();
    assert_eq!(selected, ["I_edited", "I_new", "I_theirs"]);
    assert_eq!(
        fixture.requests("issue"),
        node_reads,
        "an issue the search named was read again by its own node"
    );

    // An issue this source deleted is no longer one it commented on: it is neither selected
    // nor looked for by its own node.
    source.delete_task(&native("I_theirs")).await.unwrap();
    let node_reads = fixture.requests("issue");
    let mut selected = selected_tasks(source.as_ref(), &commented_since(None)).await;
    selected.sort();
    assert_eq!(selected, ["I_edited", "I_new"]);
    assert_eq!(
        fixture.requests("issue"),
        node_reads,
        "a deleted issue was still looked for as one this source commented on"
    );

    // What this process wrote is held for one command, like every other record of its own
    // writes: the next command answers from the search alone, which is still behind.
    source.end_command().await.unwrap();
    assert_eq!(
        selected_tasks(source.as_ref(), &commented_since(None)).await,
        Vec::<String>::new()
    );
}

/// A read narrowed to one project asks that project for its children by their own nodes, not
/// the lagging search, so a child this source commented on carries a current `updatedAt`. The
/// record that it was commented on still exempts it from being ruled out by that `updatedAt`.
/// This proves the exemption keeps the read exact. A child commented on after the instant is
/// selected. Asked from an instant after that comment, the same child has its comments read,
/// which is what the exemption costs, and is not selected. A sibling nobody commented on is
/// never selected and never has its comments read.
#[tokio::test]
async fn a_project_read_after_this_source_commented_selects_exactly_the_children_commented_since() {
    let fixture = board(vec![
        Item::issue("I_plan", "the plan").sub_issues(2),
        Item::issue("I_commented", "commented on")
            .parent("I_plan")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
        Item::issue("I_quiet", "quiet")
            .parent("I_plan")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z"),
    ]);
    let source = source(&fixture);
    let in_plan_since = |since: &str| TaskQuery {
        project: ProjectFilter::Is(native("I_plan")),
        commented_since: Some(since.parse().unwrap()),
        ..TaskQuery::default()
    };
    // The comment written below is stamped by this board's clock, which reads 2026-09-01.
    let before = in_plan_since("2026-08-01T00:00:00Z");
    let after = in_plan_since("2026-09-02T00:00:00Z");
    assert_eq!(
        selected_tasks(source.as_ref(), &before).await,
        Vec::<String>::new(),
        "before any comment activity after the instant"
    );

    source
        .add_comment(&native("I_commented"), &commenting("new"))
        .await
        .unwrap()
        .expect("a task this board holds");

    assert_eq!(
        selected_tasks(source.as_ref(), &before).await,
        ["I_commented"],
        "the child commented on after the instant, and not its quiet sibling"
    );
    assert_eq!(
        selected_tasks(source.as_ref(), &after).await,
        Vec::<String>::new(),
        "a child whose only comment predates the instant, though this source wrote it"
    );
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(
        fixture.comment_reads(),
        ["I_commented", "I_commented"],
        "the commented child's comments are what decide both reads, and a child nobody \
         commented on had its comments read"
    );
}

/// A comment write GitHub refuses wrote nothing, so it makes no issue a candidate: the next
/// comment-activity read selects nothing, sends no node read of that issue and reads none of
/// its comments — exactly what it would have done had the write never been attempted.
#[tokio::test]
async fn a_comment_write_github_refuses_leaves_the_next_comment_activity_read_as_it_was() {
    for operation in ["addComment", "updateIssueComment"] {
        let fixture = board(vec![
            Item::issue("I_task", "a step")
                .status("Todo")
                .updated("2026-06-02T09:00:00Z"),
        ]);
        let old = fixture.commented_at("I_task", LONG_BEFORE, "2026-06-02T09:00:00Z");
        fixture.indexes_behind("I_task");
        fixture.refuse(operation);
        let source = source(&fixture);
        let task = native("I_task");
        let outcome = match operation {
            "addComment" => source
                .add_comment(&task, &commenting("hello"))
                .await
                .map(|_| ()),
            _ => source
                .edit_comment(&task, &native(&old), &comment_body("changed"))
                .await
                .map(|_| ()),
        };
        assert!(
            matches!(outcome, Err(SourceError::Refused { .. })),
            "{operation} refused by GitHub answered {outcome:?}"
        );

        let node_reads = fixture.requests("issue");
        let query = TaskQuery {
            commented_since: Some("2026-08-01T00:00:00Z".parse().unwrap()),
            ..TaskQuery::default()
        };
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            Vec::<String>::new(),
            "an issue whose {operation} was refused was selected"
        );
        assert_eq!(
            fixture.requests("issue"),
            node_reads,
            "an issue whose {operation} was refused was read by its own node"
        );
        assert_eq!(
            fixture.comment_reads(),
            Vec::<String>::new(),
            "an issue whose {operation} was refused had its comments read"
        );
    }
}

/// The metadata slot at the end of `body`, parsed, or an empty object when it has none.
fn raw_slot(body: &str) -> Value {
    let Some(start) = body.rfind("<!-- onetaskgraph.metadata\n") else {
        return json!({});
    };
    let encoded = &body[start + "<!-- onetaskgraph.metadata\n".len()..];
    let end = encoded.find("\n-->").expect("a terminated slot");
    serde_json::from_str(&encoded[..end]).expect("the slot is JSON")
}

/// What the release before this one — onetaskgraph 0.2.51 — reports as an item's caller
/// metadata, restated from its `Resolved::metadata`: the slot less the five keys it treats as
/// an encoding, with the origin field's value put over the slot's own entry under that key.
///
/// Restated rather than run, because that release is not a dependency of this one; pinned to
/// its text so a reader can check it against the tag.
fn read_as_the_release_before(item: &Item) -> BTreeMap<String, Value> {
    let mut metadata: BTreeMap<String, Value> =
        serde_json::from_value(raw_slot(item.body.as_deref().unwrap_or_default()))
            .expect("a metadata map");
    for encoding in [
        "onetaskgraph.repositories",
        "onetaskgraph.depends_on",
        "onetaskgraph.item_kind",
        "onetaskgraph.delivers",
        "onetaskgraph.delivered_by",
    ] {
        metadata.remove(encoding);
    }
    if let Some(origin) = item.origin.as_ref().filter(|origin| !origin.is_empty()) {
        metadata.insert("onetaskgraph.origin".to_owned(), json!(origin));
    }
    metadata
}

fn origin_query(origin: &str) -> TaskQuery {
    TaskQuery {
        origin: Some(origin.to_owned()),
        ..TaskQuery::default()
    }
}

fn metadata_query(key: &str, path: &[&str], value: &str) -> TaskQuery {
    TaskQuery {
        metadata: vec![
            MetadataMatch::new(
                key.to_owned(),
                path.iter().map(|segment| (*segment).to_owned()).collect(),
                value.to_owned(),
            )
            .expect("a metadata location"),
        ],
        ..TaskQuery::default()
    }
}

/// A board of plain tasks, one carrying caller metadata and one a copy origin, for the reads
/// below to narrow.
fn narrowing_board() -> Fixture {
    board(vec![
        Item::issue("I_ship", "Ship it")
            .status("Todo")
            .body("the release notes"),
        Item::issue("I_shipment", "Shipment plan").status("Todo"),
        Item::issue("I_owned", "Owned")
            .status("Todo")
            .body(&slotted(
                "stale-cache is mentioned here",
                &json!({"orchestrator.follow-up": {"root_cause": "stale-cache"}}),
            )),
        Item::issue("I_copied", "Copied")
            .status("Todo")
            .carrying("work:ENG-1"),
        Item::draft("D_ship", "Ship it too").status("Todo"),
    ])
}

#[tokio::test]
async fn a_text_metadata_or_origin_query_asks_a_narrower_question_than_the_board() {
    let cases = [
        (
            "text",
            TaskQuery {
                text: text("Ship it", TextFields::Title),
                ..TaskQuery::default()
            },
            vec!["I_ship"],
            "in:title \"Ship it\"",
        ),
        (
            "metadata",
            metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache"),
            vec!["I_owned"],
            "in:body \"stale-cache\"",
        ),
        (
            "origin",
            origin_query("work:ENG-1"),
            vec!["I_copied"],
            "in:body \"work:ENG-1\"",
        ),
    ];
    for (what, query, expected, qualifier) in cases {
        let fixture = narrowing_board();
        let source = source(&fixture);
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            expected,
            "{what}"
        );
        assert_eq!(fixture.requests("board"), 0, "{what} read the board");
        assert_eq!(
            fixture.board_item_reads(),
            Vec::<String>::new(),
            "{what} walked the board's items"
        );
        let searches = fixture.searches();
        assert!(!searches.is_empty(), "{what} sent no search at all");
        for search in &searches {
            assert_eq!(
                search,
                &format!("project:octo-org/7 is:issue {qualifier}"),
                "{what} sent a board-scoped search without its own qualifier"
            );
        }
        if what == "origin" {
            assert_eq!(
                fixture.origin_filters(),
                ["onetaskgraph.origin:\"work:ENG-1\""],
                "the origin is asked of the board's own field filter, quoted"
            );
        } else {
            assert_eq!(fixture.origin_filters(), Vec::<String>::new());
        }
    }

    // And a query carrying none of the three still reads the board as before: its items and
    // its board-scoped search, and no narrowed search.
    let fixture = narrowing_board();
    let source = source(&fixture);
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                statuses: vec![StatusCategory::Todo],
                ..TaskQuery::default()
            }
        )
        .await,
        ["I_ship", "I_shipment", "I_owned", "I_copied", "D_ship"]
    );
    assert_eq!(fixture.requests("board"), 1);
    assert_eq!(fixture.searches(), ["project:octo-org/7 is:issue"]);
    assert_eq!(fixture.origin_filters(), Vec::<String>::new());
}

#[tokio::test]
async fn a_text_answer_is_what_githubs_tokens_find_confirmed_by_the_substring_rule() {
    let fixture = narrowing_board();
    let source = source(&fixture);
    let titled = |terms: &str, fields| TaskQuery {
        text: text(terms, fields),
        ..TaskQuery::default()
    };
    // `Shipment plan` holds "ship" as a substring and not as a word, so GitHub's token match
    // does not return it — the narrowing this source declares — and the substring rule is
    // never asked about it.
    assert_eq!(
        selected_tasks(source.as_ref(), &titled("ship", TextFields::Title)).await,
        ["I_ship"]
    );
    // Tokens GitHub matches that the substring rule does not: `ship-it` is the words `ship
    // it`, which `Ship it` holds, but the text `ship-it` is in no title, so nothing returned
    // fails to contain what was asked for.
    assert_eq!(
        selected_tasks(source.as_ref(), &titled("ship-it", TextFields::Title)).await,
        Vec::<String>::new()
    );
    // Content and either field are the same search in the body, and in both.
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &titled("release notes", TextFields::Content)
        )
        .await,
        ["I_ship"]
    );
    assert_eq!(
        selected_tasks(source.as_ref(), &titled("release notes", TextFields::Title)).await,
        Vec::<String>::new()
    );
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &titled("RELEASE NOTES", TextFields::TitleOrContent)
        )
        .await,
        ["I_ship"]
    );
    // A board draft is not an issue, so no search lists it, whatever its title holds.
    assert!(
        !selected_tasks(source.as_ref(), &titled("too", TextFields::Title))
            .await
            .contains(&"D_ship".to_owned())
    );
    assert_eq!(
        fixture.searches(),
        [
            "project:octo-org/7 is:issue in:title \"ship\"",
            "project:octo-org/7 is:issue in:title \"ship-it\"",
            "project:octo-org/7 is:issue in:body \"release notes\"",
            "project:octo-org/7 is:issue in:title \"release notes\"",
            "project:octo-org/7 is:issue in:title,body \"RELEASE NOTES\"",
            "project:octo-org/7 is:issue in:title \"too\"",
        ]
    );
}

#[tokio::test]
async fn a_metadata_answer_is_confirmed_against_the_parsed_metadata_comment() {
    let fixture = board(vec![
        Item::issue("I_held", "held").status("Todo").body(&slotted(
            "prose",
            &json!({"orchestrator.follow-up": {"root_cause": "stale-cache"}, "team.owner": "ada"}),
        )),
        // The value in the prose and a different one in the slot: GitHub's index finds the
        // words, and the slot says no.
        Item::issue("I_prose", "prose")
            .status("Todo")
            .body(&slotted(
                "caused by stale-cache",
                &json!({"orchestrator.follow-up": {"root_cause": "other"}}),
            )),
        // A value whose words hold the ones asked for, which GitHub's token match returns and
        // an exact comparison does not.
        Item::issue("I_longer", "longer")
            .status("Todo")
            .body(&slotted(
                "",
                &json!({"orchestrator.follow-up": {"root_cause": "stale-cache-2"}}),
            )),
        // The right value at the wrong depth.
        Item::issue("I_shallow", "shallow")
            .status("Todo")
            .body(&slotted(
                "",
                &json!({"orchestrator.follow-up": "stale-cache"}),
            )),
    ]);
    let source = source(&fixture);
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache")
        )
        .await,
        ["I_held"]
    );
    // Several matches are ANDed, and every value is one more phrase of the same search.
    let mut both = metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache");
    both.metadata.push(
        MetadataMatch::new("team.owner".to_owned(), Vec::new(), "ada".to_owned())
            .expect("a metadata location"),
    );
    assert_eq!(selected_tasks(source.as_ref(), &both).await, ["I_held"]);
    both.metadata[1] =
        MetadataMatch::new("team.owner", Vec::new(), "bob").expect("a metadata location");
    assert_eq!(
        selected_tasks(source.as_ref(), &both).await,
        Vec::<String>::new()
    );
    assert_eq!(
        fixture.searches(),
        [
            "project:octo-org/7 is:issue in:body \"stale-cache\"",
            "project:octo-org/7 is:issue in:body \"stale-cache\" \"ada\"",
            "project:octo-org/7 is:issue in:body \"stale-cache\" \"bob\"",
        ]
    );
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn every_request_a_text_metadata_or_origin_query_sends_binds_every_non_null_variable() {
    // GitHub refuses a request that leaves a variable it declares non-null unbound, before it
    // runs a thing — "Variable $filter of type String! was provided invalid value" is how the
    // credentialed lane met the origin lookup's cost probe sent without its filter. This board
    // refuses the same way, so each of these reads, walked a page at a time to exhaustion,
    // would fail naming the variable; what is asserted besides is that nothing was refused.
    let mut all_three = metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache");
    all_three.text = text("mentioned", TextFields::Content);
    all_three.origin = Some("work:ENG-1".to_owned());
    let cases = [
        (
            "title text",
            TaskQuery {
                text: text("Ship it", TextFields::Title),
                ..TaskQuery::default()
            },
            vec!["I_ship"],
        ),
        (
            "content text",
            TaskQuery {
                text: text("release notes", TextFields::Content),
                ..TaskQuery::default()
            },
            vec!["I_ship"],
        ),
        (
            "title-or-content text",
            TaskQuery {
                text: text("ship", TextFields::TitleOrContent),
                ..TaskQuery::default()
            },
            // "Shipment plan" holds the substring but not the token, so GitHub's search
            // never offers it: the documented narrowing.
            vec!["I_ship"],
        ),
        (
            "metadata",
            metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache"),
            vec!["I_owned"],
        ),
        ("origin", origin_query("work:ENG-1"), vec!["I_copied"]),
        ("all three at once", all_three, vec![]),
    ];
    for (what, query, expected) in cases {
        let fixture = narrowing_board();
        let source = source(&fixture);
        let mut selected = Vec::new();
        let mut request = page(1);
        loop {
            let answered = source
                .query_tasks(&query, &request)
                .await
                .unwrap_or_else(|error| panic!("{what}: {error}"));
            selected.extend(answered.items.into_iter().map(|task| task.id.0));
            let Some(next) = answered.next else { break };
            request = resume(&next.0, 1);
        }
        selected.sort();
        assert_eq!(selected, expected, "{what}");
        assert_eq!(
            fixture.unbound_variables(),
            Vec::<String>::new(),
            "{what} sent a request leaving a non-null variable unbound"
        );
        assert!(
            !fixture.searches().is_empty(),
            "{what} sent no search at all"
        );
    }
}

#[tokio::test]
async fn an_origin_lookup_finds_every_carrier_without_enumerating_the_board() {
    let fixture = board(vec![
        // Written the way the release before this one writes a copy: the origin in the board
        // field and nowhere in the body.
        Item::issue("I_before", "copied before")
            .status("Todo")
            .carrying("work:ENG-1")
            .body(&slotted(
                "prose",
                &json!({"onetaskgraph.item_kind": "task"}),
            )),
        // A suffix and a prefix of the id, in the field and in the mirror both: the body
        // search's tokens find the first, and neither is the origin asked for.
        Item::issue("I_suffix", "suffix")
            .status("Todo")
            .carrying("work:ENG-1-copy")
            .body(&slotted(
                "",
                &json!({"onetaskgraph.origin": "work:ENG-1-copy"}),
            )),
        Item::issue("I_prefix", "prefix")
            .status("Todo")
            .carrying("otherwork:ENG-1")
            .body(&slotted(
                "",
                &json!({"onetaskgraph.origin": "otherwork:ENG-1"}),
            )),
        Item::issue("I_unrelated", "unrelated").status("Todo"),
    ]);
    // Written by this release, from another process.
    let writer = source(&fixture);
    let mut carried = task("T-2", "copied now", status(StatusCategory::Todo, "Todo"));
    carried.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("work:ENG-1"))]);
    let now = writer.write_task(&write(carried)).await.unwrap();
    let written = fixture.item(&now.0);
    assert_eq!(written.origin.as_deref(), Some("work:ENG-1"));
    assert_eq!(
        raw_slot(written.body.as_deref().unwrap_or_default())["onetaskgraph.origin"],
        json!("work:ENG-1")
    );

    let before = fixture.documents().len();
    let reader = source(&fixture);
    let mut found = selected_tasks(reader.as_ref(), &origin_query("work:ENG-1")).await;
    found.sort();
    let mut expected = vec!["I_before".to_owned(), now.0.clone()];
    expected.sort();
    assert_eq!(found, expected);
    let sent = fixture.documents()[before..].to_vec();
    assert!(
        sent.iter()
            .all(|document| document == onetaskgraph_github_projects::graphql::ORIGIN_LOOKUP),
        "the lookup sent something other than the origin lookup: {sent:#?}"
    );
    assert_eq!(fixture.requests("board"), 0);
    assert_eq!(fixture.board_item_reads(), Vec::<String>::new());
}

#[tokio::test]
async fn a_carrier_another_process_just_wrote_is_found_through_the_body_mirror() {
    let fixture = board(vec![Item::issue("I_other", "other").status("Todo")]);
    // From here on the board's own item connection — and so its field filter — lists
    // nothing new, while its issue search is current.
    fixture.items_connection_falls_behind();
    let writer = source(&fixture);
    let mut carried = task("T-3", "fresh copy", status(StatusCategory::Todo, "Todo"));
    carried.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("work:ENG-7"))]);
    let fresh = writer.write_task(&write(carried)).await.unwrap();
    // And one the release before this one wrote in the same moment, with no mirror to find.
    fixture.filed_by_something_else(
        Item::issue("I_unmirrored", "unmirrored")
            .status("Todo")
            .carrying("work:ENG-7"),
    );

    let reader = source(&fixture);
    assert_eq!(
        selected_tasks(reader.as_ref(), &origin_query("work:ENG-7")).await,
        std::slice::from_ref(&fresh.0),
        "the mirror is found while the field filter is behind; an item carrying the origin \
         only in its field is in the window the contract states until that filter catches up"
    );

    // Once the board's own connection catches up, the field filter finds both.
    fixture.state.lock().unwrap().items_connection_behind_from = None;
    let caught_up = source(&fixture);
    let mut found = selected_tasks(caught_up.as_ref(), &origin_query("work:ENG-7")).await;
    found.sort();
    let mut expected = vec!["I_unmirrored".to_owned(), fresh.0.clone()];
    expected.sort();
    assert_eq!(found, expected);
}

#[tokio::test]
async fn what_this_process_wrote_is_returned_while_the_index_is_behind_it() {
    let fixture = board(vec![Item::issue("I_other", "other").status("Todo")]);
    let source = source(&fixture);
    let mut written = task("T-4", "Fresh ship", status(StatusCategory::Todo, "Todo"));
    written.metadata = BTreeMap::from([
        ("onetaskgraph.origin".to_owned(), json!("work:ENG-9")),
        (
            "orchestrator.follow-up".to_owned(),
            json!({"root_cause": "fresh-cause"}),
        ),
    ]);
    let id = source.write_task(&write(written)).await.unwrap();
    // Every enumeration of the board — its items, its field filter, its issue search — is
    // held behind the write.
    fixture.read_behind(1);
    fixture.items_connection_falls_behind();

    let queries = [
        TaskQuery {
            text: text("fresh ship", TextFields::Title),
            ..TaskQuery::default()
        },
        metadata_query("orchestrator.follow-up", &["root_cause"], "fresh-cause"),
        origin_query("work:ENG-9"),
    ];
    for query in &queries {
        assert_eq!(
            selected_tasks(source.as_ref(), query).await,
            std::slice::from_ref(&id.0),
            "{query:?}"
        );
        // A source that did not write it cannot see it yet, which is what makes the answer
        // above this process's own record rather than GitHub's.
        let stranger = self::source(&fixture);
        assert_eq!(
            selected_tasks(stranger.as_ref(), query).await,
            Vec::<String>::new(),
            "{query:?}"
        );
    }
    // And its own record never adds an item a query does not match.
    assert_eq!(
        selected_tasks(source.as_ref(), &origin_query("work:ENG-99")).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_narrowed_search_fetches_only_the_pages_each_answer_needs() {
    let items = (0..130)
        .map(|index| {
            Item::issue(&format!("I_{index:03}"), &format!("widget {index}")).status("Todo")
        })
        .collect::<Vec<_>>();
    let fixture = board(items);
    let source = source(&fixture);
    let query = TaskQuery {
        text: text("widget", TextFields::Title),
        ..TaskQuery::default()
    };
    let first = source.query_tasks(&query, &page(100)).await.unwrap();
    assert_eq!(first.items.len(), 100);
    assert_eq!(search_sizes(&fixture), [20; 5]);
    let next = first.next.expect("a cursor to the rest");
    let rest = source
        .query_tasks(&query, &resume(&next.0, 100))
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 30);
    assert_eq!(rest.next, None);
    assert_eq!(
        fixture.requests("search"),
        7,
        "five pages of twenty fill the first answer; only resuming asks for the rest"
    );
    assert_eq!(search_sizes(&fixture), [20; 7]);
    assert_eq!(fixture.requests("board"), 0);
}

async fn walk_tasks(source: &dyn TaskSource, query: &TaskQuery, limit: u32) -> Vec<String> {
    let mut request = page(limit);
    let mut ids = Vec::new();
    loop {
        let answer = source.query_tasks(query, &request).await.unwrap();
        ids.extend(answer.items.into_iter().map(|task| task.id.0));
        match answer.next {
            Some(cursor) => request = resume(&cursor.0, limit),
            None => return ids,
        }
    }
}

fn search_sizes(fixture: &Fixture) -> Vec<u64> {
    fixture
        .state
        .lock()
        .unwrap()
        .bindings
        .iter()
        .filter(|(operation, _)| operation == "search")
        .map(|(_, variables)| variables["first"].as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn narrowing_pages_are_bounded_and_resume_on_a_fresh_source_without_gaps() {
    for count in [0, 3, 21, 130] {
        for metadata in [false, true] {
            let fixture = board(
                (0..count)
                    .map(|index| {
                        Item::issue(&format!("I_{index:03}"), &format!("widget {index}"))
                            .status("Todo")
                            .body(&slotted("", &json!({"team.owner":"ada"})))
                    })
                    .collect(),
            );
            let query = if metadata {
                metadata_query("team.owner", &[], "ada")
            } else {
                TaskQuery {
                    text: text("widget", TextFields::Title),
                    ..TaskQuery::default()
                }
            };
            let source = source(&fixture);
            let first = source.query_tasks(&query, &page(3)).await.unwrap();
            assert_eq!(first.items.len(), count.min(3));
            assert_eq!(search_sizes(&fixture), [20]);
            assert_eq!(source.query_tasks(&query, &page(3)).await.unwrap(), first);
            assert_eq!(search_sizes(&fixture), [20]);
            if count > 3 {
                assert_eq!(
                    serde_json::from_str::<Value>(&first.next.as_ref().unwrap().0).unwrap(),
                    serde_json::from_str::<Value>(include_str!("fixtures/search-cursor-v4.json"))
                        .unwrap()
                );
            }
            let mut ids = first
                .items
                .iter()
                .map(|task| task.id.0.clone())
                .collect::<Vec<_>>();
            let mut next = first.next;
            while let Some(cursor) = next {
                let fresh = self::source(&fixture);
                let answer = fresh
                    .query_tasks(&query, &resume(&cursor.0, 20))
                    .await
                    .unwrap();
                ids.extend(answer.items.into_iter().map(|task| task.id.0));
                next = answer.next;
            }
            assert_eq!(
                ids,
                (0..count)
                    .map(|index| format!("I_{index:03}"))
                    .collect::<Vec<_>>()
            );
            // A wider ask cannot use the three-row cache entry as a complete answer.
            assert_eq!(walk_tasks(source.as_ref(), &query, 100).await, ids);
        }
    }
}

#[tokio::test]
async fn a_paged_walk_reaches_one_whole_page_when_github_orders_by_page_size() {
    // What the credentialed lane caught: GitHub answered three issues in one order at a page
    // of twenty and in another a row at a time, so a walk that sized its requests by the
    // rows it still needed disagreed with one whole page of the same search.
    for count in [3_usize, 20, 45] {
        let fixture = board(
            (0..count)
                .map(|index| {
                    Item::issue(&format!("I_{index:03}"), &format!("widget {index}")).status("Todo")
                })
                .collect(),
        );
        fixture.order_search_by_page_size();
        let query = TaskQuery {
            text: text("widget", TextFields::Title),
            ..TaskQuery::default()
        };
        let whole = source(&fixture)
            .query_tasks(&query, &page(100))
            .await
            .unwrap();
        assert_eq!(whole.next, None);
        let asked = search_requests(&fixture);
        let whole = whole
            .items
            .into_iter()
            .map(|task| task.id.0)
            .collect::<Vec<_>>();
        let mut held = whole.clone();
        held.sort();
        assert_eq!(
            held,
            (0..count)
                .map(|index| format!("I_{index:03}"))
                .collect::<Vec<_>>(),
            "one whole page holds every match once"
        );
        for limit in [1, 3] {
            let before = search_requests(&fixture).len();
            assert_eq!(
                walk_tasks(source(&fixture).as_ref(), &query, limit).await,
                whole,
                "a walk in pages of {limit} in one process"
            );
            assert_eq!(
                search_requests(&fixture)[before..],
                asked[..],
                "a walk in pages of {limit} in one process sends each of the whole read's \
                 requests exactly once"
            );
            let before = search_requests(&fixture).len();
            let mut walked = Vec::new();
            let mut request = page(limit);
            loop {
                let answer = source(&fixture)
                    .query_tasks(&query, &request)
                    .await
                    .unwrap();
                assert!(answer.items.len() <= limit as usize);
                walked.extend(answer.items.into_iter().map(|task| task.id.0));
                match answer.next {
                    Some(cursor) => request = resume(&cursor.0, limit),
                    None => break,
                }
            }
            assert_eq!(
                walked, whole,
                "a walk in pages of {limit}, each in a new process"
            );
            // A new process holds nothing, so it asks again for each page its rows lie in —
            // the very request the whole read sent for that page, and no other. Nothing it
            // was handed is answered from the token: see the module's paging contract.
            let pages_per_process = (0..count.div_ceil(limit as usize)).map(|process| {
                let rows = process * limit as usize..((process + 1) * limit as usize).min(count);
                (rows.start / 20..=(rows.end - 1) / 20)
                    .map(|page| asked[page].clone())
                    .collect::<Vec<_>>()
            });
            assert_eq!(
                search_requests(&fixture)[before..],
                pages_per_process.flatten().collect::<Vec<_>>()[..],
                "a walk in pages of {limit}, each in a new process, sends each process the \
                 whole read's requests for the pages its rows lie in, once each"
            );
        }
        assert_eq!(asked.len(), count.div_ceil(20));
        assert!(asked.iter().all(|(first, _)| *first == 20));
    }
}

/// Every board search this fixture answered, as the page size and cursor it was asked at.
fn search_requests(fixture: &Fixture) -> Vec<(u64, Value)> {
    fixture
        .state
        .lock()
        .unwrap()
        .bindings
        .iter()
        .filter(|(operation, _)| operation == "search")
        .map(|(_, variables)| {
            (
                variables["first"].as_u64().unwrap(),
                variables["after"].clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_partial_narrowing_failure_can_retry_without_truncating_the_cached_page() {
    let fixture = board(
        (0..130)
            .map(|index| Item::issue(&format!("I_{index:03}"), "widget").status("Todo"))
            .collect(),
    );
    let source = source(&fixture);
    let query = TaskQuery {
        text: text("widget", TextFields::Title),
        ..TaskQuery::default()
    };
    fixture.refuse_after("search", 1);
    let error = source.query_tasks(&query, &page(100)).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("search is refused by this board")
    );
    // Bindings record answered calls; the refused second page has no answer to cache.
    assert_eq!(search_sizes(&fixture), [20]);
    assert_eq!(fixture.requests("search"), 2);
    fixture.state.lock().unwrap().refuse_after.remove("search");
    let answer = walk_tasks(source.as_ref(), &query, 100).await;
    assert_eq!(
        answer,
        (0..130)
            .map(|index| format!("I_{index:03}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(search_sizes(&fixture), [20; 7]);
    assert_eq!(fixture.requests("search"), 8);
}

#[tokio::test]
async fn resumed_pending_writes_handle_a_missing_node_and_retry_a_refused_node() {
    for missing in [false, true] {
        let fixture = board(vec![]);
        let writer = source(&fixture);
        let mut created = Vec::new();
        for title in ["widget first", "widget second"] {
            created.push(
                writer
                    .write_task(&write(task(
                        "ignored",
                        title,
                        status(StatusCategory::Todo, "Todo"),
                    )))
                    .await
                    .unwrap(),
            );
        }
        fixture.read_behind(2);
        let query = TaskQuery {
            text: text("widget", TextFields::Title),
            ..TaskQuery::default()
        };
        let first = writer.query_tasks(&query, &page(1)).await.unwrap();
        assert_eq!(first.items[0].id, created[0]);
        let request = resume(&first.next.unwrap().0, 1);
        let fresh = source(&fixture);
        if missing {
            source(&fixture).delete_task(&created[1]).await.unwrap();
        }
        let reads_before = fixture.requests("issue");
        if !missing {
            fixture.script_for("issue", vec![Refusal::unavailable()]);
            assert!(fresh.query_tasks(&query, &request).await.is_err());
        }
        let answer = fresh.query_tasks(&query, &request).await.unwrap();
        assert_eq!(answer.next, None);
        if missing {
            assert!(answer.items.is_empty());
        } else {
            assert_eq!(answer.items.len(), 1);
            assert_eq!(answer.items[0].id, created[1]);
        }
        assert_eq!(search_sizes(&fixture), [20]);
        assert_eq!(
            fixture.requests("issue") - reads_before,
            if missing { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn project_name_searches_stop_at_the_exact_match_or_connection_end() {
    for count in [0, 3, 21, 130] {
        for exact in [false, true] {
            let mut items = (0..count)
                .map(|index| {
                    Item::issue(&format!("I_{index}"), &format!("Plan extra {index}")).sub_issues(1)
                })
                .collect::<Vec<_>>();
            if exact && count > 0 {
                items[count - 1] = Item::issue("I_exact", "Plan").sub_issues(1);
                items.push(
                    Item::issue("I_child", "child")
                        .parent("I_exact")
                        .status("Todo"),
                );
            }
            let fixture = board(items);
            let query = TaskQuery {
                project: ProjectFilter::Is(native("Plan")),
                ..TaskQuery::default()
            };
            assert_eq!(
                selected_tasks(source(&fixture).as_ref(), &query).await,
                if exact && count > 0 {
                    vec!["I_child"]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                search_sizes(&fixture),
                match count {
                    0 | 3 => vec![20],
                    21 => vec![20, 20],
                    _ => vec![20; 7],
                }
            );
        }
    }
}

#[tokio::test]
async fn a_project_name_on_a_later_search_page_is_found() {
    let mut items = (0..130)
        .map(|index| {
            Item::issue(&format!("I_{index}"), &format!("Plan extra {index}")).sub_issues(1)
        })
        .collect::<Vec<_>>();
    items.push(Item::issue("I_exact", "Plan").sub_issues(1));
    items.push(
        Item::issue("I_child", "child")
            .parent("I_exact")
            .status("Todo"),
    );
    let fixture = board(items);
    let query = TaskQuery {
        project: ProjectFilter::Is(native("Plan")),
        ..TaskQuery::default()
    };
    assert_eq!(
        selected_tasks(source(&fixture).as_ref(), &query).await,
        ["I_child"]
    );
    assert_eq!(search_sizes(&fixture), [20; 7]);
}

#[tokio::test]
async fn comment_search_pages_stop_when_the_matching_answer_is_full() {
    for count in [0, 3, 21, 130] {
        let fixture = board(
            (0..count)
                .map(|index| {
                    Item::issue(&format!("I_{index:03}"), "activity")
                        .status("Todo")
                        .updated("2026-09-23T09:00:00Z")
                })
                .collect(),
        );
        for index in 0..count {
            fixture.commented_at(
                &format!("I_{index:03}"),
                "2026-09-23T09:00:00Z",
                "2026-09-23T09:00:00Z",
            );
        }
        let source = source(&fixture);
        let query = commented_since(Vec::new());
        let first = source.query_tasks(&query, &page(100)).await.unwrap();
        assert_eq!(first.items.len(), count.min(100));
        assert_eq!(
            search_sizes(&fixture),
            vec![
                20;
                if count > 20 {
                    count.min(100).div_ceil(20)
                } else {
                    1
                }
            ]
        );
        if let Some(next) = first.next {
            assert_eq!(
                source
                    .query_tasks(&query, &resume(&next.0, 100))
                    .await
                    .unwrap()
                    .items
                    .len(),
                count - 100
            );
        }
    }
}

#[tokio::test]
async fn own_writes_win_exactly_once_on_each_side_of_a_page_boundary() {
    for count in [3, 21, 130] {
        for stale in [false, true] {
            let fixture = board(
                (0..count)
                    .map(|index| {
                        Item::issue(&format!("I_{index:03}"), &format!("widget old {index}"))
                            .status("Todo")
                    })
                    .collect(),
            );
            let source = source(&fixture);
            // Updates at the start, a page edge, and the end keep their index positions.
            for index in [0, count / 2, count - 1] {
                let id = format!("I_{index:03}");
                fixture.indexes_behind(&id);
                source
                    .write_task(&ItemWrite {
                        target: Some(native(&id)),
                        item: task(&id, "widget fresh", status(StatusCategory::Todo, "Todo")),
                        depends_on: vec![],
                    })
                    .await
                    .unwrap();
            }
            let created = source
                .write_task(&write(task(
                    "ignored",
                    "widget created",
                    status(StatusCategory::Todo, "Todo"),
                )))
                .await
                .unwrap();
            if !stale {
                fixture.read_behind(1);
            } else {
                fixture.indexes_behind(&created.0);
            }
            let query = TaskQuery {
                text: text("widget", TextFields::Title),
                ..TaskQuery::default()
            };
            let mut request = page(3);
            let mut tasks = Vec::new();
            loop {
                let answer = source.query_tasks(&query, &request).await.unwrap();
                tasks.extend(answer.items);
                match answer.next {
                    Some(cursor) => request = resume(&cursor.0, 3),
                    None => break,
                }
            }
            assert_eq!(tasks.len(), count + 1);
            let ids = tasks
                .iter()
                .map(|task| task.id.0.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(ids.len(), count + 1);
            for index in [0, count / 2, count - 1] {
                assert_eq!(
                    tasks
                        .iter()
                        .find(|task| task.id.0 == format!("I_{index:03}"))
                        .unwrap()
                        .title,
                    "widget fresh"
                );
            }
            assert!(ids.contains(&created.0));
            let before = fixture.requests("search");
            assert_eq!(
                walk_tasks(source.as_ref(), &query, 3).await.len(),
                count + 1
            );
            assert_eq!(fixture.requests("search"), before);
        }
    }
}

#[tokio::test]
async fn a_search_cursor_with_an_unknown_version_is_refused_without_a_request() {
    let fixture = narrowing_board();
    let query = TaskQuery {
        text: text("ship", TextFields::Title),
        ..TaskQuery::default()
    };
    let error = source(&fixture)
        .query_tasks(
            &query,
            &resume(r#"{"version":99,"connection":{"state":"exhausted"}}"#, 3),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("page cursor is invalid"));
    assert_eq!(fixture.requests("search"), 0);
}

#[tokio::test]
async fn metadata_accounting_names_only_search_and_needed_membership_recovery() {
    for overflow in [false, true] {
        let boards = if overflow {
            boards_ahead_of_this_one()
        } else {
            vec![]
        };
        let fixture = board(vec![
            Item::issue("I_match", "match")
                .status("Todo")
                .body(&slotted("", &json!({"team.owner":"ada"})))
                .also_on(&boards),
        ]);
        let ledger = Arc::new(Accounting::new());
        let source = recording(&fixture.endpoint, &ledger);
        let query = metadata_query("team.owner", &[], "ada");
        assert_eq!(selected_tasks(source.as_ref(), &query).await, ["I_match"]);
        let session = ledger.snapshot();
        assert_eq!(session.total_requests(), if overflow { 2 } else { 1 });
        assert_eq!(fixture.membership_walks(), usize::from(overflow));
        assert_eq!(search_sizes(&fixture), [20]);
        let report = session.report();
        assert!(report.contains("searching this board's issues"));
        assert_eq!(
            report.contains("reading one issue's board memberships past the page it came with"),
            overflow
        );
        // The report's unreported prices are lower bounds. Price the actual bindings too.
        let bindings = fixture.state.lock().unwrap().bindings.clone();
        for ((_, variables), document) in bindings.iter().zip(fixture.documents()) {
            let sizes = variables
                .as_object()
                .unwrap()
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_u64()
                        .map(|value| (key.clone(), u32::try_from(value).unwrap()))
                })
                .collect();
            assert_eq!(
                github_graphql_node_count::point_cost(&document, &sizes).unwrap(),
                1
            );
        }
        for request in session.requests() {
            assert_eq!(request.spend().amount(), 1);
        }
    }
}

#[tokio::test]
async fn a_text_metadata_or_origin_query_costs_the_same_on_a_board_of_several_pages() {
    // The one-page board is `narrowing_board`; the other is the same board with 350 more
    // items that none of the three questions matches — four pages of `ProjectV2.items` and of
    // an unqualified board search at GitHub's 100. Each question, asked of a fresh source,
    // sends the same requests to both, so what it costs is the size of its answer and never
    // the size of the board.
    let several_pages = || {
        let fixture = narrowing_board();
        for index in 0..350 {
            fixture.filed_by_something_else(
                Item::issue(&format!("I_filler_{index:03}"), &format!("filler {index}"))
                    .status("Todo")
                    .carrying(&format!("work:OTHER-{index}"))
                    .body(&slotted(
                        "unrelated prose",
                        &json!({"orchestrator.follow-up": {"root_cause": format!("cause-{index}")}}),
                    )),
            );
        }
        fixture
    };
    for (what, query, expected) in [
        (
            "text",
            TaskQuery {
                text: text("Ship it", TextFields::Title),
                ..TaskQuery::default()
            },
            vec!["I_ship"],
        ),
        (
            "metadata",
            metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache"),
            vec!["I_owned"],
        ),
        ("origin", origin_query("work:ENG-1"), vec!["I_copied"]),
    ] {
        let mut sent = Vec::new();
        for fixture in [narrowing_board(), several_pages()] {
            let source = source(&fixture);
            assert_eq!(
                selected_tasks(source.as_ref(), &query).await,
                expected,
                "{what}"
            );
            assert_eq!(fixture.requests("board"), 0, "{what} read the board");
            sent.push(fixture.operations());
        }
        assert!(!sent[0].is_empty(), "{what} sent nothing at all");
        assert_eq!(
            sent[0], sent[1],
            "{what} sent more to a board of several pages than to a board of one"
        );
    }

    // The unnarrowed read beside them does grow with the board, which is what makes the
    // equality above a property of the three questions rather than of this fixture.
    let unnarrowed = TaskQuery {
        statuses: vec![StatusCategory::Todo],
        ..TaskQuery::default()
    };
    let mut sent = Vec::new();
    for fixture in [narrowing_board(), several_pages()] {
        let source = source(&fixture);
        selected_tasks(source.as_ref(), &unnarrowed).await;
        sent.push(fixture.operations().len());
    }
    assert!(sent[1] > sent[0], "{sent:?}");
}

/// A short bound for the journey's own wait, so a wait that is not satisfied ends in
/// milliseconds; the journey itself waits [`journey::BOARD_WAIT`].
const SHORT_BOARD_WAIT: journey::BoardWait = journey::BoardWait {
    attempts: 3,
    interval: Duration::ZERO,
};

/// One canned HTTP failure that is not a rate limit, answered with `status`.
fn http_failure(status: &'static str) -> Refusal {
    Refusal {
        status,
        headers: String::new(),
        body: json!({"message": status}).to_string(),
    }
}

/// The credentialed journey's wait for a project to reach the board, run against this board:
/// a fresh source per attempt, as the journey builds one, asking for `I_plan` by its title.
async fn await_engine_project(fixture: &Fixture) -> Result<(), String> {
    let rebuilt = || source(fixture);
    journey::await_on_board(
        &rebuilt,
        &NativeId("I_plan".into()),
        ItemKind::Project,
        "engine",
        SHORT_BOARD_WAIT,
    )
    .await
}

#[tokio::test]
async fn the_journeys_board_wait_rides_out_an_unavailable_search_and_sees_the_item() {
    let fixture = board_with_documents();
    fixture.script(vec![http_failure("504 Gateway Timeout")]);

    let waited = await_engine_project(&fixture).await;

    assert_eq!(
        waited,
        Ok(()),
        "one gateway timeout is an attempt that saw nothing, not the end of the wait"
    );
    assert_eq!(
        fixture.requests("search"),
        2,
        "the timed-out search and the one that found the project"
    );
}

#[tokio::test]
async fn the_journeys_board_wait_fails_naming_the_latest_answer_when_github_stays_unavailable() {
    let fixture = board_with_documents();
    fixture.script(vec![
        http_failure("502 Bad Gateway"),
        http_failure("503 Service Unavailable"),
        http_failure("504 Gateway Timeout"),
    ]);

    let message = await_engine_project(&fixture)
        .await
        .expect_err("a board that never answers fails the journey");

    assert!(
        message.contains("waiting for a created project to reach the board failed")
            && message.contains("504 Gateway Timeout"),
        "it names the latest answer: {message}"
    );
    assert!(
        !message.contains("502") && !message.contains("503"),
        "and not an earlier one: {message}"
    );
    assert_eq!(
        fixture.requests("search"),
        3,
        "bounded by the wait's own attempts"
    );
}

#[tokio::test]
async fn the_journeys_board_wait_ends_at_once_on_an_error_that_is_not_unavailability() {
    let fixture = board_with_documents();
    fixture.script(vec![http_failure("401 Unauthorized")]);

    let message = await_engine_project(&fixture)
        .await
        .expect_err("a rejected credential is not something waiting fixes");

    assert!(
        message.contains("waiting for a created project to reach the board failed")
            && message.contains("rejected the configured credential"),
        "{message}"
    );
    assert_eq!(
        fixture.requests("search"),
        1,
        "and no second attempt is made, although the board would answer one"
    );
}

#[derive(Clone, Debug)]
enum TextSearch {
    Projects(ProjectQuery),
    Documents(DocumentQuery),
}

impl TextSearch {
    fn projects(terms: &str, fields: TextFields) -> Self {
        Self::Projects(ProjectQuery {
            text: text(terms, fields),
            ..ProjectQuery::default()
        })
    }

    fn documents(project: ProjectFilter, terms: &str, fields: TextFields) -> Self {
        Self::Documents(document_query(
            LabelFilter::default(),
            project,
            text(terms, fields),
        ))
    }

    async fn selected(&self, source: &dyn TaskSource) -> Vec<String> {
        match self {
            Self::Projects(query) => selected_projects(source, query).await,
            Self::Documents(query) => selected_documents(source, query).await,
        }
    }
}

/// Every project and unscoped document text search below, the matches each returned when it
/// read the whole board, and the qualifier it now sends instead. Each search's text is also
/// held by an item of the other kinds, so a candidate the search names is kept only once its
/// kind is confirmed.
fn text_searches() -> Vec<(&'static str, TextSearch, Vec<&'static str>, &'static str)> {
    vec![
        (
            "a project title",
            TextSearch::projects("engine", TextFields::Title),
            vec!["I_plan"],
            "in:title \"engine\"",
        ),
        (
            "a project title or content",
            TextSearch::projects("ENGINE", TextFields::TitleOrContent),
            vec!["I_plan"],
            "in:title,body \"ENGINE\"",
        ),
        (
            "a document title",
            TextSearch::documents(ProjectFilter::Any, "runbook", TextFields::Title),
            vec!["I_filed"],
            "in:title \"runbook\"",
        ),
        (
            "a document content",
            TextSearch::documents(ProjectFilter::Any, "engine core", TextFields::Content),
            vec!["I_design"],
            "in:body \"engine core\"",
        ),
        (
            "a document title or content",
            TextSearch::documents(
                ProjectFilter::Any,
                "alpha design",
                TextFields::TitleOrContent,
            ),
            vec!["I_design", "I_filed"],
            "in:title,body \"alpha design\"",
        ),
        (
            "an orphan document title or content",
            TextSearch::documents(
                ProjectFilter::Orphans,
                "alpha design",
                TextFields::TitleOrContent,
            ),
            vec!["I_design"],
            "in:title,body \"alpha design\"",
        ),
    ]
}

#[tokio::test]
async fn project_and_document_text_searches_ask_a_narrower_question_than_the_board() {
    for (what, search, expected, qualifier) in text_searches() {
        let fixture = board_with_documents();
        let source = source(&fixture);
        assert_eq!(search.selected(source.as_ref()).await, expected, "{what}");
        assert_eq!(fixture.requests("board"), 0, "{what} read the board");
        assert_eq!(
            fixture.board_item_reads(),
            Vec::<String>::new(),
            "{what} walked the board's items"
        );
        assert_eq!(
            fixture.searches(),
            [format!("project:octo-org/7 is:issue {qualifier}")],
            "{what} sent something other than one board-scoped search for its text"
        );
        // Asked again of the same source, the answer it holds for the command is the one
        // the search returned: nothing more is sent.
        let sent = fixture.operations();
        assert_eq!(search.selected(source.as_ref()).await, expected, "{what}");
        assert_eq!(fixture.operations(), sent, "{what} asked GitHub twice");
    }

    // A document read scoped to one project keeps its read of that project's sub-issues,
    // text or none, and sends no search.
    let fixture = board_with_documents();
    let source = source(&fixture);
    assert_eq!(
        TextSearch::documents(
            ProjectFilter::Is(NativeId("I_plan".to_owned())),
            "runbook",
            TextFields::Title,
        )
        .selected(source.as_ref())
        .await,
        ["I_filed"]
    );
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(fixture.requests("board"), 0);
}

async fn walk_text_search(source: &dyn TaskSource, search: &TextSearch, limit: u32) -> Vec<String> {
    let mut request = page(limit);
    let mut ids = Vec::new();
    loop {
        let (items, next) = match search {
            TextSearch::Projects(query) => {
                let answer = source.query_projects(query, &request).await.unwrap();
                (
                    answer.items.into_iter().map(|p| p.id.0).collect::<Vec<_>>(),
                    answer.next,
                )
            }
            TextSearch::Documents(query) => {
                let answer = source.query_documents(query, &request).await.unwrap();
                (
                    answer.items.into_iter().map(|d| d.id.0).collect(),
                    answer.next,
                )
            }
        };
        ids.extend(items);
        match next {
            Some(cursor) => request = resume(&cursor.0, limit),
            None => return ids,
        }
    }
}

#[tokio::test]
async fn project_and_document_text_searches_walk_every_page_of_matches_once() {
    // Seventy-five issues hold the text — a project, a document and a task of each number —
    // so the search answers four pages of twenty, and the caller walks the survivors ten at
    // a time across three of its own pages.
    let widgets = || {
        board(
            (0..25)
                .flat_map(|index| {
                    [
                        Item::issue(&format!("P_{index:02}"), &format!("widget plan {index}"))
                            .sub_issues(1),
                        design(&format!("D_{index:02}"), &format!("widget note {index}")),
                        Item::issue(&format!("I_{index:02}"), &format!("widget task {index}")),
                    ]
                })
                .map(|item| item.status("Todo"))
                .collect(),
        )
    };
    for (what, search, prefix) in [
        (
            "projects",
            TextSearch::projects("widget", TextFields::Title),
            "P",
        ),
        (
            "documents",
            TextSearch::documents(ProjectFilter::Any, "widget", TextFields::Title),
            "D",
        ),
    ] {
        let fixture = widgets();
        let source = source(&fixture);
        assert_eq!(
            walk_text_search(source.as_ref(), &search, 10).await,
            (0..25)
                .map(|index| format!("{prefix}_{index:02}"))
                .collect::<Vec<_>>(),
            "{what}: every match, once, in the board's order"
        );
        assert_eq!(
            search_sizes(&fixture),
            [20; 4],
            "{what}: the search is walked once to its end, whatever page the caller is on"
        );
        assert_eq!(fixture.requests("board"), 0, "{what} read the board");
    }
}

#[tokio::test]
async fn project_and_document_text_searches_answer_with_what_this_process_wrote() {
    let fixture = board(vec![
        Item::issue("P_old", "Ship plan")
            .status("Todo")
            .sub_issues(1),
        design("D_old", "Ship notes").status("Todo"),
    ]);
    // Every index this board keeps still answers both as they are now, after the writes below
    // retitle them out of the text.
    fixture.indexes_behind("P_old");
    fixture.indexes_behind("D_old");
    let source = source(&fixture);
    source
        .write_project(&ItemWrite {
            target: Some(NativeId("P_old".to_owned())),
            item: project("P_old", "Parked plan", status(StatusCategory::Todo, "Todo")),
            depends_on: vec![],
        })
        .await
        .expect("the project is retitled");
    source
        .write_document(&ItemWrite {
            target: Some(NativeId("D_old".to_owned())),
            item: document("D_old", "Parked notes"),
            depends_on: vec![],
        })
        .await
        .expect("the document is retitled");
    // And two the search cannot see yet, because they were created a moment ago.
    let fresh_project = source
        .write_project(&write(project(
            "ignored",
            "Fresh ship plan",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a project this board accepts");
    let fresh_document = source
        .write_document(&write(document("ignored", "Fresh ship notes")))
        .await
        .expect("a document this board accepts");
    fixture.read_behind(2);

    for (what, search, expected) in [
        (
            "projects holding the old title",
            TextSearch::projects("ship", TextFields::Title),
            vec![fresh_project.0.clone()],
        ),
        (
            "projects holding the new title",
            TextSearch::projects("parked", TextFields::Title),
            vec!["P_old".to_owned()],
        ),
        (
            "documents holding the old title",
            TextSearch::documents(ProjectFilter::Any, "ship", TextFields::Title),
            vec![fresh_document.0.clone()],
        ),
        (
            "documents holding the new title",
            TextSearch::documents(ProjectFilter::Orphans, "parked", TextFields::Title),
            vec!["D_old".to_owned()],
        ),
    ] {
        assert_eq!(search.selected(source.as_ref()).await, expected, "{what}");
    }
    assert!(
        fixture
            .searches()
            .iter()
            .all(|search| search.contains("in:title")),
        "every answer came from a narrowed search: {:?}",
        fixture.searches()
    );
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn project_and_document_text_searches_narrow_by_whole_words_and_confirm_by_substring() {
    let fixture = board(vec![
        Item::issue("P_ship", "Ship it")
            .status("Todo")
            .sub_issues(1),
        Item::issue("P_shipment", "Shipment plan")
            .status("Todo")
            .sub_issues(1),
        design("D_ship", "Ship notes"),
        design("D_shipment", "Shipment notes"),
        Item::draft("D_draft", &format!("{DESIGN_TITLE_PREFIX}Ship draft")).status("Todo"),
    ]);
    let source = source(&fixture);
    // The draft is a document of this board, read off its items connection.
    assert_eq!(
        selected_documents(source.as_ref(), &DocumentQuery::default()).await,
        ["D_ship", "D_shipment", "D_draft"]
    );
    for (what, search, expected) in [
        // `Shipment` holds "ship" as a substring and not as a word, so GitHub's token match
        // does not name it: the narrowing a task text search already declares.
        (
            "a project word",
            TextSearch::projects("ship", TextFields::Title),
            vec!["P_ship"],
        ),
        // GitHub reads `ship-it` as the words `ship it`, which `Ship it` holds; the text
        // `ship-it` is in no title, so the substring rule turns the candidate away.
        (
            "a project candidate the substring rule refuses",
            TextSearch::projects("ship-it", TextFields::Title),
            vec![],
        ),
        // A board draft is not an issue, so no search lists it, whatever its title holds.
        (
            "a document word",
            TextSearch::documents(ProjectFilter::Any, "ship", TextFields::Title),
            vec!["D_ship"],
        ),
        (
            "a document candidate the substring rule refuses",
            TextSearch::documents(ProjectFilter::Orphans, "ship-notes", TextFields::Title),
            vec![],
        ),
    ] {
        assert_eq!(search.selected(source.as_ref()).await, expected, "{what}");
    }
}

#[tokio::test]
async fn project_and_document_text_searches_cost_the_same_on_a_board_of_several_pages() {
    // The one-page board is `board_with_documents`; the other is the same board with 350
    // more projects, documents and tasks that no search below matches — four pages of
    // `ProjectV2.items` and of an unqualified board search at GitHub's 100.
    let several_pages = || {
        let fixture = board_with_documents();
        for index in 0..350 {
            let item = match index % 3 {
                0 => Item::issue(
                    &format!("P_filler_{index:03}"),
                    &format!("filler plan {index}"),
                )
                .sub_issues(1),
                1 => design(
                    &format!("D_filler_{index:03}"),
                    &format!("filler note {index}"),
                ),
                _ => Item::issue(&format!("I_filler_{index:03}"), &format!("filler {index}")),
            };
            fixture.filed_by_something_else(item.status("Todo").body("unrelated prose"));
        }
        fixture
    };
    for (what, search, expected, _) in text_searches() {
        let mut sent = Vec::new();
        for fixture in [board_with_documents(), several_pages()] {
            let source = source(&fixture);
            assert_eq!(search.selected(source.as_ref()).await, expected, "{what}");
            assert_eq!(fixture.requests("board"), 0, "{what} read the board");
            sent.push(fixture.operations());
        }
        assert!(!sent[0].is_empty(), "{what} sent nothing at all");
        assert_eq!(
            sent[0], sent[1],
            "{what} sent more to a board of several pages than to a board of one"
        );
    }

    // The same reads without a text do grow with the board, which is what makes the
    // equality above a property of the text searches rather than of this fixture.
    for (what, search) in [
        ("projects", TextSearch::Projects(ProjectQuery::default())),
        ("documents", TextSearch::Documents(DocumentQuery::default())),
    ] {
        let mut sent = Vec::new();
        for fixture in [board_with_documents(), several_pages()] {
            let source = source(&fixture);
            search.selected(source.as_ref()).await;
            sent.push(fixture.operations().len());
        }
        assert!(sent[1] > sent[0], "{what}: {sent:?}");
    }
}

#[tokio::test]
async fn an_item_this_process_wrote_out_of_a_predicate_is_not_returned_from_a_stale_index() {
    let fixture = board(vec![
        Item::issue("I_moved", "Ship it")
            .status("Todo")
            .carrying("work:ENG-1")
            .body(&slotted(
                "prose",
                &json!({"onetaskgraph.origin": "work:ENG-1", "team.owner": "ada"}),
            )),
    ]);
    // Every index this board keeps still answers the item as it is now, after the write below.
    fixture.indexes_behind("I_moved");
    let source = source(&fixture);
    let mut moved = task("I_moved", "Parked", status(StatusCategory::Todo, "Todo"));
    moved.metadata = BTreeMap::from([
        ("onetaskgraph.origin".to_owned(), json!("work:ENG-2")),
        ("team.owner".to_owned(), json!("bob")),
    ]);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("I_moved".to_owned())),
            item: moved,
            depends_on: vec![],
        })
        .await
        .expect("the update lands");
    assert_eq!(fixture.item("I_moved").title, "Parked");

    let titled = |terms: &str| TaskQuery {
        text: text(terms, TextFields::Title),
        ..TaskQuery::default()
    };
    let none = Vec::<String>::new();
    for (query, expected) in [
        (titled("ship"), none.clone()),
        (metadata_query("team.owner", &[], "ada"), none.clone()),
        (origin_query("work:ENG-1"), none.clone()),
        (titled("parked"), vec!["I_moved".to_owned()]),
        (
            metadata_query("team.owner", &[], "bob"),
            vec!["I_moved".to_owned()],
        ),
        (origin_query("work:ENG-2"), vec!["I_moved".to_owned()]),
    ] {
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            expected,
            "{query:?}"
        );
    }
    // A source that did not write it still reads the index's answer, which is what makes the
    // answers above this process's own record rather than GitHub's.
    let stranger = self::source(&fixture);
    assert_eq!(
        selected_tasks(stranger.as_ref(), &origin_query("work:ENG-1")).await,
        ["I_moved"]
    );
}

#[tokio::test]
async fn an_origin_lookup_walks_each_of_its_connections_past_its_first_page() {
    // More carriers than one page of either connection holds: five in the board field alone,
    // which only the field filter finds, and four this release wrote, which both find. Both
    // connections have to be walked on from their own cursors for all nine to come back.
    let mut items = (0..5)
        .map(|index| {
            Item::issue(&format!("I_field_{index}"), "field alone")
                .status("Todo")
                .carrying("work:ENG-1")
        })
        .collect::<Vec<_>>();
    items.extend((0..4).map(|index| {
        Item::issue(&format!("I_mirrored_{index}"), "mirrored")
            .status("Todo")
            .carrying("work:ENG-1")
            .body(&slotted("", &json!({"onetaskgraph.origin": "work:ENG-1"})))
    }));
    items.push(
        Item::issue("I_other", "other")
            .status("Todo")
            .carrying("work:ENG-2"),
    );
    let fixture = board(items);
    let source = source(&fixture);
    let mut found = selected_tasks(source.as_ref(), &origin_query("work:ENG-1")).await;
    found.sort();
    let mut expected = (0..5)
        .map(|index| format!("I_field_{index}"))
        .chain((0..4).map(|index| format!("I_mirrored_{index}")))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(found, expected);
    // Nine carriers at three a page is three pages of the field filter; the four the search
    // finds are two pages of it, walked alongside.
    assert_eq!(fixture.requests("originItems"), 3);
    for (operation, variables) in &fixture.state.lock().unwrap().bindings {
        if operation == "originItems" {
            assert_eq!(variables["originFirst"], json!(3));
        }
    }
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn an_origin_lookup_keeps_its_page_of_three_whatever_the_answer_holds() {
    // The bounded narrowing searches size their pages by the rows a caller needs; the origin
    // lookup already costs one point a page and keeps its own size, so the number of pages it
    // sends grows with the carriers alone.
    for count in [0_usize, 3, 21, 130] {
        let mut items = (0..count)
            .map(|index| {
                Item::issue(&format!("I_{index:03}"), "carrier")
                    .status("Todo")
                    .carrying("work:ENG-1")
            })
            .collect::<Vec<_>>();
        items.push(
            Item::issue("I_other", "other")
                .status("Todo")
                .carrying("work:ENG-2"),
        );
        let fixture = board(items);
        let mut found =
            walk_tasks(source(&fixture).as_ref(), &origin_query("work:ENG-1"), 100).await;
        found.sort();
        assert_eq!(
            found,
            (0..count)
                .map(|index| format!("I_{index:03}"))
                .collect::<Vec<_>>(),
            "{count}"
        );
        let firsts = fixture
            .state
            .lock()
            .unwrap()
            .bindings
            .iter()
            .filter(|(operation, _)| operation == "originItems")
            .map(|(_, variables)| variables["originFirst"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(firsts, vec![3; count.div_ceil(3).max(1)], "{count}");
        assert_eq!(fixture.requests("originItems"), count.div_ceil(3).max(1));
        assert_eq!(fixture.requests("search"), 0, "{count}");
        assert_eq!(fixture.requests("board"), 0, "{count}");
    }
}

#[tokio::test]
async fn an_origin_lookup_answer_this_source_cannot_read_is_refused_by_what_is_wrong() {
    let empty = |more: bool, cursor: Value| json!({"nodes":[],"pageInfo":{"hasNextPage":more,"endCursor":cursor}});
    let answer = |items: Value, search: Value| json!({"data":{"originItems":{"projectV2":{"items":items}},"search":search}});
    for (what, bodies, expected) in [
        (
            "a board the token cannot see",
            vec![json!({"data":{"originItems":{"projectV2":null},
                                "search":empty(false, Value::Null)}})],
            "was not found or is not visible to the token",
        ),
        (
            "no search connection",
            vec![json!({"data":{"originItems":{"projectV2":{"items":empty(false, Value::Null)}}}})],
            "has no search connection",
        ),
        (
            "a connection without pageInfo",
            vec![answer(json!({"nodes":[]}), empty(false, Value::Null))],
            "connection has no pageInfo",
        ),
        (
            "another page and no cursor to it",
            vec![answer(empty(true, Value::Null), empty(false, Value::Null))],
            "reports another page and no endCursor",
        ),
        (
            "a cursor that does not advance",
            vec![
                answer(empty(true, json!("c1")), empty(false, Value::Null)),
                answer(empty(true, json!("c1")), empty(false, Value::Null)),
            ],
            "cursor is empty or did not advance",
        ),
    ] {
        let endpoint = sequence_server(bodies);
        let message = refusal(
            configured(&endpoint, json!({}))
                .query_tasks(&origin_query("work:ENG-1"), &page(10))
                .await
                .expect_err(what),
        );
        assert!(message.contains(expected), "{what}: {message}");
    }
}

#[tokio::test]
async fn a_title_search_with_metadata_searches_both_fields_and_confirms_each_predicate() {
    let fixture = board(vec![
        Item::issue("I_both", "Ship it")
            .status("Todo")
            .body(&slotted(
                "prose",
                &json!({"orchestrator.follow-up": {"root_cause": "stale-cache"}}),
            )),
        // The title's words are in the body and the value is in the slot: GitHub finds it,
        // because the phrases are searched in both fields, and the title rule refuses it.
        Item::issue("I_body_title", "Parked")
            .status("Todo")
            .body(&slotted(
                "ship it later",
                &json!({"orchestrator.follow-up": {"root_cause": "stale-cache"}}),
            )),
        Item::issue("I_title_only", "Ship it too").status("Todo"),
    ]);
    let source = source(&fixture);
    let mut query = metadata_query("orchestrator.follow-up", &["root_cause"], "stale-cache");
    query.text = text("ship it", TextFields::Title);
    assert_eq!(selected_tasks(source.as_ref(), &query).await, ["I_both"]);
    assert_eq!(
        fixture.searches(),
        ["project:octo-org/7 is:issue in:title,body \"ship it\" \"stale-cache\""]
    );
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn comment_activity_with_a_narrowed_search_asks_for_both_qualifiers_at_once() {
    let fixture = board(vec![
        Item::issue("I_tagged_fresh", "Tagged fresh")
            .status("Todo")
            .updated("2026-09-21T09:00:00Z")
            .body(&slotted("", &json!({"team.owner": "ada"}))),
        Item::issue("I_tagged_stale", "Tagged stale")
            .status("Todo")
            .updated("2026-06-02T09:00:00Z")
            .body(&slotted("", &json!({"team.owner": "ada"}))),
        Item::issue("I_untagged_fresh", "Untagged fresh")
            .status("Todo")
            .updated("2026-09-22T09:00:00Z"),
    ]);
    fixture.commented_at(
        "I_tagged_fresh",
        "2026-09-21T09:00:00Z",
        "2026-09-21T09:00:00Z",
    );
    fixture.commented_at("I_tagged_stale", LONG_BEFORE, "2026-06-02T09:00:00Z");
    fixture.commented_at(
        "I_untagged_fresh",
        "2026-09-22T09:00:00Z",
        "2026-09-22T09:00:00Z",
    );
    let source = source(&fixture);
    let mut query = metadata_query("team.owner", &[], "ada");
    query.commented_since = Some(COMMENTED_SINCE.parse().expect("an RFC 3339 instant"));
    assert_eq!(
        selected_tasks(source.as_ref(), &query).await,
        ["I_tagged_fresh"]
    );
    let searches = fixture.searches();
    assert_eq!(searches.len(), 1, "{searches:?}");
    assert!(
        searches[0].starts_with("project:octo-org/7 is:issue updated:>=")
            && searches[0].ends_with(" in:body \"ada\""),
        "one search carrying both qualifiers: {searches:?}"
    );
    assert_eq!(
        fixture.comment_reads(),
        ["I_tagged_fresh"],
        "only the candidate both qualifiers kept has its comments read"
    );
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn a_metadata_value_json_escapes_is_found_by_the_escape_the_body_holds() {
    // The slot is JSON, so a newline, a tab and a quote are held as their escapes, and GitHub
    // reads `\n` beside a word as part of that word. Searching for the raw characters would
    // ask for different words than the body holds and miss the item.
    let values = [
        "line one\nline two",
        "column\tvalue",
        "said \"stale\" twice",
    ];
    let fixture = board(
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Item::issue(&format!("I_{index}"), "escaped")
                    .status("Todo")
                    .body(&slotted("", &json!({"team.note": value})))
            })
            .collect(),
    );
    let source = source(&fixture);
    for (index, value) in values.iter().enumerate() {
        assert_eq!(
            selected_tasks(source.as_ref(), &metadata_query("team.note", &[], value)).await,
            [format!("I_{index}")],
            "{value:?}"
        );
    }
}

#[tokio::test]
async fn a_project_scoped_query_confirms_metadata_and_origin_over_that_projects_own_tasks() {
    // A read narrowed to one project asks that project for its own tasks, whatever else the
    // query carries, and confirms the metadata and origin predicates over them in process:
    // no issue search, no origin lookup and no board read, and nothing outside the project,
    // however well it matches.
    let fixture = board(vec![
        Item::issue("I_p1", "Plan")
            .status("Todo")
            .sub_issues(3)
            .body("<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->"),
        Item::issue("I_tagged", "tagged")
            .status("Todo")
            .parent("I_p1")
            .carrying("work:ENG-1")
            .body(&slotted("", &json!({"team.owner": "ada"}))),
        Item::issue("I_other", "other")
            .status("Todo")
            .parent("I_p1")
            .carrying("work:ENG-10")
            .body(&slotted("", &json!({"team.owner": "bob"}))),
        Item::issue("I_plain", "plain")
            .status("Todo")
            .parent("I_p1"),
        Item::issue("I_elsewhere", "elsewhere")
            .status("Todo")
            .carrying("work:ENG-1")
            .body(&slotted("", &json!({"team.owner": "ada"}))),
    ]);
    let source = source(&fixture);
    let in_plan = |mut query: TaskQuery| {
        query.project = ProjectFilter::Is(NativeId("I_p1".to_owned()));
        query
    };
    let mut both = metadata_query("team.owner", &[], "ada");
    both.origin = Some("work:ENG-1".to_owned());
    let mut crossed = metadata_query("team.owner", &[], "ada");
    crossed.origin = Some("work:ENG-10".to_owned());
    for (what, query, expected) in [
        (
            "metadata",
            metadata_query("team.owner", &[], "ada"),
            vec!["I_tagged"],
        ),
        ("origin", origin_query("work:ENG-1"), vec!["I_tagged"]),
        (
            "an origin a suffix longer",
            origin_query("work:ENG-10"),
            vec!["I_other"],
        ),
        ("both, held by one task", both, vec!["I_tagged"]),
        ("both, held by different tasks", crossed, vec![]),
    ] {
        assert_eq!(
            selected_tasks(source.as_ref(), &in_plan(query)).await,
            expected,
            "{what}"
        );
    }
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(fixture.origin_filters(), Vec::<String>::new());
    assert_eq!(fixture.requests("board"), 0);
}

#[tokio::test]
async fn a_value_or_text_with_no_searchable_words_is_refused_before_any_request() {
    // GitHub's index holds words, so an empty value, one of whitespace or punctuation alone,
    // and a text of punctuation alone name none to find, and no bounded query answers them.
    // The source says so, naming what it was asked, rather than reading the whole board or
    // answering nothing.
    let mut beside_a_text = metadata_query("team.note", &["kind"], "---");
    beside_a_text.text = text("divider", TextFields::Title);
    let cases = [
        (
            "an empty value",
            metadata_query("team.note", &[], ""),
            "the metadata value \"\" at \"team.note\"",
        ),
        (
            "a whitespace value",
            metadata_query("team.note", &[], "  "),
            "the metadata value \"  \" at \"team.note\"",
        ),
        (
            "a punctuation value beside a text with words",
            beside_a_text,
            "the metadata value \"---\" at \"team.note/kind\"",
        ),
        (
            "a punctuation text",
            TaskQuery {
                text: text("--", TextFields::Title),
                ..TaskQuery::default()
            },
            "the text \"--\"",
        ),
    ];
    for (what, query, named) in cases {
        let fixture = board(vec![
            Item::issue("I_empty", "--")
                .status("Todo")
                .body(&slotted("", &json!({"team.note": ""}))),
        ]);
        let source = source(&fixture);
        let error = source.query_tasks(&query, &page(10)).await.expect_err(what);
        assert!(
            matches!(error, SourceError::Refused { .. }),
            "{what}: {error:?}"
        );
        let message = refusal(error);
        assert!(message.contains(named), "{what}: {message}");
        assert!(
            message.contains("GitHub's issue search indexes words")
                && message.contains("holding a letter or a digit"),
            "{what} says why and what to ask instead: {message}"
        );
        assert_eq!(
            fixture.operations(),
            Vec::<String>::new(),
            "{what} asked GitHub something"
        );
    }

    // A blank text is not refused: it keeps the board read it always had, confirmed by the
    // same substring rule.
    let fixture = board(vec![
        Item::issue("I_gap", "a   gap").status("Todo"),
        Item::issue("I_two", "Two").status("Todo"),
    ]);
    let source = source(&fixture);
    assert_eq!(
        selected_tasks(
            source.as_ref(),
            &TaskQuery {
                text: text("   ", TextFields::Title),
                ..TaskQuery::default()
            }
        )
        .await,
        ["I_gap"]
    );
    assert_eq!(fixture.requests("board"), 1);
    assert_eq!(fixture.searches(), ["project:octo-org/7 is:issue"]);
}

#[tokio::test]
async fn a_project_or_document_text_with_no_searchable_words_is_refused_before_any_request() {
    // The same refusal a task text search gives, for the same reason: the search these reads
    // send could not find the text, and the source does not read the board for it instead.
    for (what, search) in [
        ("projects", TextSearch::projects("--", TextFields::Title)),
        (
            "documents",
            TextSearch::documents(ProjectFilter::Any, "--", TextFields::TitleOrContent),
        ),
        (
            "orphan documents",
            TextSearch::documents(ProjectFilter::Orphans, "?!", TextFields::Content),
        ),
    ] {
        let fixture = board_with_documents();
        let source = source(&fixture);
        let error = match &search {
            TextSearch::Projects(query) => source
                .query_projects(query, &page(10))
                .await
                .map(|_| ())
                .expect_err(what),
            TextSearch::Documents(query) => source
                .query_documents(query, &page(10))
                .await
                .map(|_| ())
                .expect_err(what),
        };
        assert!(
            matches!(error, SourceError::Refused { .. }),
            "{what}: {error:?}"
        );
        let message = refusal(error);
        assert!(
            message.contains("cannot search for the text")
                && message.contains("GitHub's issue search indexes words")
                && message.contains("holding a letter or a digit"),
            "{what} says what, why and what to ask instead: {message}"
        );
        assert_eq!(
            fixture.operations(),
            Vec::<String>::new(),
            "{what} asked GitHub something"
        );
    }

    // A document read scoped to one project sends no search, so nothing about GitHub's index
    // bounds it: it answers such a text over that project's sub-issues, inside words, as it
    // always did.
    let fixture = board(vec![
        Item::issue("I_plan", "Engine").sub_issues(2),
        design("I_dashed", "Runbook -- v2").parent("I_plan"),
        design("I_plain", "Runbook").parent("I_plan"),
    ]);
    let scoped = source(&fixture);
    assert_eq!(
        TextSearch::documents(
            ProjectFilter::Is(NativeId("I_plan".to_owned())),
            "--",
            TextFields::Title,
        )
        .selected(scoped.as_ref())
        .await,
        ["I_dashed"]
    );
    assert_eq!(fixture.searches(), Vec::<String>::new());
    assert_eq!(fixture.requests("board"), 0);

    // A blank text is not refused: it keeps the read it always had.
    let fixture = board_with_documents();
    let projects = source(&fixture);
    assert_eq!(
        TextSearch::projects("   ", TextFields::Title)
            .selected(projects.as_ref())
            .await,
        Vec::<String>::new()
    );
    assert_eq!(fixture.searches(), ["project:octo-org/7 is:issue"]);
    let fixture = board_with_documents();
    let documents = source(&fixture);
    TextSearch::documents(ProjectFilter::Any, "   ", TextFields::Title)
        .selected(documents.as_ref())
        .await;
    assert_eq!(fixture.requests("board"), 1);
}

#[tokio::test]
async fn a_draft_this_process_wrote_is_not_an_answer_to_a_narrowed_read() {
    let fixture = board(vec![Item::draft("D_1", "Draft").status("Todo")]);
    let source = source(&fixture);
    let mut written = task("D_1", "Ship it", status(StatusCategory::Todo, "Todo"));
    written.metadata = BTreeMap::from([
        ("onetaskgraph.origin".to_owned(), json!("work:ENG-3")),
        ("team.owner".to_owned(), json!("ada")),
    ]);
    source
        .write_task(&ItemWrite {
            target: Some(NativeId("D_1".to_owned())),
            item: written,
            depends_on: vec![],
        })
        .await
        .expect("the draft is updated");
    for query in [
        TaskQuery {
            text: text("ship", TextFields::Title),
            ..TaskQuery::default()
        },
        metadata_query("team.owner", &[], "ada"),
        origin_query("work:ENG-3"),
    ] {
        assert_eq!(
            selected_tasks(source.as_ref(), &query).await,
            Vec::<String>::new(),
            "a board draft is not an issue, so no narrowed read returns one: {query:?}"
        );
    }
    // It is still the board's, and still read by its id.
    assert_eq!(
        source
            .get_task(&NativeId("D_1".to_owned()))
            .await
            .expect("a read by id")
            .map(|task| task.title),
        Some("Ship it".to_owned())
    );
}

#[tokio::test]
async fn a_carrier_filed_by_something_else_is_seen_by_the_next_source_and_not_by_this_one() {
    // The narrowed counterpart of the board read's own bargain, pinned beside it for the same
    // reason: a source keeps each narrowed answer for as long as it lives, completed with its
    // own writes every time, so what another process files after the first ask is not an
    // answer this source gives — and one invocation of the binary is one source. A source
    // built the way the next command builds one asks again and sees it once GitHub's index
    // has it.
    let fixture = board(vec![
        Item::issue("I_first", "first")
            .status("Todo")
            .carrying("work:ENG-5")
            .body(&slotted("", &json!({"onetaskgraph.origin": "work:ENG-5"}))),
    ]);
    let held = source(&fixture);
    assert_eq!(
        selected_tasks(held.as_ref(), &origin_query("work:ENG-5")).await,
        ["I_first"]
    );
    fixture.filed_by_something_else(
        Item::issue("I_later", "later")
            .status("Todo")
            .carrying("work:ENG-5")
            .body(&slotted("", &json!({"onetaskgraph.origin": "work:ENG-5"}))),
    );
    assert_eq!(
        selected_tasks(held.as_ref(), &origin_query("work:ENG-5")).await,
        ["I_first"],
        "this source asked a second time, which is the request its one answer buys"
    );
    assert_eq!(fixture.requests("originItems"), 1);

    let next = source(&fixture);
    let mut found = selected_tasks(next.as_ref(), &origin_query("work:ENG-5")).await;
    found.sort();
    assert_eq!(found, ["I_first", "I_later"]);
    assert_eq!(fixture.requests("originItems"), 2);
}

#[test]
fn the_published_paging_contract_matches_its_constants_and_both_documents() {
    fn contract(document: &str) -> &str {
        document
            .split_once("<!-- github-search-paging:start -->")
            .unwrap()
            .1
            .split_once("<!-- github-search-paging:end -->")
            .unwrap()
            .0
            .trim()
    }
    let source_docs = include_str!("../src/lib.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("//! "))
        .collect::<Vec<_>>()
        .join("\n");
    let protocol = include_str!("../../../docs/plugin-protocol.md");
    assert_eq!(contract(&source_docs), contract(protocol));
    let stated = contract(protocol);
    for clause in [
        format!(
            "every\npage at `first = {}` (SEARCH_PAGE_SIZE)",
            onetaskgraph_github_projects::SEARCH_PAGE_SIZE
        ),
        format!(
            "version-{} source cursor",
            onetaskgraph_github_projects::SEARCH_CURSOR_VERSION
        ),
    ] {
        assert!(
            stated.contains(&clause),
            "published contract is missing {clause}"
        );
    }
    let golden: Value =
        serde_json::from_str(include_str!("fixtures/search-cursor-v4.json")).unwrap();
    assert_eq!(
        golden["version"],
        onetaskgraph_github_projects::SEARCH_CURSOR_VERSION
    );
}

#[tokio::test]
async fn inconsistent_search_cursor_states_are_refused_without_a_request() {
    let fixture = narrowing_board();
    let query = TaskQuery {
        text: text("ship", TextFields::Title),
        ..TaskQuery::default()
    };
    for (connection, offset) in [
        (json!({"state":"continuing","after":""}), 0),
        (json!({"state":"exhausted","after":"3"}), 0),
        (json!({"state":"initial"}), 0),
        // An exhausted connection has no page to be part of the way through.
        (json!({"state":"exhausted"}), 2),
        // No page is resumed a whole page or more into it: those rows were never handed out.
        (json!({"state":"initial"}), 20),
        (json!({"state":"continuing","after":"20"}), 20),
    ] {
        let token = json!({"version":onetaskgraph_github_projects::SEARCH_CURSOR_VERSION,"connection":connection,"offset":offset}).to_string();
        assert!(
            source(&fixture)
                .query_tasks(&query, &resume(&token, 3))
                .await
                .is_err()
        );
    }
    assert_eq!(fixture.requests("search"), 0);
}

#[tokio::test]
async fn a_fresh_source_resumes_own_writes_missing_from_the_index() {
    let fixture = board(
        (0..3)
            .map(|index| Item::issue(&format!("I_{index}"), "widget").status("Todo"))
            .collect(),
    );
    let writer = source(&fixture);
    let mut created = Vec::new();
    for title in ["widget first", "widget second"] {
        created.push(
            writer
                .write_task(&write(task(
                    "ignored",
                    title,
                    status(StatusCategory::Todo, "Todo"),
                )))
                .await
                .unwrap(),
        );
    }
    fixture.read_behind(2);
    let query = TaskQuery {
        text: text("widget", TextFields::Title),
        ..TaskQuery::default()
    };
    let first = writer.query_tasks(&query, &page(4)).await.unwrap();
    assert_eq!(first.items.len(), 4);
    assert_eq!(first.items[3].id, created[0]);
    let before = fixture.documents().len();
    let fresh = source(&fixture);
    let rest = fresh
        .query_tasks(&query, &resume(&first.next.unwrap().0, 1))
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert_eq!(rest.items[0].id, created[1]);
    assert_eq!(rest.next, None);
    assert_eq!(
        &fixture.documents()[before..],
        &[onetaskgraph_github_projects::graphql::ISSUE]
    );
}

#[tokio::test]
async fn a_fresh_source_resumes_stale_written_rows_with_the_current_record() {
    for moved_out in [false, true] {
        let fixture = board(
            (0..3)
                .map(|index| Item::issue(&format!("I_{index}"), "widget old").status("Todo"))
                .collect(),
        );
        fixture.indexes_behind("I_2");
        let writer = source(&fixture);
        writer
            .write_task(&ItemWrite {
                target: Some(native("I_2")),
                item: task(
                    "I_2",
                    if moved_out { "Parked" } else { "widget fresh" },
                    status(StatusCategory::Todo, "Todo"),
                ),
                depends_on: vec![],
            })
            .await
            .unwrap();
        let query = TaskQuery {
            text: text("widget", TextFields::Title),
            ..TaskQuery::default()
        };
        let first = writer.query_tasks(&query, &page(1)).await.unwrap();
        assert_eq!(first.items[0].id, native("I_0"));
        let fresh = source(&fixture);
        let rest = fresh
            .query_tasks(&query, &resume(&first.next.unwrap().0, 100))
            .await
            .unwrap();
        assert_eq!(rest.items.len(), if moved_out { 1 } else { 2 });
        assert_eq!(rest.items[0].id, native("I_1"));
        if !moved_out {
            assert_eq!(rest.items[1].title, "widget fresh");
        }
        assert_eq!(rest.next, None);
    }
}

#[tokio::test]
async fn every_batched_field_answer_is_validated_and_a_failed_write_can_retry() {
    for alias in [
        "updateProjectV2ItemFieldValue",
        "second",
        "third",
        "cleared",
    ] {
        for response in [Value::Null, json!({"projectV2Item":{"id":"wrong"}})] {
            let mut fields = usable_fields();
            let priority_field = json!({"__typename":"ProjectV2SingleSelectField","id":"FIELD_priority","name":"Priority","options":[{"id":"urgent","name":"Urgent"}]});
            fields["nodes"]
                .as_array_mut()
                .unwrap()
                .push(priority_field.clone());
            let mut held = held_issue("I_1", "one", None);
            if alias == "cleared" {
                held["data"]["node"]["projectItems"]["nodes"][0]["fieldValues"]["nodes"] =
                    json!([{"name":"Urgent","field":priority_field}]);
            }
            let mut answer = json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}},"second":{"projectV2Item":{"id":"PVTI_1"}},"third":{"projectV2Item":{"id":"PVTI_1"}},"cleared":{"projectV2Item":{"id":"PVTI_1"}}});
            answer[alias] = response;
            // The board fields go first, and a refusal of any of them is reached before the
            // item's content is sent; the origin those fields carried is then put back.
            let endpoint = sequence_server(vec![
                held,
                fields_json("PVT_board", fields),
                json!({"data":answer}),
                json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}}),
            ]);
            let mut item = task("I_1", "revised", status(StatusCategory::Todo, "Todo"));
            if alias == "third" {
                item.priority = Priority::Urgent;
            }
            item.metadata
                .insert("onetaskgraph.origin".into(), json!("notes:T-1"));
            item.repositories =
                vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
            let error = configured(&endpoint, json!({"priority_mapping":{}}))
                .write_task(&ItemWrite {
                    target: Some(native("I_1")),
                    item,
                    depends_on: vec![],
                })
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains(&format!("field update {alias}")),
                "{alias}: {error}"
            );
        }
    }
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    let source = source(&fixture);
    source.get_task(&native("I_1")).await.unwrap();
    let mut item = task(
        "I_1",
        "revised",
        status(StatusCategory::InProgress, "In Progress"),
    );
    item.metadata
        .insert("onetaskgraph.origin".into(), json!("notes:T-1"));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let write = ItemWrite {
        target: Some(native("I_1")),
        item,
        depends_on: vec![],
    };
    fixture.refuse_after("updateProjectV2ItemFieldValue", 0);
    assert!(source.write_task(&write).await.is_err());
    fixture
        .state
        .lock()
        .unwrap()
        .refuse_after
        .remove("updateProjectV2ItemFieldValue");
    source.write_task(&write).await.unwrap();
    assert_eq!(
        fixture.requests("issue"),
        2,
        "the failed mutation invalidates the original binding"
    );
    assert_eq!(fixture.item("I_1").origin.as_deref(), Some("notes:T-1"));
    assert_eq!(fixture.item("I_1").status.as_deref(), Some("In Progress"));
}

#[tokio::test]
async fn a_status_already_held_reuses_its_record_and_sends_no_mutation() {
    let fixture = board(vec![Item::issue("I_1", "one").status("Todo")]);
    let source = source(&fixture);
    for _ in 0..2 {
        let result = source
            .set_task_status(&native("I_1"), StatusCategory::Todo)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, status(StatusCategory::Todo, "Todo"));
    }
    assert_eq!(fixture.requests("issue"), 1);
    assert_eq!(fixture.requests("boardFields"), 0);
    assert!(fixture.seen().is_empty());
}

#[tokio::test]
async fn a_write_after_a_stale_search_preserves_this_processs_newer_metadata() {
    let fixture = board(vec![Item::issue("I_1", "widget").status("Todo")]);
    fixture.indexes_behind("I_1");
    let source = source(&fixture);
    source
        .set_task_metadata(
            &native("I_1"),
            &MetadataKey::try_from("team.keep".to_owned()).unwrap(),
            &json!("new"),
        )
        .await
        .unwrap();
    let query = TaskQuery {
        text: text("widget", TextFields::Title),
        ..TaskQuery::default()
    };
    let answer = source.query_tasks(&query, &page(1)).await.unwrap();
    assert_eq!(answer.items[0].metadata["team.keep"], "new");
    source
        .set_task_metadata(
            &native("I_1"),
            &MetadataKey::try_from("team.next".to_owned()).unwrap(),
            &json!("also new"),
        )
        .await
        .unwrap();
    let held = fixture.item("I_1");
    let slot = raw_slot(held.body.as_deref().unwrap());
    assert_eq!(slot["team.keep"], "new");
    assert_eq!(slot["team.next"], "also new");
    assert_eq!(fixture.requests("issue"), 1);
}

/// A create reads the board's fields and the repository's id in one request, creates the
/// issue on no board and files it with `addProjectV2ItemById` — never through
/// `CreateIssueInput.projectV2Ids`, whose filing GitHub does not answer with the item and
/// after which the filing that has to follow is refused "Content already exists in this
/// project", as this board refuses it.
#[tokio::test]
async fn a_create_files_its_issue_on_the_board_with_one_filing_after_one_context_read() {
    let fixture = board(vec![]);
    let writer = source(&fixture);
    let created = writer
        .write_task(&write(task(
            "T-1",
            "a step",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a create");
    assert_eq!(
        fixture.operations(),
        [
            "boardFields",
            "createIssue",
            "addProjectV2ItemById",
            "updateProjectV2ItemFieldValue"
        ],
        "{:#?}",
        fixture.documents()
    );
    assert_eq!(
        fixture.documents()[0],
        onetaskgraph_github_projects::graphql::CREATION_CONTEXT
    );
    let created_input = fixture
        .seen()
        .into_iter()
        .find(|call| call[0] == "createIssue")
        .expect("createIssue was sent");
    assert_eq!(created_input[1].get("projectV2Ids"), None);
    assert!(fixture.holds(&created.0), "the issue is on the board");
    assert_eq!(
        writer
            .get_task(&created)
            .await
            .unwrap()
            .map(|task| task.title),
        Some("a step".to_owned())
    );

    // A second create in the same command knows both halves already and reads neither.
    let before = fixture.operations().len();
    writer
        .write_task(&write(task(
            "T-2",
            "a second",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect("a second create");
    assert_eq!(
        fixture.operations()[before..],
        [
            "createIssue",
            "addProjectV2ItemById",
            "updateProjectV2ItemFieldValue"
        ]
    );

    // And a filing GitHub refuses then takes the issue back, so nothing is left on no board.
    let fixture = board(vec![]);
    fixture.refuse("addProjectV2ItemById");
    let refused = source(&fixture)
        .write_task(&write(task(
            "T-1",
            "a step",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .expect_err("a refused filing fails the write");
    assert!(refusal(refused).contains("addProjectV2ItemById"));
    assert_eq!(
        fixture.operations(),
        [
            "boardFields",
            "createIssue",
            "addProjectV2ItemById",
            "deleteIssue"
        ]
    );
}

/// A batch GitHub refuses because one id resolves to no node at all is read again one item
/// at a time, so that id answers as missing and the rest as themselves; any other refusal is
/// every id's answer.
#[tokio::test]
async fn a_batch_refused_for_an_id_naming_no_node_is_read_again_one_item_at_a_time() {
    let unresolvable = json!({"errors":[{
        "message":"Could not resolve to a node with the global id of 'garbage'"}]});
    let no_comments = json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    let endpoint = sequence_server(vec![
        unresolvable.clone(),
        issue_with_comments("I_1", no_comments),
        unresolvable,
    ]);
    let read = configured(&endpoint, json!({}))
        .get_task_details(&[native("I_1"), native("garbage")], Some(&page(50)))
        .await;
    assert_eq!(read.len(), 2);
    match &read[0] {
        Ok(Some(detail)) => {
            assert_eq!(detail.task.id, native("I_1"));
            assert!(
                matches!(&detail.comments, Some(Ok(Some(comments))) if comments.items.is_empty()),
                "{detail:?}"
            );
        }
        other => panic!("I_1 is read on its own: {other:?}"),
    }
    assert!(matches!(read[1], Ok(None)), "{:?}", read[1]);

    let endpoint = sequence_server(vec![json!({"errors":[{
        "message":"Something went wrong while executing your query"}]})]);
    let read = configured(&endpoint, json!({}))
        .get_task_details(&[native("I_1"), native("I_2")], None)
        .await;
    assert_eq!(read.len(), 2);
    for answered in &read {
        assert!(
            matches!(answered, Err(SourceError::Refused { message })
                if message.contains("Something went wrong")),
            "{answered:?}"
        );
    }
}

/// A batch answer is held to the ids it was asked about: an alias left out, and an alias
/// answering about another issue, are each refused as malformed rather than read as missing
/// or reported under the id asked for.
#[tokio::test]
async fn a_batch_answer_missing_an_item_or_naming_another_is_refused_as_malformed() {
    let other = issue_read("I_2")["data"]["node"].clone();
    let endpoint = sequence_server(vec![json!({"data":{"i0": other}})]);
    let read = configured(&endpoint, json!({}))
        .get_task_details(&[native("I_1"), native("I_3")], None)
        .await;
    assert!(
        matches!(&read[0], Err(SourceError::Malformed { message })
            if message.contains("answered the read of I_1 with issue I_2")),
        "{:?}",
        read[0]
    );
    assert!(
        matches!(&read[1], Err(SourceError::Malformed { message })
            if message.contains("no item for I_3")),
        "{:?}",
        read[1]
    );
    // And one read whose answer carries no node at all is refused, not read as missing.
    let endpoint = sequence_server(vec![json!({"data":{}})]);
    let read = configured(&endpoint, json!({}))
        .get_task_details(&[native("I_1")], Some(&page(50)))
        .await;
    assert!(
        matches!(&read[0], Err(SourceError::Malformed { message }) if message.contains("no node")),
        "{:?}",
        read[0]
    );
}

/// The origin field is the one piece of an existing item's metadata written before its body,
/// so a write refused after it moved puts it back: the item's metadata is as it stood.
#[tokio::test]
async fn a_write_refused_after_it_moved_the_origin_puts_the_origin_back() {
    let fixture = board(vec![
        Item::issue("I_1", "one")
            .status("Todo")
            .carrying("plans:OLD"),
    ]);
    fixture.refuse("updateIssue");
    let mut item = task("I_1", "one, revised", status(StatusCategory::Todo, "Todo"));
    item.metadata
        .insert("onetaskgraph.origin".to_owned(), json!("plans:NEW"));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let error = source(&fixture)
        .write_task(&ItemWrite {
            target: Some(native("I_1")),
            item,
            depends_on: vec![],
        })
        .await
        .expect_err("the content update is refused");
    assert!(refusal(error).contains("updateIssue is refused"));
    let origins = fixture
        .seen()
        .iter()
        .filter(|call| {
            call[0] == "updateProjectV2ItemFieldValue" && call[1]["fieldId"] == "FIELD_origin"
        })
        .map(|call| call[1]["value"]["text"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        [json!("plans:NEW"), json!("plans:OLD")],
        "the origin moved with the board fields, then was put back"
    );
    let held = fixture.item("I_1");
    assert_eq!(held.origin.as_deref(), Some("plans:OLD"));
    assert_eq!(held.title, "one");
}

/// When putting the origin back is refused as well, the write's own refusal is still what the
/// caller is told, and it says what was left behind: the one key that moved, what it holds,
/// and what it held — the body, and every key in it, as they stood.
#[tokio::test]
async fn a_write_whose_origin_cannot_be_put_back_says_which_key_it_left_moved() {
    let fixture = board(vec![
        Item::issue("I_1", "one")
            .body("as it stood")
            .status("Todo")
            .carrying("plans:OLD"),
    ]);
    fixture.refuse("updateIssue");
    // The batched field write moving the origin lands; the one putting it back is refused.
    fixture.refuse_after("updateProjectV2ItemFieldValue", 1);
    let mut item = task("I_1", "one, revised", status(StatusCategory::Todo, "Todo"));
    item.content = Some("a body that must not land".to_owned());
    item.metadata
        .insert("onetaskgraph.origin".to_owned(), json!("plans:NEW"));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let error = source(&fixture)
        .write_task(&ItemWrite {
            target: Some(native("I_1")),
            item,
            depends_on: vec![],
        })
        .await
        .expect_err("the content update is refused");
    // The kind is the content write's own, exactly as a refusal with nothing left behind has
    // it: a caller branching on the kind is not told the restore's failure instead.
    let alone = board(vec![
        Item::issue("I_1", "one")
            .body("as it stood")
            .status("Todo")
            .carrying("plans:OLD"),
    ]);
    alone.refuse("updateIssue");
    let mut lone = task("I_1", "one, revised", status(StatusCategory::Todo, "Todo"));
    lone.content = Some("a body that must not land".to_owned());
    lone.metadata
        .insert("onetaskgraph.origin".to_owned(), json!("plans:NEW"));
    lone.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let original = source(&alone)
        .write_task(&ItemWrite {
            target: Some(native("I_1")),
            item: lone,
            depends_on: vec![],
        })
        .await
        .expect_err("the content update is refused");
    assert_eq!(
        std::mem::discriminant(&error),
        std::mem::discriminant(&original),
        "{error:?} is not the kind of {original:?}"
    );
    let said = refusal(error);
    assert!(said.contains("updateIssue is refused"), "{said}");
    assert!(
        said.contains(
            "its onetaskgraph.origin was moved to \"plans:NEW\" before that and could not be put \
             back to \"plans:OLD\""
        ) && said.contains("item I_1 still holds \"plans:NEW\" there")
            && said.contains("next: set onetaskgraph.origin on it back to \"plans:OLD\""),
        "{said}"
    );
    let held = fixture.item("I_1");
    assert_eq!(held.origin.as_deref(), Some("plans:NEW"));
    assert_eq!(held.title, "one");
    assert_eq!(held.body.as_deref(), Some("as it stood"));
}

/// A content write that fails for a rate limit, an outage, a credential or an answer this
/// source cannot read keeps that kind — and a rate limit the wait GitHub asked for — when the
/// restore after it is refused too: a caller waiting out a limit is not told to stop instead,
/// and is still told which key was left moved.
#[tokio::test]
async fn a_double_refusal_keeps_the_content_writes_kind_and_wait() {
    for (refused, kind) in [
        (Refusal::secondary_forbidden().after(30), "rate limited"),
        (Refusal::unavailable(), "unavailable"),
        (
            Refusal {
                status: "401 Unauthorized",
                headers: String::new(),
                body: json!({"message": "Bad credentials"}).to_string(),
            },
            "auth",
        ),
        (
            Refusal {
                status: "200 OK",
                headers: String::new(),
                body: "not json".to_owned(),
            },
            "malformed",
        ),
    ] {
        let fixture = board(vec![
            Item::issue("I_1", "one")
                .body("as it stood")
                .status("Todo")
                .carrying("plans:OLD"),
        ]);
        fixture.script_for("updateIssue", vec![refused]);
        fixture.refuse_after("updateProjectV2ItemFieldValue", 1);
        let mut item = task("I_1", "one, revised", status(StatusCategory::Todo, "Todo"));
        item.content = Some("a body that must not land".to_owned());
        item.metadata
            .insert("onetaskgraph.origin".to_owned(), json!("plans:NEW"));
        item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
        let error = paced(&fixture.endpoint, no_waiting())
            .write_task(&ItemWrite {
                target: Some(native("I_1")),
                item,
                depends_on: vec![],
            })
            .await
            .expect_err("the content update is refused");
        let said = match (&error, kind) {
            (
                SourceError::RateLimited {
                    retry_after_seconds: Some(30),
                    message: Some(said),
                },
                "rate limited",
            )
            | (SourceError::Unavailable { message: said }, "unavailable")
            | (SourceError::Auth { message: said }, "auth")
            | (SourceError::Malformed { message: said }, "malformed") => said.clone(),
            _ => panic!("a {kind} content write was reported as {error:?}"),
        };
        assert!(
            said.contains("so item I_1 still holds \"plans:NEW\" there")
                && said.contains("next: set onetaskgraph.origin on it back to \"plans:OLD\""),
            "{kind}: {said}"
        );
        let held = fixture.item("I_1");
        assert_eq!(held.origin.as_deref(), Some("plans:NEW"), "{kind}");
        assert_eq!(held.body.as_deref(), Some("as it stood"), "{kind}");
    }
}

/// When the board-field write carrying the origin is itself refused, GitHub does not say which
/// of its fields ran, so a refused restore after it says the origin holds one of the two values
/// rather than claiming it moved — and the item's body is as it stood.
#[tokio::test]
async fn a_refused_field_write_whose_restore_is_refused_does_not_claim_the_origin_moved() {
    let fixture = board(vec![
        Item::issue("I_1", "one")
            .body("as it stood")
            .status("Todo")
            .carrying("plans:OLD"),
    ]);
    fixture.refuse("updateProjectV2ItemFieldValue");
    let mut item = task("I_1", "one, revised", status(StatusCategory::Todo, "Todo"));
    item.content = Some("a body that must not land".to_owned());
    item.metadata
        .insert("onetaskgraph.origin".to_owned(), json!("plans:NEW"));
    item.repositories = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    let error = source(&fixture)
        .write_task(&ItemWrite {
            target: Some(native("I_1")),
            item,
            depends_on: vec![],
        })
        .await
        .expect_err("the field write is refused");
    let SourceError::Refused { message: said } = &error else {
        panic!("a refused field write was reported as {error:?}");
    };
    assert!(
        said.contains("updateProjectV2ItemFieldValue is refused"),
        "{said}"
    );
    assert!(
        said.contains("GitHub does not say whether that part of it ran")
            && said.contains("item I_1 holds \"plans:NEW\" or \"plans:OLD\" there")
            && said.contains("next: set onetaskgraph.origin on it back to \"plans:OLD\""),
        "{said}"
    );
    assert!(!said.contains("still holds"), "{said}");
    assert_eq!(fixture.requests("updateIssue"), 0);
    let held = fixture.item("I_1");
    assert_eq!(held.origin.as_deref(), Some("plans:OLD"));
    assert_eq!(held.body.as_deref(), Some("as it stood"));
    assert_eq!(held.title, "one");
}

/// A clock that completes each wait in virtual time while the source drives real HTTP.
#[derive(Default)]
struct AdvancingClock {
    waits: Mutex<Vec<Duration>>,
}

impl onetaskgraph_plugin_api::Clock for AdvancingClock {
    fn now(&self) -> Duration {
        self.waits.lock().unwrap().iter().sum()
    }

    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.waits.lock().unwrap().push(duration);
        Box::pin(std::future::ready(()))
    }
}

#[tokio::test]
async fn the_supplied_clock_spaces_writes_and_bounds_rate_limit_recovery() {
    let fixture = board(vec![]);
    let clock = Arc::new(AdvancingClock::default());
    let source = Plugin
        .build_with_clock(
            &SourceName::new("work").unwrap(),
            &fixture_config(&fixture.endpoint, &json!({"pacing": null})),
            &Secrets,
            clock.clone(),
        )
        .unwrap();
    source
        .write_task(&write(task(
            "T-clock",
            "Clock",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .unwrap();
    assert_eq!(
        *clock.waits.lock().unwrap(),
        vec![Duration::from_millis(750); 2]
    );
    source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap();
    assert_eq!(clock.waits.lock().unwrap().len(), 2, "reads take no slot");

    let endpoint = always(&Refusal::secondary_forbidden().after(0));
    let clock = Arc::new(AdvancingClock::default());
    let source = Plugin
        .build_with_clock(
            &SourceName::new("work").unwrap(),
            &fixture_config(
                &endpoint,
                &json!({"pacing": {
                    "min_mutation_interval_ms": 0, "retry_backoff_ms": 50,
                    "retry_budget_ms": 200
                }}),
            ),
            &Secrets,
            clock.clone(),
        )
        .unwrap();
    let message = refusal(
        source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap_err(),
    );
    assert!(message.contains("secondary rate limit"), "{message}");
    assert_eq!(
        *clock.waits.lock().unwrap(),
        vec![Duration::from_millis(50), Duration::from_millis(100)]
    );
}

/// The `Host` text field the projection cases put on the board.
fn host_field() -> Value {
    json!({"__typename":"ProjectV2Field","id":"FIELD_host","name":"Host","dataType":"TEXT"})
}

/// A source projecting `orchestrator.follow-up` at `host` onto the board's `Host` field — a
/// fresh one per call, as each command line is.
fn projecting(fixture: &Fixture) -> Box<dyn TaskSource> {
    configured(
        &fixture.endpoint,
        json!({"metadata_fields":[{"field":"Host","key":"orchestrator.follow-up","path":["host"]}]}),
    )
}

/// The writes of the `Host` field this board received, in order: the text written, or
/// `None` for a clear.
fn host_writes(fixture: &Fixture) -> Vec<Option<String>> {
    fixture
        .seen()
        .into_iter()
        .filter(|call| call[1]["fieldId"] == "FIELD_host")
        .map(|call| match call[0].as_str() {
            Some("updateProjectV2ItemFieldValue") => {
                Some(call[1]["value"]["text"].as_str().unwrap().to_owned())
            }
            Some("clearProjectV2ItemFieldValue") => None,
            other => panic!("an unexpected write of the Host field: {other:?}"),
        })
        .collect()
}

/// What the board's `Host` field holds for one item.
fn host_of(fixture: &Fixture, content_id: &str) -> Option<String> {
    fixture.item(content_id).texts.get("FIELD_host").cloned()
}

/// Caller metadata holding `host` at `orchestrator.follow-up.host`, beside a key of its own.
fn follow_up(host: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "orchestrator.follow-up".to_owned(),
            json!({"host": host, "run": "r-1"}),
        ),
        ("team.owner".to_owned(), json!("ada")),
    ])
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Task,
    Project,
    Document,
}

/// Write one item of `kind` with `metadata` — a create when `target` is `None`, and a copy
/// over the item `target` names otherwise — answering its id.
async fn write_kind(
    source: &dyn TaskSource,
    kind: Kind,
    target: Option<&str>,
    metadata: BTreeMap<String, Value>,
) -> Result<NativeId, SourceError> {
    let target = target.map(|id| NativeId(id.to_owned()));
    match kind {
        Kind::Task => {
            let mut item = task("T-1", "Investigate", status(StatusCategory::Todo, "Todo"));
            item.metadata = metadata;
            source
                .write_task(&ItemWrite {
                    target,
                    item,
                    depends_on: vec![],
                })
                .await
        }
        Kind::Project => {
            let mut item = project("P-1", "Follow-ups", status(StatusCategory::Todo, "Todo"));
            item.metadata = metadata;
            source
                .write_project(&ItemWrite {
                    target,
                    item,
                    depends_on: vec![],
                })
                .await
        }
        Kind::Document => {
            let mut item = document("D-1", "Design");
            item.metadata = metadata;
            source
                .write_document(&ItemWrite {
                    target,
                    item,
                    depends_on: vec![],
                })
                .await
        }
    }
}

/// Set one metadata key of one item of `kind` on its own — `metadata set`, and for a task the
/// targeted `task update` too, whichever `through_update` says.
async fn update_kind(
    source: &dyn TaskSource,
    kind: Kind,
    id: &NativeId,
    value: Option<Value>,
    through_update: bool,
) {
    let key = MetadataKey::new("orchestrator.follow-up").unwrap();
    let value = value.map(|host| json!({"host": host, "run": "r-1"}));
    match (kind, through_update) {
        (Kind::Task, true) => {
            let mut update = TaskUpdate::default();
            match value {
                Some(value) => {
                    update.metadata_set.insert(key, value);
                }
                None => {
                    update.metadata_remove.insert(key);
                }
            }
            source.update_task(id, &update).await.unwrap().unwrap();
        }
        (Kind::Task, false) => {
            source
                .set_task_metadata(id, &key, &value.unwrap_or(Value::Null))
                .await
                .unwrap()
                .unwrap();
        }
        (Kind::Project, _) => {
            source
                .set_project_metadata(id, &key, &value.unwrap_or(Value::Null))
                .await
                .unwrap()
                .unwrap();
        }
        (Kind::Document, _) => {
            source
                .set_document_metadata(id, &key, &value.unwrap_or(Value::Null))
                .await
                .unwrap()
                .unwrap();
        }
    }
}

#[tokio::test]
async fn a_metadata_fields_entry_that_cannot_stand_is_refused_naming_the_source_and_the_entry() {
    let base = fixture_config("http://127.0.0.1:9/graphql", &json!({}));
    let with = |entries: Value| {
        let mut config = base.clone();
        config["metadata_fields"] = entries;
        config
    };
    let host = json!({"field":"Host","key":"orchestrator.follow-up","path":["host"]});
    for (entries, said) in [
        (
            json!([{"field":" ","key":"orchestrator.follow-up"}]),
            "blank field",
        ),
        (json!([{"field":"Host","key":""}]), "blank key"),
        (
            json!([{"field":"Host","key":"onetaskgraph.origin"}]),
            "reserved \"onetaskgraph.\" prefix",
        ),
        (json!([{"field":"status","key":"team.state"}]), "\"Status\""),
        (
            json!([{"field":"PRIORITY","key":"team.state"}]),
            "\"Priority\"",
        ),
        (
            json!([{"field":"OneTaskGraph.Origin","key":"team.state"}]),
            "\"onetaskgraph.origin\"",
        ),
        (json!([{"field":"title","key":"team.state"}]), "\"Title\""),
        (
            json!([{"field":"Sub-issues Progress","key":"team.state"}]),
            "\"Sub-issues progress\"",
        ),
        (
            json!([{"field":"Linked pull requests","key":"team.state"}]),
            "GitHub",
        ),
        (
            json!([host, {"field":"host","key":"team.machine"}]),
            "same board field as metadata_fields[0]",
        ),
    ] {
        let message = build_refusal(with(entries.clone()));
        assert!(message.contains("source work"), "{entries}: {message}");
        assert!(message.contains("metadata_fields["), "{entries}: {message}");
        assert!(message.contains(said), "{entries}: {said}: {message}");
    }
    let unknown = build_refusal(with(json!([{"field":"Host","key":"a.b","depth":1}])));
    assert!(unknown.contains("depth"), "{unknown}");
    // A usable list, an empty one and none at all all build.
    for config in [with(json!([host])), with(json!([])), base.clone()] {
        Plugin
            .build(&SourceName::new("work").unwrap(), &config, &Secrets)
            .unwrap_or_else(|error| panic!("{config}: {error}"));
    }
}

#[tokio::test]
async fn a_projected_value_lands_is_kept_moves_and_clears_on_every_kind_and_every_write() {
    for kind in [Kind::Task, Kind::Project, Kind::Document] {
        let fixture = board(vec![]).with_field(host_field());
        // Create: the string lands in the field.
        let id = write_kind(
            projecting(&fixture).as_ref(),
            kind,
            None,
            follow_up(json!("alpha")),
        )
        .await
        .unwrap();
        assert_eq!(
            host_of(&fixture, &id.0).as_deref(),
            Some("alpha"),
            "{kind:?}"
        );
        assert_eq!(
            host_writes(&fixture),
            [Some("alpha".to_owned())],
            "{kind:?}"
        );

        // Copy over it, unchanged: no write of the field at all.
        let copied = |host: Value| {
            let fixture = &fixture;
            let id = id.clone();
            async move {
                write_kind(
                    projecting(fixture).as_ref(),
                    kind,
                    Some(&id.0),
                    follow_up(host),
                )
                .await
                .unwrap()
            }
        };
        copied(json!("alpha")).await;
        assert_eq!(
            host_writes(&fixture).len(),
            1,
            "{kind:?}: an unchanged copy"
        );
        // Changed: written again. Absent and null: cleared once, then nothing to clear.
        copied(json!("beta")).await;
        assert_eq!(
            host_of(&fixture, &id.0).as_deref(),
            Some("beta"),
            "{kind:?}"
        );
        copied(Value::Null).await;
        assert_eq!(host_of(&fixture, &id.0), None, "{kind:?}");
        let mut absent = follow_up(json!("unused"));
        absent.remove("orchestrator.follow-up");
        write_kind(projecting(&fixture).as_ref(), kind, Some(&id.0), absent)
            .await
            .unwrap();
        assert_eq!(
            host_writes(&fixture),
            [Some("alpha".to_owned()), Some("beta".to_owned()), None],
            "{kind:?}: a field already empty is not cleared again"
        );

        // Update: setting the key on its own moves the field the same way.
        update_kind(
            projecting(&fixture).as_ref(),
            kind,
            &id,
            Some(json!("gamma")),
            false,
        )
        .await;
        assert_eq!(
            host_of(&fixture, &id.0).as_deref(),
            Some("gamma"),
            "{kind:?}"
        );
        update_kind(
            projecting(&fixture).as_ref(),
            kind,
            &id,
            Some(json!("gamma")),
            false,
        )
        .await;
        update_kind(projecting(&fixture).as_ref(), kind, &id, None, false).await;
        assert_eq!(host_of(&fixture, &id.0), None, "{kind:?}");
        update_kind(projecting(&fixture).as_ref(), kind, &id, None, false).await;
        assert_eq!(
            host_writes(&fixture)[3..],
            [Some("gamma".to_owned()), None],
            "{kind:?}: an unchanged update writes nothing, an empty field is not cleared"
        );
    }
}

#[tokio::test]
async fn a_task_update_projects_the_keys_it_names_and_nothing_when_it_names_none() {
    let fixture = board(vec![]).with_field(host_field());
    let id = write_kind(
        projecting(&fixture).as_ref(),
        Kind::Task,
        None,
        follow_up(json!("alpha")),
    )
    .await
    .unwrap();
    update_kind(
        projecting(&fixture).as_ref(),
        Kind::Task,
        &id,
        Some(json!("beta")),
        true,
    )
    .await;
    assert_eq!(host_of(&fixture, &id.0).as_deref(), Some("beta"));
    update_kind(
        projecting(&fixture).as_ref(),
        Kind::Task,
        &id,
        Some(json!("beta")),
        true,
    )
    .await;
    // An update naming another key leaves the field alone, and sends it nothing.
    let mut update = TaskUpdate::default();
    update
        .metadata_set
        .insert(MetadataKey::new("team.owner").unwrap(), json!("grace"));
    projecting(&fixture)
        .update_task(&id, &update)
        .await
        .unwrap()
        .unwrap();
    update_kind(projecting(&fixture).as_ref(), Kind::Task, &id, None, true).await;
    assert_eq!(host_of(&fixture, &id.0), None);
    assert_eq!(
        host_writes(&fixture),
        [Some("alpha".to_owned()), Some("beta".to_owned()), None]
    );
}

#[tokio::test]
async fn a_value_no_text_field_can_hold_is_refused_before_any_mutation() {
    for (value, found) in [
        (json!(7), "a number"),
        (json!(true), "a boolean"),
        (json!({"name": "alpha"}), "an object"),
        (json!(["alpha"]), "an array"),
    ] {
        for kind in [Kind::Task, Kind::Project, Kind::Document] {
            let fixture = board(vec![]).with_field(host_field());
            let message = refusal(
                write_kind(
                    projecting(&fixture).as_ref(),
                    kind,
                    None,
                    follow_up(value.clone()),
                )
                .await
                .expect_err("a value no text field holds"),
            );
            for said in [
                "source work",
                "\"orchestrator.follow-up\"",
                "[\"host\"]",
                found,
            ] {
                assert!(
                    message.contains(said),
                    "{kind:?} {value}: {said}: {message}"
                );
            }
            assert_eq!(fixture.seen(), Vec::<Value>::new(), "{kind:?} {value}");
        }
    }
    // An update and a metadata set of an existing item are refused the same way.
    let fixture = board(vec![Item::issue("I_1", "held")]).with_field(host_field());
    let key = MetadataKey::new("orchestrator.follow-up").unwrap();
    let message = refusal(
        projecting(&fixture)
            .set_task_metadata(&NativeId("I_1".to_owned()), &key, &json!({"host": 7}))
            .await
            .expect_err("a number"),
    );
    assert!(message.contains("a number"), "{message}");
    let mut update = TaskUpdate::default();
    update.metadata_set.insert(key, json!({"host": [1]}));
    let message = refusal(
        projecting(&fixture)
            .update_task(&NativeId("I_1".to_owned()), &update)
            .await
            .expect_err("an array"),
    );
    assert!(message.contains("an array"), "{message}");
    assert_eq!(fixture.seen(), Vec::<Value>::new());
}

#[tokio::test]
async fn a_board_without_the_text_field_refuses_the_write_before_any_mutation() {
    for field in [
        None,
        Some(
            json!({"__typename":"ProjectV2Field","id":"FIELD_host","name":"Host","dataType":"NUMBER"}),
        ),
        Some(
            json!({"__typename":"ProjectV2SingleSelectField","id":"FIELD_host","name":"Host","options":[]}),
        ),
    ] {
        for kind in [Kind::Task, Kind::Project, Kind::Document] {
            let mut fixture = board(vec![]);
            if let Some(field) = field.clone() {
                fixture = fixture.with_field(field);
            }
            let message = refusal(
                write_kind(
                    projecting(&fixture).as_ref(),
                    kind,
                    None,
                    follow_up(json!("alpha")),
                )
                .await
                .expect_err("no text field to hold it"),
            );
            for said in ["source work", "\"Host\"", "sources fields work --apply"] {
                assert!(
                    message.contains(said),
                    "{kind:?} {field:?}: {said}: {message}"
                );
            }
            assert_eq!(fixture.seen(), Vec::<Value>::new(), "{kind:?} {field:?}");
        }
    }
}

#[tokio::test]
async fn a_read_reports_the_metadata_comment_and_never_the_projected_field() {
    // The field holds what a person typed on the board; the comment holds the value. A read
    // reports the comment's, and nothing of the field joins the metadata.
    let fixture = board(vec![
        Item::issue("I_1", "held")
            .holding_text("FIELD_host", "typed-on-the-board")
            .body(&slotted(
                "prose",
                &json!({"orchestrator.follow-up": {"host": "alpha", "run": "r-1"}}),
            )),
    ])
    .with_field(host_field());
    let read = projecting(&fixture)
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    let unprojected = source(&fixture)
        .get_task(&NativeId("I_1".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.metadata,
        BTreeMap::from([(
            "orchestrator.follow-up".to_owned(),
            json!({"host": "alpha", "run": "r-1"})
        )])
    );
    assert_eq!(read.metadata, unprojected.metadata);
}

/// A text field of the board, as its field list answers it.
fn text_field_def(id: &str, name: &str) -> Value {
    json!({"__typename":"ProjectV2Field","id":id,"name":name,"dataType":"TEXT"})
}

/// The text field `id` holds for one item.
fn text_of(fixture: &Fixture, content_id: &str, id: &str) -> Option<String> {
    fixture.item(content_id).texts.get(id).cloned()
}

/// The field writes this board received for `id`, in order: the text, or `None` for a clear.
fn text_writes(fixture: &Fixture, id: &str) -> Vec<Option<String>> {
    fixture
        .seen()
        .into_iter()
        .filter(|call| call[1]["fieldId"] == id)
        .map(|call| call[1]["value"]["text"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn several_projections_move_independently_and_validate_together_before_any_mutation() {
    let fixture = board(vec![])
        .with_field(host_field())
        .with_field(text_field_def("FIELD_team", "Team"));
    let source = || {
        configured(
            &fixture.endpoint,
            json!({"metadata_fields":[
                {"field":"Host","key":"orchestrator.follow-up","path":["host"]},
                {"field":"Team","key":"team.name"},
            ]}),
        )
    };
    let metadata = |follow_up: Value, team: Value| {
        BTreeMap::from([
            ("orchestrator.follow-up".to_owned(), follow_up),
            ("team.name".to_owned(), team),
        ])
    };
    // The key's own value with no path; a path whose step is missing projects nothing.
    let id = write_kind(
        source().as_ref(),
        Kind::Task,
        None,
        metadata(json!({"run":"r-1"}), json!("core")),
    )
    .await
    .unwrap();
    assert_eq!(
        text_of(&fixture, &id.0, "FIELD_team").as_deref(),
        Some("core")
    );
    assert_eq!(text_of(&fixture, &id.0, "FIELD_host"), None);
    assert_eq!(
        text_writes(&fixture, "FIELD_host"),
        Vec::<Option<String>>::new()
    );

    write_kind(
        source().as_ref(),
        Kind::Task,
        Some(&id.0),
        metadata(json!({"host":"alpha"}), json!("core")),
    )
    .await
    .unwrap();
    assert_eq!(
        text_of(&fixture, &id.0, "FIELD_host").as_deref(),
        Some("alpha")
    );
    // An empty string is no value, so it clears the field that held one.
    write_kind(
        source().as_ref(),
        Kind::Task,
        Some(&id.0),
        metadata(json!({"host":""}), json!("core")),
    )
    .await
    .unwrap();
    assert_eq!(text_of(&fixture, &id.0, "FIELD_host"), None);
    assert_eq!(
        text_writes(&fixture, "FIELD_host"),
        [Some("alpha".to_owned()), None]
    );
    assert_eq!(
        text_writes(&fixture, "FIELD_team"),
        [Some("core".to_owned())]
    );

    // An update naming one projected key moves that field and no other.
    let mut update = TaskUpdate::default();
    update
        .metadata_set
        .insert(MetadataKey::new("team.name").unwrap(), json!("infra"));
    source().update_task(&id, &update).await.unwrap().unwrap();
    assert_eq!(
        text_of(&fixture, &id.0, "FIELD_team").as_deref(),
        Some("infra")
    );
    assert_eq!(text_writes(&fixture, "FIELD_host").len(), 2);

    // One entry's value refused refuses the write, before the other's is sent.
    let before = fixture.seen().len();
    let message = refusal(
        write_kind(
            source().as_ref(),
            Kind::Task,
            Some(&id.0),
            metadata(json!({"host":"beta"}), json!(7)),
        )
        .await
        .expect_err("a number"),
    );
    assert!(message.contains("\"team.name\" at path []"), "{message}");
    assert_eq!(fixture.seen().len(), before, "nothing was sent");
    assert_eq!(text_of(&fixture, &id.0, "FIELD_host"), None);
}

#[tokio::test]
async fn more_writes_than_one_request_holds_go_in_further_requests_and_a_refusal_keeps_the_body() {
    let mut fixture = board(vec![]);
    let mut entries = Vec::new();
    for n in 0..8 {
        fixture = fixture.with_field(text_field_def(&format!("FIELD_b{n}"), &format!("B{n}")));
        entries
            .push(json!({"field":format!("B{n}"),"key":"batch.values","path":[format!("f{n}")]}));
    }
    let source = || configured(&fixture.endpoint, json!({"metadata_fields": entries}));
    let values = |text: &str| {
        BTreeMap::from([(
            "batch.values".to_owned(),
            Value::Object(
                (0..8)
                    .map(|n| (format!("f{n}"), json!(format!("{text}{n}"))))
                    .collect(),
            ),
        )])
    };
    let field_requests = |fixture: &Fixture| fixture.requests("updateProjectV2ItemFieldValue");

    // The Status option and eight fields: six writes in one request, three in the next.
    let id = write_kind(source().as_ref(), Kind::Task, None, values("a"))
        .await
        .unwrap();
    assert_eq!(field_requests(&fixture), 2);
    for n in 0..8 {
        assert_eq!(
            text_of(&fixture, &id.0, &format!("FIELD_b{n}")),
            Some(format!("a{n}"))
        );
    }

    // Eight clears and no write: three requests of three, three and two clears.
    let cleared = BTreeMap::from([("batch.values".to_owned(), json!({}))]);
    write_kind(source().as_ref(), Kind::Task, Some(&id.0), cleared)
        .await
        .unwrap();
    assert_eq!(field_requests(&fixture), 5);
    assert_eq!(
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "clearProjectV2ItemFieldValue")
            .count(),
        8
    );
    for n in 0..8 {
        assert_eq!(text_of(&fixture, &id.0, &format!("FIELD_b{n}")), None);
    }

    // The second request of a write refused: the first's fields landed, and the item's body —
    // the metadata's home — was never sent, so it holds what it held.
    let body = fixture.item(&id.0).body.clone();
    let updates = |fixture: &Fixture| {
        fixture
            .seen()
            .iter()
            .filter(|call| call[0] == "updateIssue")
            .count()
    };
    let issue_updates = updates(&fixture);
    fixture.refuse_after("updateProjectV2ItemFieldValue", 1);
    write_kind(source().as_ref(), Kind::Task, Some(&id.0), values("b"))
        .await
        .expect_err("the second request is refused");
    assert_eq!(updates(&fixture), issue_updates, "the body is written last");
    assert_eq!(fixture.item(&id.0).body, body);
    assert_eq!(
        text_of(&fixture, &id.0, "FIELD_b0").as_deref(),
        Some("b0"),
        "the first request landed"
    );
    assert_eq!(text_of(&fixture, &id.0, "FIELD_b7"), None);
}

/// A board with the eight `B*` text fields, and a source projecting `batch.values` at `f<n>`
/// onto each.
fn eight_fields() -> (Fixture, Value) {
    let mut fixture = board(vec![]);
    let mut entries = Vec::new();
    for n in 0..8 {
        fixture = fixture.with_field(text_field_def(&format!("FIELD_b{n}"), &format!("B{n}")));
        entries
            .push(json!({"field":format!("B{n}"),"key":"batch.values","path":[format!("f{n}")]}));
    }
    (fixture, json!({"metadata_fields": entries}))
}

/// `batch.values` holding `text<n>` at `f<n>` for each `n` in `set`, and nothing else.
fn batch(set: std::ops::Range<usize>, text: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([(
        "batch.values".to_owned(),
        Value::Object(
            set.map(|n| (format!("f{n}"), json!(format!("{text}{n}"))))
                .collect(),
        ),
    )])
}

#[tokio::test]
async fn writes_and_clears_together_span_requests_in_order_and_a_refused_one_stops_the_rest() {
    let (fixture, config) = eight_fields();
    let source = || configured(&fixture.endpoint, config.clone());
    let id = write_kind(source().as_ref(), Kind::Task, None, batch(0..8, "a"))
        .await
        .unwrap();
    let before = fixture.documents().len();
    // Four moved and four cleared: four writes and three clears in one request, then the last
    // clear in the next — writes run before clears within each.
    write_kind(source().as_ref(), Kind::Task, Some(&id.0), batch(0..4, "b"))
        .await
        .unwrap();
    let sent = fixture.documents()[before..]
        .iter()
        .filter(|document| *document == graphql::UPDATE_FIELDS)
        .count();
    assert_eq!(sent, 2);
    let order = fixture
        .seen()
        .iter()
        .filter(|call| {
            call[0]
                .as_str()
                .is_some_and(|op| op.contains("ProjectV2ItemFieldValue"))
        })
        .map(|call| {
            (
                call[0].as_str().unwrap().to_owned(),
                call[1]["fieldId"].clone(),
            )
        })
        .collect::<Vec<_>>();
    let order = order[order.len() - 8..].to_vec();
    let expected = (0..4)
        .map(|n| {
            (
                "updateProjectV2ItemFieldValue".to_owned(),
                json!(format!("FIELD_b{n}")),
            )
        })
        .chain((4..8).map(|n| {
            (
                "clearProjectV2ItemFieldValue".to_owned(),
                json!(format!("FIELD_b{n}")),
            )
        }))
        .collect::<Vec<_>>();
    assert_eq!(order, expected);
    for n in 0..4 {
        assert_eq!(
            text_of(&fixture, &id.0, &format!("FIELD_b{n}")),
            Some(format!("b{n}"))
        );
    }
    for n in 4..8 {
        assert_eq!(text_of(&fixture, &id.0, &format!("FIELD_b{n}")), None);
    }

    // Four moved back and four set again, the second request refused: what the first carried
    // landed, the rest did not, and the body was never sent.
    let body = fixture.item(&id.0).body.clone();
    fixture.refuse_after("updateProjectV2ItemFieldValue", 1);
    write_kind(source().as_ref(), Kind::Task, Some(&id.0), batch(0..7, "c"))
        .await
        .expect_err("the second request is refused");
    assert_eq!(fixture.item(&id.0).body, body);
    assert_eq!(text_of(&fixture, &id.0, "FIELD_b5").as_deref(), Some("c5"));
    assert_eq!(
        text_of(&fixture, &id.0, "FIELD_b6"),
        None,
        "the seventh write never ran"
    );
}

#[tokio::test]
async fn every_added_alias_answering_no_item_or_the_wrong_one_is_refused_by_name() {
    for alias in ["fourth", "fifth", "sixth", "clearedSecond", "clearedThird"] {
        for response in [Value::Null, json!({"projectV2Item":{"id":"wrong"}})] {
            let (fixture, config) = eight_fields();
            let source = || configured(&fixture.endpoint, config.clone());
            // A create sends the Status option and eight writes, six of them in its first
            // request; a copy over it moving five and clearing three sends one request of
            // five writes and three clears.
            let id = if alias.starts_with("cleared") {
                let id = write_kind(source().as_ref(), Kind::Task, None, batch(0..8, "a"))
                    .await
                    .unwrap();
                fixture.answer_alias_with(alias, response.clone());
                let error =
                    write_kind(source().as_ref(), Kind::Task, Some(&id.0), batch(0..5, "b"))
                        .await
                        .expect_err("a malformed answer");
                assert!(
                    refusal(error).contains(&format!("field update {alias}")),
                    "{alias} {response}"
                );
                continue;
            } else {
                fixture.answer_alias_with(alias, response.clone());
                write_kind(source().as_ref(), Kind::Task, None, batch(0..8, "a")).await
            };
            let error = id.expect_err("a malformed answer");
            assert!(
                refusal(error).contains(&format!("field update {alias}")),
                "{alias} {response}"
            );
        }
    }
}
