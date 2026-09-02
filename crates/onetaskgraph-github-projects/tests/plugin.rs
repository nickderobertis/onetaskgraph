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
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter, Location,
    NativeId, PageRequest, Project, ProjectFilter, ProjectQuery, Repository, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
    TaskSource, TextFields, TextQuery, WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, var: &str) -> Option<SecretString> {
        (var == "GH_PROJECTS_TOKEN").then(|| "test-token".into())
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
    parent: Option<String>,
    sub_issues: u64,
    repository: Option<&'static str>,
    labels: Vec<(&'static str, &'static str)>,
    status: Option<String>,
    origin: Option<String>,
    /// Whether this item's board half carries the board's built-in `Labels` field.
    ///
    /// GitHub derives that field from the issue's own labels — it is absent from
    /// `ProjectV2CustomFieldType`, so no project can create one, and
    /// `ProjectV2FieldValue` offers no way to write one — so what it holds is
    /// `labels_seen_on` and never a set of its own.
    board_labels_field: bool,
    /// What that field holds, when it is deliberately not the issue's own labels.
    ///
    /// GitHub cannot produce this for `Issue` content — the field mirrors those labels —
    /// but `graphql::BOARD` selects both connections because a board item's content may be
    /// a draft with no labels of its own, so the reader unions them, and a union is only
    /// measurable over two sets that differ.
    board_labels: Option<Vec<(&'static str, &'static str)>>,
    /// A label set this board answers one path with, instead of the one above.
    ///
    /// Nothing GitHub does. It is how the four-way equivalence check is watched failing:
    /// a check that agrees with itself over every tree is not evidence that it would
    /// catch a path serving another path's answer.
    path_labels: BTreeMap<&'static str, Vec<(&'static str, &'static str)>>,
}

/// What the document that just arrived asked for, as far as rendering an item needs it.
///
/// Both halves are read off the document rather than assumed, so this board answers what
/// was selected: a selection put back into the shared board-issue fragment would start
/// being answered on those three paths again, and the equivalence check would see it.
#[derive(Clone, Copy)]
struct Asked<'a> {
    /// Which of this source's reads this is, by the name `operation_name` gives it.
    path: &'a str,
    /// Whether it selected the board's built-in `Labels` field value.
    board_labels: bool,
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
            parent: None,
            sub_issues: 0,
            repository: Some("acme/work"),
            labels: vec![],
            status: None,
            origin: None,
            board_labels_field: false,
            board_labels: None,
            path_labels: BTreeMap::new(),
        }
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
    fn sub_issues(mut self, total: u64) -> Self {
        self.sub_issues = total;
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
    /// Give this item's board half the board's built-in `Labels` field.
    fn board_labels_field(mut self) -> Self {
        self.board_labels_field = true;
        self
    }
    /// The same, holding a set of its own. See [`Item::board_labels`].
    fn board_labels_field_of(mut self, labels: &[(&'static str, &'static str)]) -> Self {
        self.board_labels = Some(labels.to_vec());
        self.board_labels_field()
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

    fn field_values(&self, options: &Value, asked: Asked) -> Value {
        let mut nodes = Vec::new();
        if let Some(status) = &self.status {
            nodes.push(
                json!({"name":status,"field":{"id":"FIELD_status","name":"Status","options":options}}),
            );
        }
        nodes.push(
            json!({"text":self.origin.clone().unwrap_or_default(),"field":{"id":"FIELD_origin","name":"onetaskgraph.origin"}}),
        );
        if self.board_labels_field && asked.board_labels {
            let held = self
                .board_labels
                .as_deref()
                .unwrap_or_else(|| self.labels_seen_on(asked.path));
            nodes.push(json!({"labels":{
                "nodes":held.iter().map(|(id,name)| json!({"id":id,"name":name,"color":null}))
                    .collect::<Vec<_>>(),
                "pageInfo":{"hasNextPage":false}}}));
        }
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }

    /// The board half of this item, as `Issue.projectItems` carries it.
    ///
    /// The same board item id and the same field values a `ProjectV2.items` read gives it,
    /// reached from the issue instead of from the board. Every fixture item here sits on
    /// the one board this suite configures, which is project number 7.
    fn project_items(&self, options: &Value, asked: Asked) -> Value {
        json!({"nodes":[{"id":self.item_id,"project":{"number":7},
                         "fieldValues":self.field_values(options, asked)}],
               "pageInfo":{"hasNextPage":false}})
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
            _ => json!({"__typename":"Issue","id":self.content_id,"title":self.title,
                "body":self.body.clone().unwrap_or_default(),
                "url":format!("https://github.example/{}", self.content_id),
                "createdAt":null,"updatedAt":null,"state":self.state,
                "stateReason":self.state_reason,
                "repository":self.repository.map(|r| json!({"nameWithOwner":r})),
                "parent":self.parent.as_ref().map(|id| json!({"id":id})),
                "subIssuesSummary":{"total":self.sub_issues},
                "labels":self.label_nodes(asked.path)}),
        }
    }
}

/// Everything the fixture remembers between requests.
struct State {
    items: Vec<Item>,
    /// Issues created but not yet added to the board.
    pending: Vec<Item>,
    options: Vec<(&'static str, &'static str)>,
    origin_field: bool,
    status_field: bool,
    blocked_by: BTreeMap<String, Vec<String>>,
    /// Mutations this board answers with a GraphQL error rather than performing. GitHub
    /// fails one call of the several a write is, and what the source does about the calls
    /// that already landed is only readable if one of them can be made to fail.
    refuses: BTreeSet<String>,
    /// How many of the most recently filed items this board's own reads do not show yet.
    /// GitHub's `projectV2.items` is eventually consistent, so an item a run just created
    /// is answered out of the source's own record of what it created until the board
    /// catches up — and what that record holds is only observable while it is behind.
    lagging_reads: usize,
    seen: Vec<Value>,
    /// Every GraphQL document this board received, in order.
    ///
    /// The operation counts beside it say how *many* requests a read made; this says what
    /// each one asked for, which is the only place a test can see that a read scoped to one
    /// project never asked the board for its items.
    documents: Vec<String>,
    /// The search string of every board-scoped search this board answered, in order.
    searches: Vec<String>,
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
                json!({"__typename":"ProjectV2Field","id":"FIELD_origin","name":"onetaskgraph.origin"}),
            );
        }
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
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
    /// Every GraphQL document this board received, in order.
    fn documents(&self) -> Vec<String> {
        self.state.lock().unwrap().documents.clone()
    }
    /// The search string of every board-scoped search this board answered, in order.
    fn searches(&self) -> Vec<String> {
        self.state.lock().unwrap().searches.clone()
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
}

/// The whole allowance this board reports in its own rate-limit headers.
///
/// **Deliberately not GitHub's published hourly figure, and that is what makes the
/// assertions below evidence.** A fixture that mirrored the real allowance would restate
/// GitHub's contract with nothing reconciling the two, and — worse — a report that printed
/// a number both sides already knew would pass whether or not it had read a single header.
/// This one is the board's own, so the only way the report can carry it is by having read
/// what this board sent.
const FIXTURE_BUDGET_LIMIT: u64 = 4_321;

fn board(items: Vec<Item>) -> Fixture {
    board_with(items, true, true)
}

fn board_with(items: Vec<Item>, status_field: bool, origin_field: bool) -> Fixture {
    let state = Arc::new(Mutex::new(State {
        items,
        pending: Vec::new(),
        options: vec![
            ("OPT_backlog", "Backlog"),
            ("OPT_todo", "Todo"),
            ("OPT_doing", "In Progress"),
            ("OPT_shipped", "Shipped"),
        ],
        origin_field,
        status_field,
        blocked_by: BTreeMap::new(),
        refuses: BTreeSet::new(),
        lagging_reads: 0,
        seen: Vec::new(),
        documents: Vec::new(),
        searches: Vec::new(),
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
                    match refused(&served, query, variables) {
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

/// The GraphQL error a refused mutation answers with, or `None` to perform it.
///
/// A refused call is still recorded as seen and still changes nothing: that is what GitHub
/// failing one mutation of a write looks like from here.
fn refused(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Option<String> {
    let mut state = state.lock().unwrap();
    let operation = operation_name(query);
    if !state.refuses.contains(operation) {
        return None;
    }
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state.seen.push(json!([operation, input]));
    }
    Some(format!("{operation} is refused by this board"))
}

fn answer(state: &Arc<Mutex<State>>, query: &str, variables: &Value) -> Value {
    let mut state = state.lock().unwrap();
    // What this document asked for, read off the document itself. The three reads that
    // reach an issue share one fragment and do not select the board's built-in `Labels`
    // field; the board's own item read does. Answering what was selected is what makes the
    // four-way equivalence check below a check on the production documents.
    let asked = Asked {
        path: operation_name(query),
        board_labels: query.contains("ProjectV2ItemFieldLabelValue"),
    };
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if !input.is_null() {
        state
            .seen
            .push(json!([operation_name(query), input.clone()]));
    }
    if query.contains("repository(owner:$owner,name:$name)") {
        return if variables["name"] == "missing" {
            json!({ "repository": null })
        } else {
            json!({"repository":{"id":"REPO_1","nameWithOwner":format!("{}/{}", variables["owner"].as_str().unwrap(), variables["name"].as_str().unwrap())}})
        };
    }
    if query.contains("deleteIssue(input:$input)") {
        let id = input["issueId"].as_str().expect("an issue id").to_owned();
        state.items.retain(|item| item.content_id != id);
        return json!({"deleteIssue":{"repository":{"id":"REPO_1"}}});
    }
    if query.contains("createIssue(input:$input)") {
        state.next += 1;
        let id = format!("I_new{}", state.next);
        let mut created = Item::issue(&id, input["title"].as_str().unwrap_or_default());
        created.body = input["body"].as_str().map(str::to_owned);
        state.pending.push(created);
        return json!({"createIssue":{"issue":{"id":id,
            "url":format!("https://github.example/{id}")}}});
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
        item.title = input["title"].as_str().unwrap_or_default().to_owned();
        item.body = input["body"].as_str().map(str::to_owned);
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
        if let Some(text) = text {
            item.origin = Some(text);
        }
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id}}});
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
    if query.contains("search(query:$search") {
        assert_eq!(variables["type"], "ISSUE");
        let search = variables["search"].as_str().expect("a search query");
        let wanted = search
            .strip_prefix("project:octo-org/7 is:issue")
            .unwrap_or_else(|| panic!("a search scoped to the configured board: {search}"))
            .to_owned();
        state.searches.push(search.to_owned());
        let wanted = wanted.as_str();
        // The server side of `in:title "..."`, which is what makes naming a project by name
        // one bounded query rather than a walk of the board.
        let title = wanted.trim().strip_prefix("in:title ").map(|quoted| {
            quoted
                .trim()
                .trim_matches('"')
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        });
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
        let matched = state.items[..visible]
            .iter()
            .filter(|item| item.typename == "Issue")
            .filter(|item| {
                title
                    .as_ref()
                    .is_none_or(|title| item.title.eq_ignore_ascii_case(title))
            })
            .collect::<Vec<_>>();
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
    if query.contains("node(id:$id){__typename ...BoardIssue}") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let options = state.options();
        return json!({ "node": item.as_issue(&options, asked) });
    }
    if query.contains("node(id:$id)") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = state.items.iter().find(|item| item.content_id == id) else {
            return json!({ "node": null });
        };
        if item.typename != "Issue" {
            return json!({"node":{"__typename":item.typename}});
        }
        let related = |ids: Vec<String>| {
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
        };
        let blocked = state.blocked_by.get(&id).cloned().unwrap_or_default();
        let blocking = state
            .blocked_by
            .iter()
            .filter(|(_, blockers)| blockers.contains(&id))
            .map(|(issue, _)| issue.clone())
            .collect::<Vec<_>>();
        return json!({"node":{"__typename":"Issue",
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
    let visible = state.items.len().saturating_sub(state.lagging_reads);
    let end = (offset + first).min(visible);
    let options = state.options();
    let nodes = state.items[offset.min(end)..end]
        .iter()
        .map(|item| {
            json!({"id":item.item_id,"fieldValues":item.field_values(&options, asked),
                   "content":item.content(asked)})
        })
        .collect::<Vec<_>>();
    json!({"owner":{"projectV2":{"id":"PVT_board","title":"Roadmap","fields":state.fields(),
        "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < visible,"endCursor":end.to_string()}}}}})
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
        "owner" => "board",
        "node" if query.contains("subIssues(") => "projectTasks",
        "node" if query.contains("blockedBy(") => "issueDependencies",
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
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/graphql")
}

fn sequence_server(bodies: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
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
    Accounting, Basis, Budget, BudgetReport, Method, Mode, Outcome, RateLimit, Request, Session,
    StatusCode,
};
use onetaskgraph_github_projects::{DESIGN_TITLE_PREFIX, Plugin, graphql};

fn source(fixture: &Fixture) -> Box<dyn TaskSource> {
    configured(&fixture.endpoint, json!({}))
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
        title: title.to_owned(),
        content: None,
        status,
        labels: vec![],
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
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
                let title = search
                    .strip_prefix("project:octo-org/7 is:issue")
                    .expect("a search scoped to the configured board")
                    .trim()
                    .strip_prefix("in:title ")
                    .map(|quoted| quoted.trim().trim_matches('"').to_owned());
                let matched = recorded
                    .iter()
                    .filter(|node| {
                        title.as_ref().is_none_or(|title| {
                            node["title"].as_str().is_some_and(|held| held == title)
                        })
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

/// A board of one project and its one task, both carrying the board's `Labels` field.
///
/// The field is the point: it is the connection the shared board-issue fragment stopped
/// selecting, so this is the shape that would show the loss if anything were lost.
fn equivalence_board(plan: Item, step: Item) -> Fixture {
    board(vec![plan, step])
}

fn labelled_plan() -> Item {
    Item::issue("I_plan", "Delivery plan")
        .sub_issues(1)
        .status("In Progress")
        .labelled(&[("L_bug", "bug"), ("L_team", "team")])
        .board_labels_field()
}

fn labelled_step() -> Item {
    Item::issue("I_step", "First step")
        .parent("I_plan")
        .status("Todo")
        .labelled(&[("L_bug", "bug"), ("L_team", "team")])
        .board_labels_field()
}

#[tokio::test]
async fn an_item_reports_the_same_labels_title_status_and_id_however_it_is_reached() {
    // The board-issue fragment no longer selects the board's built-in `Labels` field, which
    // is what took `search` and `subIssues` under GitHub's node limit. It is only sound
    // because that field mirrors the issue's own labels, which the fragment does select —
    // so the four documents have to go on agreeing, and this is where that is measured.
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

    let selecting = fixture
        .documents()
        .into_iter()
        .filter(|document| document.contains("ProjectV2ItemFieldLabelValue"))
        .count();
    assert_eq!(
        selecting, 1,
        "the board's own item read is the one document that still asks for the board \
         `Labels` field; the three that reach an issue must not, or they are back over the \
         node limit"
    );
    assert!(
        fixture
            .board_item_reads()
            .iter()
            .all(|document| document.contains("ProjectV2ItemFieldLabelValue")),
        "and it is the board read that asks for it"
    );
}

#[tokio::test]
async fn the_boards_own_labels_field_is_read_and_folded_in_without_doubling_the_issues_own() {
    // `graphql::BOARD` is the one document that still selects the board's built-in
    // `Labels` field, and it must: a board item's content may be a draft, which has no
    // `labels` of its own. So the reader unions the two sets and dedupes them by id, and
    // this is where that is measured — with one label in both and one in the field alone,
    // neither of which the answer may double or drop.
    let fixture = board(vec![
        Item::issue("I_loose", "Loose end")
            .labelled(&[("L_bug", "bug"), ("L_team", "team")])
            .board_labels_field_of(&[("L_bug", "bug"), ("L_extra", "extra")]),
    ]);
    let source = source(&fixture);

    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("the board lists its tasks");

    assert_eq!(
        tasks.items[0]
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["bug", "team", "extra"],
        "the issue's own labels first, then whatever the board field adds, each once"
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

#[tokio::test]
async fn the_shipped_mapping_puts_each_category_where_it_says_it_does() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    for (category, name, expected_option, expected_state) in [
        (StatusCategory::Backlog, "Backlog", Some("Backlog"), "OPEN"),
        (StatusCategory::Todo, "Todo", Some("Todo"), "OPEN"),
        (
            StatusCategory::InProgress,
            "In Progress",
            Some("In Progress"),
            "OPEN",
        ),
        (StatusCategory::Done, "Done", None, "CLOSED"),
        (StatusCategory::Cancelled, "Cancelled", None, "CLOSED"),
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
async fn done_closes_by_default_and_an_override_puts_it_back_on_a_column() {
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
    assert_eq!(held.state, "OPEN", "an overridden done stays a column");
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
async fn the_copy_origin_is_kept_in_the_boards_own_text_field() {
    let fixture = board(vec![]);
    let source = source(&fixture);
    let mut item = task("T-1", "Publish", status(StatusCategory::Todo, "Todo"));
    item.metadata = BTreeMap::from([("onetaskgraph.origin".to_owned(), json!("notes:T-1"))]);
    let id = source.write_task(&write(item)).await.unwrap();
    assert_eq!(fixture.item(&id.0).origin.as_deref(), Some("notes:T-1"));
    assert!(
        !fixture
            .item(&id.0)
            .body
            .unwrap_or_default()
            .contains("origin"),
        "a short typed value belongs in a typed field, not in the caller's own prose"
    );
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().metadata["onetaskgraph.origin"],
        json!("notes:T-1")
    );
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
                json!({"nodes":[{"id":"PVTI","content":{"__typename":"Issue","id":"I","subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
                complete.clone(),
            ),
            "missing fieldValues",
        ),
        (
            board(
                json!({"nodes":[{"id":"PVTI","fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":true}},
                                 "content":{"__typename":"Issue","id":"I","subIssuesSummary":{"total":0}}}],"pageInfo":{"hasNextPage":false}}),
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
    let empty_board = json!({"data":{"owner":{"projectV2":{"id":"B","title":"T","fields":complete,
        "items":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    for (bodies, expected) in [
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":null}}}),
            ],
            "returned no issue",
        ),
        (
            vec![
                empty_board.clone(),
                json!({"data":{"createIssue":{"issue":{"id":"I_new"}}}}),
                json!({"data":{"addProjectV2ItemById":{"item":null}}}),
            ],
            "returned no project item",
        ),
    ] {
        let mut bodies = bodies;
        bodies.insert(
            1,
            json!({"data":{"repository":{"id":"R","nameWithOwner":"acme/work"}}}),
        );
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

fn plain_issue() -> Value {
    json!({"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
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
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":"",
                    "createdAt":"not-a-time","subIssuesSummary":{"total":0}})
                )])),
            ),
            "is not a timestamp",
        ),
        (
            board_json(
                usable_fields(),
                complete(json!([issue_item(
                    json!({"__typename":"Issue","id":"I_1","title":"one","body":"",
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
        let body = board_json(fields, complete(json!([])));
        let endpoint = sequence_server(vec![
            body.clone(),
            json!({"data":{"repository":{"id":"R","nameWithOwner":"acme/work"}}}),
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
async fn a_category_configured_closed_closes_the_issue_it_is_written_to() {
    let fixture = board(vec![]);
    let source = configured(
        &fixture.endpoint,
        json!({"status_mapping":{"in-progress":{"closed":"not-planned"}}}),
    );
    let id = source
        .write_task(&write(task(
            "T-1",
            "one",
            status(StatusCategory::InProgress, "In Progress"),
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
    let board_with_one = board_json(
        fields.clone(),
        complete(json!([{"id":"PVTI_1","fieldValues":complete(json!([])),
                         "content":{"__typename":"Issue","id":"I_1","title":"one","body":"",
                                    "state":"OPEN","stateReason":null,
                                    "repository":{"nameWithOwner":"acme/work"},
                                    "parent":{"id":"I_old"},"subIssuesSummary":{"total":0},
                                    "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}])),
    );
    let ok_update = json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}});
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![board_with_one.clone(), json!({"data":{"updateIssue":{}}})],
            "item update returned no item",
        ),
        (
            vec![
                board_with_one.clone(),
                json!({"data":{"updateIssue":{"issue":{"id":"I_other"}}}}),
            ],
            "item update returned the wrong item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{}}}),
            ],
            "field update returned no project item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_other"}}}}),
            ],
            "field update returned the wrong project item",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"issue":{"id":"I_old"}}}}),
            ],
            "sub-issue update returned no sub-issue",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"removeSubIssue":{"subIssue":{"id":"I_1"}}}}),
            ],
            "sub-issue update returned no issue",
        ),
        (
            vec![
                board_with_one.clone(),
                ok_update.clone(),
                ok_field.clone(),
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
    let board_with_two = board_json(
        usable_fields(),
        complete(json!([
            {"id":"PVTI_1","fieldValues":complete(json!([])),
             "content":{"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
                        "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                        "parent":null,"subIssuesSummary":{"total":0},
                        "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}},
            {"id":"PVTI_2","fieldValues":complete(json!([])),
             "content":{"__typename":"Issue","id":"I_2","title":"two","body":"","state":"OPEN",
                        "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                        "parent":null,"subIssuesSummary":{"total":0},
                        "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}
        ])),
    );
    let ok_update = json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}});
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let held = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue"}}}),
            ],
            "no blockedBy connection",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                json!({"data":{"node":{"__typename":"Issue",
                    "blockedBy":{"nodes":"no","pageInfo":{"hasNextPage":false}}}}}),
            ],
            "nodes is not an array",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"blockingIssue":{"id":"I_2"}}}}),
            ],
            "dependency update returned no issue",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
                ok_field.clone(),
                held.clone(),
                json!({"data":{"addBlockedBy":{"issue":{"id":"I_1"}}}}),
            ],
            "returned no blocking issue",
        ),
        (
            vec![
                board_with_two.clone(),
                ok_update.clone(),
                ok_field.clone(),
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
    let board_one = board_json(
        usable_fields(),
        complete(json!([{"id":"PVTI_1","fieldValues":complete(json!([])),
            "content":{"__typename":"Issue","id":"I_1","title":"one","body":"","state":"OPEN",
                       "stateReason":null,"repository":{"nameWithOwner":"acme/work"},
                       "parent":null,"subIssuesSummary":{"total":0},
                       "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}])),
    );
    let ok_field =
        json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}});
    let endpoint = sequence_server(vec![
        board_one,
        json!({"data":{"updateIssue":{"issue":{"id":"I_1"}}}}),
        ok_field.clone(),
        ok_field,
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_a"}],"pageInfo":{"hasNextPage":true,"endCursor":"c1"}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue",
            "blockedBy":{"nodes":[{"id":"I_b"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_a"}}}}),
        json!({"data":{"removeBlockedBy":{"issue":{"id":"I_1"},"blockingIssue":{"id":"I_b"}}}}),
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
async fn a_closed_status_still_selects_the_column_that_spells_it_so_a_copy_settles() {
    // The closed state carries the category and the option carries the name, so a write
    // that closed the issue and left the option alone would read back under whatever
    // column the item happened to sit in — and a copy would report a change forever.
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
    assert_eq!(fixture.item(&id.0).status.as_deref(), Some("Shipped"));
    assert_eq!(
        source.get_task(&id).await.unwrap().unwrap().status,
        status(StatusCategory::Done, "Shipped")
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
    let mut revised = task("T-1", "one, revised", status(StatusCategory::Todo, "Todo"));
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
        "one, revised",
        "the mutation before the failure landed, and writing that back is the engine's \
         journal's job — which it can only do while the item is still there"
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
async fn a_project_copy_reads_the_board_and_the_repository_once_for_the_whole_command() {
    // Six items, so a source reading the board per item written reads it seven times and
    // the repository six — which is the burst this counts, not the writes themselves.
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
        fixture.requests("board"),
        1,
        "the board was re-read per item written"
    );
    assert_eq!(
        fixture.requests("repository"),
        1,
        "the destination repository was re-resolved per issue created"
    );
    assert_eq!(
        fixture.item(&plan.0).sub_issues,
        5,
        "and the copy still filed every task under its project"
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
    // these four mutations inside a millisecond of each other, so every gap below fails.
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
    assert!(
        gaps.len() >= 3,
        "a created task is several mutations, and this saw {}",
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
    assert!(
        gaps.len() >= 3,
        "a created task is several mutations, and this saw {}",
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
        ("repository", "reading the destination repository"),
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
        // A project write is `repository`, `createIssue`, `addProjectV2ItemById` and the
        // board fields; the two dependency operations need an item with a far end, so
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
    fixture.read_behind(1);

    let mut design = document("D-1", "Alpha design");
    design.content = Some("the engine core, reviewed".to_owned());
    design.project = Some(NativeId("I_plan".to_owned()));
    design.metadata = BTreeMap::from([("caller.flags".to_owned(), json!([true, null]))]);
    let created = source
        .write_document(&write(design))
        .await
        .expect("a document copies onto this board");

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
        .find(|report| report.budget == Budget::Graphql)
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
    assert_eq!(budget_of(&whole).limit, Some(FIXTURE_BUDGET_LIMIT));
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
    assert_eq!(graphql.limit, Some(FIXTURE_BUDGET_LIMIT));
    assert_eq!(graphql.used_by_the_account, Some(used));
    assert_eq!(
        graphql.remaining_last_seen,
        Some(FIXTURE_BUDGET_LIMIT - used)
    );
    // The session's own spend is attributed per call rather than read off the account.
    assert_eq!(graphql.spent, session.total_requests() as u64);
    assert_eq!(graphql.modelled, graphql.spent);
    assert_eq!(graphql.reported, 0);

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
            "spent {} points: 0 reported by GitHub",
            graphql.spent
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
        shared_budget.spent, alone_budget.spent,
        "the session's spend moved with the account's allowance"
    );
    assert!(
        shared_budget.account_allowance_fall() > alone_budget.account_allowance_fall(),
        "the shared board's allowance was supposed to fall faster"
    );
    assert!(
        shared_budget.account_allowance_fall().unwrap() > shared_budget.spent,
        "the account's allowance fell by {:?} and this session spent {}",
        shared_budget.account_allowance_fall(),
        shared_budget.spent
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
    assert_eq!(refused_record.spend().basis, Basis::Modelled);
    assert_eq!(refused_record.spend().amount, 1);

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
    assert_eq!(refusal.spend().basis, Basis::NotRun);
    assert_eq!(refusal.spend().amount, 0);
    assert!(
        session.report().contains("1 rate-limited"),
        "{}",
        session.report()
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
        Request::rest(Method::Get, "/repos/{owner}/{repo}/labels")
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
    assert_eq!(session.spent(Budget::Rest), 1);
    assert_eq!(
        session.spent(Budget::Graphql),
        from_the_source as u64 + 37,
        "GitHub's own reported cost is what a call that reports one is attributed"
    );
    let rest = session
        .budgets()
        .into_iter()
        .find(|report| report.budget == Budget::Rest)
        .expect("the session drew on the REST budget");
    assert_eq!(rest.counted, 1);
    assert_eq!(rest.limit, Some(1234));
    assert_eq!(rest.remaining_last_seen, Some(1221));
    assert_eq!(budget_of(&session).reported, 37);

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
    let listed =
        Request::rest(Method::Get, "/repos/{owner}/{repo}/labels").answered(RateLimit::default());
    assert_eq!(
        listed.call(),
        &onetaskgraph_github_projects::accounting::Call::Endpoint {
            endpoint: "GET /repos/{owner}/{repo}/labels".to_owned()
        }
    );
    assert_eq!(listed.node_count(), None);
    assert_eq!(listed.spend().basis, Basis::Counted);
    assert_eq!(listed.spend().amount, 1);
    let refused = Request::rest(Method::Delete, "/repos/{owner}/{repo}/labels/{name}")
        .finished(Outcome::Refused, RateLimit::default());
    assert_eq!(refused.mode(), Mode::Write);
    assert_eq!(refused.spend().basis, Basis::Counted);
    assert_eq!(refused.spend().amount, 1);
    let limited = Request::rest(Method::Post, "/repos/{owner}/{repo}/labels")
        .finished(Outcome::RateLimited, RateLimit::default());
    assert_eq!(limited.spend().basis, Basis::NotRun);
    assert_eq!(limited.spend().amount, 0);

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
    assert_eq!(session.spent(Budget::Graphql), 1);

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
