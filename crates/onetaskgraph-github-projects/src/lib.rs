//! A stateless onetaskgraph source over one GitHub Projects v2 board.
//!
//! **A board is a container of projects, not a project.** Its own `title`,
//! `shortDescription` and `readme` are never read as an item's fields and are never
//! written: nothing in this source can rename the board a user configured.
//!
//! **A project is an issue and its tasks are that issue's sub-issues.** GitHub's schema
//! decides that: `Issue` exposes `parent`, `subIssues` and `subIssuesSummary`, and
//! `DraftIssue` exposes none of them. Creating an issue needs a `repositoryId`, and a
//! board has none, so [`GitHubProjectsConfig::repository`] names the one repository this
//! source creates its project and task issues in; a write without it is refused naming
//! the field.
//!
//! **A document is an ordinary issue whose title begins [`DESIGN_TITLE_PREFIX`].** A
//! board has no document type and nothing but issues to hold one in, so the title is the
//! discriminator and it is the whole of it. The title this source *reports* is the one a
//! person wrote, with the prefix taken off — the same way the metadata slot is taken off
//! the body so `content` is what the person wrote — and writing a document puts the prefix
//! back, so a round trip returns the title that went in.
//!
//! **Telling a document from a project from a task.** The design prefix is read **first**:
//! a document is never a project and never a task, whatever sub-issues it has or does not
//! have. Only then does the rest apply — a board issue is a project when *either* it has
//! sub-issues *or* it carries [`ItemKind::METADATA_KEY`]; otherwise it is a task. A
//! sub-issue is always a task, whatever it carries. The marker is sufficient and never
//! necessary: it is what makes an *empty* project — the state a project copy passes
//! through between creating the project and filing its first task — readable as a
//! project, while the sub-issue arm lets a person author a project on the board by hand
//! with no knowledge of this product's metadata at all. Reading the prefix later than the
//! sub-issue rule would make a design issue with no sub-issues an empty project, which is
//! exactly the state that rule exists to catch. Pull requests are neither a project nor a
//! task nor a document and are ignored.
//!
//! **Where an entity is, is a link.** Every project, task and document this source reports
//! carries a [`Location::Url`] naming the issue's own web address — the same address the
//! `url` field already reports, in the shape that says a reader can open it. That is the
//! contrast the location contract exists for: a reader holding an entity from this source
//! is handed something to link to and one holding an entity from a folder of Markdown is
//! handed a path, and neither has to know which plugin answered. It does not replace or
//! derive from `url`; that field goes on reporting what it always reported.
//!
//! **Where metadata lives.** Short typed things go to typed fields and native relations:
//! status to the board's `Status` single-select and the issue's own state, the copy
//! origin to a source-owned `onetaskgraph.origin` text field, and dependencies to
//! `blockedBy` and to sub-issue links. Unbounded caller JSON goes in a trailing
//! `<!-- onetaskgraph.metadata ... -->` comment at the end of the issue body — the same
//! encoding `docs/metadata.md` settles for Linear, not a second one. A ProjectV2 text
//! field is length-bounded and `shortDescription` is capped at 300 characters, which is
//! why neither can hold a caller's own prose.
//!
//! **Status.** `status_mapping` is per-instance configuration from a status category to
//! `null`, a board `Status` option name, or a closed state of `completed` or
//! `not-planned`. Nothing here ever calls `updateProjectV2Field`: that mutation's
//! `singleSelectOptions` *overwrites* a field's option set, so no addition is additive
//! and a mistake destroys every item's status. A status this board cannot represent is a
//! refusal naming the status and the instance instead.
//!
//! `done` closes the issue by default because GitHub derives `subIssuesSummary.completed`
//! and the board's own `Sub-issues progress` field from closed sub-issues: a plan whose
//! finished tasks were only moved to a "Done" column would read 0% complete forever.
//!
//! # What this source declares, field by field
//!
//! One verdict per field of [`Capabilities`], and what `Native` means when this source
//! says it. *Proven* means a shared journey drives it against the real
//! binary over this source's own row in `crates/onetaskgraph/tests/e2e/fixtures.rs`, and
//! `every_row_declares_exactly_what_its_plugin_reports` is what keeps this list and
//! [`capabilities`](TaskSource::capabilities) from parting.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven,** and the one predicate here that is pushed down rather than applied in process: a task's project is the issue it is a sub-issue of, so a listing scoped to one *asks that issue* for its own sub-issues. This is the field that was declared and then not applied, which silently returned another project's tasks. |
//! | `documents` | **Supported and proven.** A board holds issues, so a document is one: the issue whose title begins [`DESIGN_TITLE_PREFIX`]. Reads, filters and paging answer on exactly the terms a task read does, and a write puts the prefix back. |
//! | `orphan_tasks` | **Supported and proven.** A task issue with no `parent` is in no project. |
//! | `filter_by_label` | **Supported and proven,** over the issue's own labels. |
//! | `filter_by_status` | **Supported and proven,** over the board's `Status` option and the issue's open or closed state, through this instance's own `status_mapping`. |
//! | `search_title` | **Supported and proven,** over `Issue.title`. |
//! | `search_content` | **Supported and proven,** over the visible body — the trailing metadata comment is not part of it. |
//! | `task_dependencies` | **Supported and proven,** in both directions: `blockedBy` and `blocking`. |
//! | `project_dependencies` | **Supported and proven,** in both directions, over the same two connections, because a project here is an issue. |
//! | `max_page_size` | **Supported and proven.** [`MAX_PAGE_SIZE`], GitHub's own connection maximum. |
//!
//! Nothing here is unsupported. `documents` is not a predicate — it says this source has
//! documents, which it does — and the three facts behind the uniform `Native` on the
//! predicates beside it are recorded below rather than re-derived, because a reader who
//! takes `Native` to mean *the remote service filters* will read that uniformity as a
//! lie.
//!
//! First, the plugin contract defines `Support::Native` as *the source applies this
//! predicate itself*, and says nothing about where it applies it. What the declaration
//! promises the engine is capability rule 1 — a predicate declared `Native` **is** applied
//! — so that the engine may push it down and apply nothing of its own.
//!
//! Second, this source can keep that promise for every predicate at no additional API
//! cost, because whichever of the three reads below answers a query has already read every
//! item that query could keep before it filters anything. Filtering those items is
//! in-process work over data already in hand.
//!
//! Third, no predicate but `projects` could be pushed into the API even if that were
//! wanted, and `projects` is pushed down: `ProjectV2.items` takes `first` and `after` and
//! offers no filter argument of any kind, GitHub's issue search offers no qualifier for a
//! label set, a status column or a substring of a body, and its title qualifier matches
//! tokens where this source — and the local Markdown source beside it — match substrings,
//! so pushing a search down would silently *narrow* the answer. What a project filter has
//! instead is a relationship: a project's tasks are that issue's sub-issues, and asking
//! the issue for them is both cheaper and exact. So there is one predicate this source
//! applies by asking a narrower question, six it applies in process, and none it is unable
//! to apply. Declaring one `Unsupported` would make the engine compensate for work this
//! source has already done, and declaring `projects` native while ignoring the filter
//! (which this source once did) silently returns another project's tasks, because the
//! engine trusts the declaration and applies nothing locally.
//!
//! # The three ways this source reaches an item, and what each costs
//!
//! A board read is charged for what its *nested* connections could return rather than for
//! what was asked, so one whole-board read costs the same whether the question was about
//! one project or about all of them. That is why a question about one project is never
//! answered by reading the board:
//!
//! | The question | What is sent | What it costs |
//! | --- | --- | --- |
//! | one item, by its own id | [`graphql::ISSUE`] — `node(id:)` | the item |
//! | one project's tasks or documents | [`graphql::SUB_ISSUES`] — that issue's own `subIssues` | that project |
//! | which projects this board holds | [`graphql::SEARCH_ISSUES`] — an issue search scoped to the board | the board's issues, without their board items |
//! | every task, every document, every label | [`graphql::BOARD`] — the board's own `items` | the board |
//!
//! The board half of an issue — its board item's id, its `Status` option and this
//! source's origin text field — rides along on `Issue.projectItems` in the first three, so
//! an item reached any of those ways resolves through the same
//! [`GitHubProjectsSource::resolve`] the board walk uses and reports the same title, the
//! same status, the same labels and the same qualified id. An issue with no entry for
//! *this* board is not this source's to report, which is what keeps an id naming another
//! repository's issue from being answered as an item of this board.
//!
//! **The board's own `Labels` field is not selected in those three, and nothing is lost by
//! that.** The board half they read is a fragment `on Issue`, and an issue's labels are
//! already selected one level up, on the issue itself. A board's `Labels` field is not one
//! anybody fills in: it is a built-in `ProjectV2FieldType`, it is absent from
//! `ProjectV2CustomFieldType` so no project can create one, and `ProjectV2FieldValue` —
//! the whole of what `updateProjectV2ItemFieldValue` accepts — offers no way to write one.
//! For `Issue` content it *is* the issue's own labels, so selecting it beside them unions a
//! set with itself. [`graphql::BOARD`] still selects it and must: a board item's content
//! may be a `DraftIssue`, which has no `labels` of its own to select instead. The four ways
//! an item is reached are held to reporting one label set by
//! `an_item_reports_the_same_labels_title_status_and_id_however_it_is_reached` in
//! `tests/plugin.rs`, which drives each of the four documents against the fixture board.
//!
//! The last row is still the board's own item connection, and deliberately: a **draft**
//! board item is not an issue, so no search and no node read can reach one, and the reads
//! that have to answer for the whole board are the ones whose cost is the board's size
//! anyway.
//!
//! **What a read may return is capped too, and that cap is on the document rather than on
//! the board.** GitHub limits the number of nodes **one query may return** to
//! [`NODE_COUNT_LIMIT`] and refuses a query above that before executing it: the answer is
//! an error naming the connection the count crossed at, not a slow or a partial result.
//! Every board this source reads is refused the same way, so no board is too big for these
//! documents and none is small enough to save one that is over.
//!
//! The count is arithmetic over the document's own text: each connection contributes the
//! `first:` it asks for, counts **multiply** down a nested path and **sum** across sibling
//! paths. Those are [GitHub's published rules][node-limits] and this workspace does not
//! restate them — `github-graphql-node-count` implements them, and
//! [`worst_case_node_count`] under [`largest_page_sizes`] is where every node count here
//! comes from. `every_document_this_source_sends_stays_under_githubs_node_limit`, in
//! `tests/node_count.rs`, recomputes every document in [`graphql::DOCUMENTS`] from that
//! same text on every run and fails naming any that reaches the limit — so a connection
//! added to a shared fragment is caught there rather than by GitHub.
//!
//! What decides those counts is the page sizes: [`MAX_PAGE_SIZE`] on the outer page,
//! `NESTED_PAGE_SIZE` on the connections hanging off one item, and
//! `BOARD_ITEMS_PAGE_SIZE` on an issue's board memberships. `$nestedFirst` is spent twice
//! down one path of a board read, so that constant is effectively squared there, which is
//! why it is the one the limit is most sensitive to.
//!
//! **`nodeCount` and `cost` are two numbers against two limits, and none of this is about
//! the second.** `nodeCount` is the one above: the most nodes one query may return,
//! checked per query. `cost` is rate-limit points, metered per hour across everything one
//! credential does; it is what the two limiters [`Limiter`] tells apart meter, and a
//! document under [`NODE_COUNT_LIMIT`] says nothing about it.
//!
//! [node-limits]: https://docs.github.com/en/graphql/overview/rate-limits-and-node-limits-for-the-graphql-api
//!
//! **Where a read-after-write guarantee comes from, since a search index cannot supply
//! one.** GitHub's issue search is eventually consistent and answers a write made moments
//! ago with the value from before it. Resolving a node id is not, so a read by id and a
//! project's own sub-issues are already current. What closes the gap for the search is
//! [`GitHubProjectsSource::created`]: every read this source answers is completed with
//! what this process itself wrote, so an item created seconds ago is reported whether or
//! not GitHub's index has caught up. Nothing else is remembered, nothing is written down,
//! and the record dies with the process.
//!
//! Filtering happens before paging, so a page of a filtered result is a page of the
//! survivors rather than the survivors of a page. Label and text matching answer the same
//! question the same way the local Markdown source's do, so one cross-source expectation
//! holds for both.
//!
//! <!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The declaration itself
//! has one source, `capabilities`, and the note above is the reasoning behind it rather
//! than a second copy of it: without the three facts recorded here a reader takes the
//! uniform `Native` for a lie and reverts it. The drift gate on the declaration is this
//! crate's own capabilities test, which pins every field of it against a fully spelled-out
//! `Capabilities` literal — a struct with no `Default`, so a field added to the contract
//! fails to compile there rather than going unasserted. -->
//! The fixture-server tests above run wherever this crate is selected; the credentialed
//! lane runs in the same required check, beside them, and can fail it — it verifies the
//! current schema, then drives every field of the table above against the real board. It builds its own fixture there — two projects, one task filed under each,
//! one filed under neither, a label on one of the three and a closed status on another —
//! because that shape is what tells an honoured predicate from an ignored one: a board
//! holding a single project answers a project filter the same way whether or not this
//! source applies it, which is exactly how the defect above went unseen.
//!
//! That lane writes only to the board `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER` name,
//! and only into the repository `GH_PROJECTS_REPOSITORY` names, and skips — as it does
//! without `GH_PROJECTS_TOKEN` — when any of them is absent. Requiring both to be
//! nominated is what keeps a credentialed write lane off a board and a repository nobody
//! nominated; it never asks GitHub which project was updated most recently. Before it
//! starts, the lane also clears any item titled — and any repository label named — the way
//! it titles and names its own artifacts, which is self-healing after an interrupted run:
//! a process killed between its writes and its cleanup leaves artifacts the next run
//! removes.
//!
//! # What a session of requests costs, and where the report is
//!
//! This source records **every** request it sends into [`accounting::Accounting`], at
//! `send_once` — the one place a request leaves this crate, which is why a read path added
//! later is counted without anybody remembering to count it. That is the whole of what this
//! crate adds to the arrangement; [`accounting`] is where what a record carries, how a
//! session's spend is arrived at, and what it deliberately does not know are set out.
//!
//! What one whole session of the live journey costs, counted that way against this crate's
//! loopback fixture board, is written down in `session-cost.md` beside this crate — with the
//! reduction it came out of, and with what it does and does not say about rate-limit points.
//!
//! [`GitHubProjectsSource::accounting`] is the read: a snapshot to hold and compare, which
//! [`accounting::Session::report`] renders the session report from. It is on the ordinary
//! code path — no environment variable, no feature, no build configuration — because an
//! instrument nobody switches on measures nothing, and
//! [`Plugin::build_recording_into`] is how a caller making its own calls beside this
//! source's counts the whole session rather than this source's share. The credentialed lane
//! in `tests/live.rs` does exactly that, and prints the report at the end of every run,
//! passed or failed.
//!
//! **GitHub is the authority on node count, and the credentialed lane goes and asks it.**
//! Everything above computes `nodeCount` offline from a document's own text, which is what
//! lets it run on every platform and on a pull request from a fork with no credential — and
//! that is what actually stops a regression merging. But an offline arithmetic can only
//! ever agree with itself: if GitHub changes its rules, this workspace goes on computing
//! the old answer and nothing notices. So `tests/live.rs` reconciles the two. GitHub's
//! schema exposes `rateLimit(dryRun: true)`, whose `nodeCount` is *"the maximum number of
//! nodes this query may return"* for a document **without executing it**, and the lane asks
//! it for every query document this source sends, under the largest bindings this source
//! sends, and fails when GitHub's number and [`worst_case_node_count`] disagree. It records
//! what those calls reported about the account's own allowance, because whether asking is
//! free is a thing to observe rather than to assume. Two quantities, not one:
//! [`NODE_COUNT_LIMIT`] bounds `nodeCount`, and the accounting above measures `cost`.
//!
//! **GitHub has two rate limiters and this source is refused by both, so nothing here
//! treats them as one thing.** The primary budget is the hourly allowance `gh api
//! rate_limit` reports; the secondary limiter is a burst limiter over content-generating
//! requests, and *nothing* reports it. Which one refused decides the operator's next step,
//! so [`Limiter`] is a type rather than a detail, and it is what [`MIN_MUTATION_INTERVAL_MS`],
//! [`GitHubProjectsSource::board_cache`] and [`GitHubProjectsSource::graphql`] each answer
//! one part of.
#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label, LabelFilter, Location,
    NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery, Repository, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
    TaskSource, TextFields, TextQuery, WriteSupport,
};
use reqwest::{Client, StatusCode, Url};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub mod accounting;

use accounting::Accounting;

/// The registry name for this plugin.
pub const KIND: &str = "github-projects";
/// GitHub's maximum connection page size.
pub const MAX_PAGE_SIZE: u32 = 100;

/// The most nodes any one document this source sends may be asked to return.
///
/// GitHub's own published per-query ceiling, taken from
/// [`github_graphql_node_count::NODE_LIMIT`] rather than written out again here, so this
/// workspace cannot hold a stale copy of somebody else's number. A query above it is
/// **refused before it is executed**, whoever is asking and whatever board they are
/// asking about — so this is a bound on the documents rather than a budget that runs out.
///
/// This is `nodeCount`, the maximum number of nodes *one query may return*. It is not
/// `cost`, the rate-limit points a call spends against an hourly allowance shared by
/// everything the credential does. Two numbers, two limits; nothing here is about the
/// second. The module section on the three ways this source reaches an item says how the
/// count is arrived at, and which of the page sizes below decide it.
pub const NODE_COUNT_LIMIT: u64 = github_graphql_node_count::NODE_LIMIT;

/// Nested connection size for the connections that hang off one item.
///
/// It multiplies through every document that reaches an item under a page — the count
/// rules multiply down a nested path — so it is the constant [`NODE_COUNT_LIMIT`] is most
/// sensitive to. `tests/node_count.rs` is what holds the pair together: it recomputes
/// every document under these constants and fails naming any that reaches the limit, so
/// raising this is caught there rather than by GitHub.
const NESTED_PAGE_SIZE: u32 = 50;
/// How many of one issue's board memberships are read when an issue is reached directly.
///
/// An issue reached through a search or through its own node id carries its board half in
/// `Issue.projectItems`, and only the entry for *this* board is read. Ten is deliberately
/// far smaller than [`NESTED_PAGE_SIZE`]: this connection sits under a page of issues, so
/// its size multiplies through the whole document, and an issue on ten boards at once is
/// already well past what a person keeps track of. An issue whose entry for this board sits
/// past it is refused naming the connection rather than reported as not on the board.
const BOARD_ITEMS_PAGE_SIZE: u32 = 10;

pub use github_graphql_node_count::{NodeCountError, Variables};

/// The largest value this source can bind to each page-size variable its documents name.
///
/// Every `first:` in [`graphql`] reads one of these three, and each is capped at the
/// constant above it wherever a caller's own limit could reach it — `$first` at
/// [`MAX_PAGE_SIZE`], `$nestedFirst` at `NESTED_PAGE_SIZE`, `$boardItems` at
/// `BOARD_ITEMS_PAGE_SIZE`. So this is the worst case a caller can drive this source to,
/// not one configuration of it, which is what makes a bound computed under it a bound on
/// every read.
pub fn largest_page_sizes() -> Variables {
    Variables::from([
        ("first".to_owned(), MAX_PAGE_SIZE),
        ("nestedFirst".to_owned(), NESTED_PAGE_SIZE),
        ("boardItems".to_owned(), BOARD_ITEMS_PAGE_SIZE),
    ])
}

/// The most nodes `document` could be asked to return, by GitHub's published rules.
///
/// Computed offline from the document's own text under [`largest_page_sizes`] — no
/// network, no credential and no schema — by
/// [`github_graphql_node_count::node_count`], which is where the rules themselves live.
/// A document at or above [`NODE_COUNT_LIMIT`] is one GitHub refuses before executing, so
/// this is what a check holds every document in [`graphql::DOCUMENTS`] below.
///
/// # Errors
///
/// Returns the calculation's own [`NodeCountError`] when `document` does not parse, holds
/// no single operation, or binds a page size this source does not name — each of which is
/// a defect in the document rather than a number.
pub fn worst_case_node_count(document: &str) -> Result<u64, NodeCountError> {
    node_count(document, &largest_page_sizes())
}

/// The most nodes `document` could be asked to return under `variables`.
///
/// [`worst_case_node_count`] is this under [`largest_page_sizes`], and the accounting in
/// [`accounting`] is this under the bindings one request really sent — one spelling of the
/// calculation, so a bound checked offline and a cost recorded at run time cannot come to
/// disagree. The rules themselves live in [`github_graphql_node_count::node_count`].
///
/// # Errors
///
/// Returns the calculation's own [`NodeCountError`] when `document` does not parse, holds
/// no single operation, or binds a page size `variables` does not name.
pub fn node_count(document: &str, variables: &Variables) -> Result<u64, NodeCountError> {
    github_graphql_node_count::node_count(document, variables)
}

/// The issue-title prefix that makes a board issue a document.
///
/// A GitHub Projects board has no document type — it holds issues — so the discriminator
/// is the title, and this is the whole of it: an issue whose title begins with these bytes
/// is a document and every other issue is the task or project the sub-issue rule makes it.
///
/// It is spelled **once**, here, and read rather than restated everywhere else — including
/// by the shared journeys, which take it from this constant so a board fixture cannot
/// drift from what this source reads. `docs/metadata.md` records the two consequences that
/// are not obvious from the bytes: the reported title has this prefix taken off, exactly
/// as the body's metadata slot is taken off `content`, and this prefix is read *before*
/// the sub-issue rule, so a design issue with no sub-issues is never an empty project.
pub const DESIGN_TITLE_PREFIX: &str = "DESIGN: ";

/// Exact GraphQL query documents issued by this plugin.
///
/// Keeping the production documents here lets the pinned-schema test validate the same
/// bytes that are sent to GitHub, rather than a test-only copy which could drift
/// independently. No document in this module writes the board itself, and none of them
/// names `updateProjectV2Field`.
pub mod graphql {
    /// Everything this source reads about one issue, wherever it reaches that issue.
    ///
    /// A macro rather than a constant so the three documents below can `concat!` it: one
    /// spelling of these fields is what makes an issue read through the board-scoped
    /// search, through its own node id, and through its project's sub-issue relationship
    /// resolve to *the same* item, which is the whole of what
    /// [`GitHubProjectsSource::resolve_issue`](super::GitHubProjectsSource) relies on.
    ///
    /// `projectItems` is what carries the board half of an issue: the board item's own id
    /// and the field values — the `Status` option and this source's origin text field —
    /// that a `ProjectV2.items` read used to carry. It is asked for on the issue rather
    /// than on the board, which is what makes the cost of a read proportional to what was
    /// asked for instead of to the board's size.
    ///
    /// It does **not** select the board's `Labels` field value, and that is the whole of
    /// what keeps the three documents below under [`NODE_COUNT_LIMIT`](super::NODE_COUNT_LIMIT):
    /// a label connection there sits under `fieldValues` under `projectItems` under a page
    /// of issues, spending `$nestedFirst` twice down one path, and took
    /// [`SEARCH_ISSUES`] and [`SUB_ISSUES`] to 2,556,100 nodes against a limit of 500,000.
    /// No label is lost — this is a fragment `on Issue`, whose own `labels` are selected
    /// above, and a board's `Labels` field is a built-in mirror of exactly those. The
    /// module documentation records why that mirroring holds.
    macro_rules! board_issue {
        () => {
            r#" fragment BoardIssue on Issue{__typename id title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total}
      labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}
      projectItems(first:$boardItems){nodes{id project{number}
        fieldValues(first:$nestedFirst){nodes{
          ... on ProjectV2ItemFieldSingleSelectValue{name field{
            ... on ProjectV2SingleSelectField{id name options{id name}}
          }}
          ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{id name}}}
        }pageInfo{hasNextPage}}}pageInfo{hasNextPage}}}"#
        };
    }

    /// Every issue of one board, found by a search scoped to that board.
    ///
    /// This is how the projects a board holds are listed, and it selects no `items`
    /// connection on `ProjectV2`: the board is a *qualifier of the search* rather than a
    /// container walked page by page, so nothing nested inside a board item is paid for.
    /// Which of the issues it returns is a project is then read off `parent` — GitHub
    /// accepts `-has:parent` as a search qualifier and silently ignores it, so the
    /// discriminator has to be applied to the field, which is a scalar on the issue and
    /// costs nothing.
    pub const SEARCH_ISSUES: &str = concat!(
        r#"query($search:String!,$type:SearchType!,$first:Int!,$after:String,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      search(query:$search,type:$type,first:$first,after:$after){
        pageInfo{hasNextPage endCursor}
        nodes{__typename ...BoardIssue}
      }
    }"#,
        board_issue!()
    );

    /// One issue by its own node id, which is what a qualified id names here.
    ///
    /// Strongly consistent, unlike the search above: GitHub's issue search is an index and
    /// answers a write made moments ago with the value from before it, and resolving a node
    /// id does not.
    pub const ISSUE: &str = concat!(
        r#"query($id:ID!,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      node(id:$id){__typename ...BoardIssue}
    }"#,
        board_issue!()
    );

    /// One project's tasks: the sub-issues of the issue that project is.
    ///
    /// The work this costs is the project's own size. Nothing about it grows as the board
    /// gains projects, or as those projects gain tasks.
    pub const SUB_ISSUES: &str = concat!(
        r#"query($id:ID!,$first:Int!,$after:String,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      node(id:$id){__typename
        ... on Issue{subIssues(first:$first,after:$after){
          pageInfo{hasNextPage endCursor}
          nodes{__typename ...BoardIssue}
        }}}
    }"#,
        board_issue!()
    );

    /// Reads the board's fields and one page of its items.
    pub const BOARD: &str = r#"query($owner:String!,$number:Int!,$first:Int!,$after:String,$nestedFirst:Int!,$duplicates:Boolean!){
      owner:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){...Board}}
      }
    } fragment Board on ProjectV2 { id title
      fields(first:$nestedFirst){nodes{
        ... on ProjectV2SingleSelectField{__typename id name options{id name}}
        ... on ProjectV2Field{__typename id name}
      }pageInfo{hasNextPage}}
      items(first:$first,after:$after){nodes{id fieldValues(first:$nestedFirst){nodes{
        ... on ProjectV2ItemFieldSingleSelectValue{name field{
          ... on ProjectV2SingleSelectField{id name options{id name}}
        }}
        ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{id name}}}
        ... on ProjectV2ItemFieldLabelValue{labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
      }pageInfo{hasNextPage}} content{
        ... on Issue{__typename id title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total} labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        ... on PullRequest{__typename id}
        ... on DraftIssue{__typename id title body createdAt updatedAt}
      }} pageInfo{hasNextPage endCursor}}
    }"#;
    /// Resolves the configured repository's node id, which creating an issue requires.
    pub const REPOSITORY: &str = r#"query($owner:String!,$name:String!){repository(owner:$owner,name:$name){id nameWithOwner}}"#;
    /// Reads both dependency directions for one issue, with each far end's own kind.
    pub const ISSUE_DEPENDENCIES: &str = r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){__typename
      ... on Issue{
        blockedBy(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
        blocking(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
      }}} fragment Related on Issue{id title body parent{id} subIssuesSummary{total}}"#;
    /// Creates one issue in the configured repository.
    pub const CREATE_ISSUE: &str =
        r#"mutation($input:CreateIssueInput!){createIssue(input:$input){issue{id url}}}"#;
    /// Puts an existing issue on the configured board.
    pub const ADD_TO_BOARD: &str = r#"mutation($input:AddProjectV2ItemByIdInput!){addProjectV2ItemById(input:$input){item{id}}}"#;
    /// Updates an issue's visible fields and its open or closed state in one call.
    pub const UPDATE_ISSUE: &str =
        r#"mutation($input:UpdateIssueInput!){updateIssue(input:$input){issue{id}}}"#;
    /// Updates an existing draft's user-visible fields.
    pub const UPDATE_DRAFT: &str = r#"mutation($input:UpdateProjectV2DraftIssueInput!){updateProjectV2DraftIssue(input:$input){draftIssue{id}}}"#;
    /// Updates a text or single-select value on one project item.
    pub const UPDATE_FIELD: &str = r#"mutation($input:UpdateProjectV2ItemFieldValueInput!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id}}}"#;
    /// Files one issue under another as a sub-issue, which is what project membership is.
    pub const ADD_SUB_ISSUE: &str =
        r#"mutation($input:AddSubIssueInput!){addSubIssue(input:$input){issue{id} subIssue{id}}}"#;
    /// Takes one issue back out of its parent.
    pub const REMOVE_SUB_ISSUE: &str = r#"mutation($input:RemoveSubIssueInput!){removeSubIssue(input:$input){issue{id} subIssue{id}}}"#;
    /// Adds GitHub's native issue blocked-by relationship.
    pub const ADD_BLOCKED_BY: &str = r#"mutation($input:AddBlockedByInput!){addBlockedBy(input:$input){issue{id} blockingIssue{id}}}"#;
    /// Removes one native issue blocked-by relationship.
    pub const REMOVE_BLOCKED_BY: &str = r#"mutation($input:RemoveBlockedByInput!){removeBlockedBy(input:$input){issue{id} blockingIssue{id}}}"#;
    /// Deletes one issue, which takes its board item with it.
    ///
    /// The engine sends this in one situation only: undoing a copy that could not finish,
    /// over the items that same copy created. Deleting the issue removes the board item
    /// too, so there is no second `deleteProjectV2Item` to keep in step with it.
    pub const DELETE_ISSUE: &str =
        r#"mutation($input:DeleteIssueInput!){deleteIssue(input:$input){repository{id}}}"#;

    /// Every document above, with what this source is doing when it sends one.
    ///
    /// One list rather than a `match` beside the constants: a rate-limit diagnostic has to
    /// name the call that was refused, and a `match` with a catch-all arm would answer a
    /// document added later with "talking to GitHub" and never say so.
    ///
    /// `documents_are_all_inventoried` reads this file back and fails naming any `pub
    /// const` here that this list omits, so the two cannot part — which is the same guard
    /// `CATEGORIES` carries, in the one shape available to a set of `&str` constants.
    pub const DOCUMENTS: [(&str, &str); 16] = [
        (SEARCH_ISSUES, "searching this board's issues"),
        (ISSUE, "reading one issue"),
        (SUB_ISSUES, "reading a project's tasks"),
        (BOARD, "reading the board"),
        (REPOSITORY, "reading the destination repository"),
        (ISSUE_DEPENDENCIES, "reading an issue's dependencies"),
        (CREATE_ISSUE, "creating an issue"),
        (ADD_TO_BOARD, "adding an issue to the board"),
        (UPDATE_ISSUE, "updating an issue"),
        (UPDATE_DRAFT, "updating a draft item"),
        (UPDATE_FIELD, "writing a board field"),
        (ADD_SUB_ISSUE, "filing an issue under its project"),
        (REMOVE_SUB_ISSUE, "taking an issue out of its project"),
        (ADD_BLOCKED_BY, "recording a dependency"),
        (REMOVE_BLOCKED_BY, "removing a dependency"),
        (DELETE_ISSUE, "deleting an issue"),
    ];
}

/// Which of GitHub's two rate limiters refused a request.
///
/// Waiting is the whole answer to the primary budget, and polling is what *extends* the
/// secondary one — so an operator told the wrong one takes the wrong next step, which is
/// the whole reason this is carried rather than collapsed into "rate limited".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limiter {
    /// The hourly API budget, which `gh api rate_limit` reports and a wait answers.
    Primary,
    /// The burst limiter over content-generating requests, which nothing reports.
    Secondary,
}

/// The wordings GitHub answers a secondary rate limit with.
///
/// It sends them under a forbidden status, under a too-many-requests status, and inside
/// the `errors` of a *successful* response, which is why the text is what this matches on
/// rather than the status. `abuse detection` is the wording GitHub used before the
/// limiter was renamed and still returns from some endpoints; `submitted too quickly` is
/// what a burst of content creation is refused with.
///
/// This is GitHub's vocabulary rather than this source's, so it is pinned rather than
/// remembered: `tests/fixtures/rate-limits.json` records where each wording was read and
/// when, and the drift gate reconciles the two lists both ways. Public for that gate
/// alone — a caller has no use for it, and matching on a refusal is this source's job.
pub const SECONDARY_WORDINGS: [&str; 5] = [
    "secondary rate limit",
    "temporarily blocked from content creation",
    "abuse detection",
    "submitted too quickly",
    "exceeded a secondary",
];

/// The wordings GitHub answers an exhausted primary budget with.
///
/// `rate_limited` is the `type` its GraphQL error carries, which is read as a field rather
/// than looked for in the response text. Pinned and gated exactly as
/// [`SECONDARY_WORDINGS`] is, and public for the same one reason.
pub const PRIMARY_WORDINGS: [&str; 3] = [
    "api rate limit exceeded",
    "rate limit exceeded",
    "rate_limited",
];

/// What a response *says about itself*, which is the only place a refusal can be read.
///
/// Deliberately not the whole response body. A board is a place people write about their
/// own work, and a task on it titled "the secondary rate limit" would, matched across the
/// raw text, turn a perfectly good answer into a refusal this source then waited out and
/// reported. So the item data is never read: what is read is GitHub's own REST-style
/// `message` envelope, which is what a forbidden status carries, and the `message` and
/// `type` of each GraphQL error, which is where a *successful* response says it.
///
/// A body that is not JSON at all has nothing structured to read, so only a failing
/// response's own text is taken — a successful response that is not JSON is malformed
/// rather than refused, and [`GitHubProjectsSource::answer`] says so.
fn refusal_wording(status: StatusCode, body: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return if status.is_success() {
            String::new()
        } else {
            body.to_owned()
        };
    };
    let mut said: Vec<&str> = parsed
        .get("message")
        .and_then(Value::as_str)
        .into_iter()
        .collect();
    if let Some(errors) = parsed.get("errors").and_then(Value::as_array) {
        for error in errors {
            said.extend(
                ["message", "type"]
                    .into_iter()
                    .filter_map(|key| error.get(key).and_then(Value::as_str)),
            );
        }
    }
    said.join("; ")
}

impl Limiter {
    /// Which limiter refused this response, or `None` when none of them did.
    ///
    /// The wording is read first and the status only decides what carries none of it,
    /// because GitHub answers a secondary limit with a forbidden status far more often
    /// than with too-many-requests — while a forbidden status saying nothing about a limit
    /// really is a credential this token lacks.
    ///
    /// A response is a refusal because of its status or its own wording. A spent budget
    /// only ever explains one; it never turns an answer into a refusal.
    fn classify(status: StatusCode, budget_exhausted: bool, body: &str) -> Option<Self> {
        let normalized = refusal_wording(status, body).to_ascii_lowercase();
        if SECONDARY_WORDINGS
            .iter()
            .any(|wording| normalized.contains(wording))
        {
            return Some(Self::Secondary);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Some(Self::Primary);
        }
        // An exhausted budget *explains* a response that failed; it does not make one that
        // succeeded into a failure. GitHub sets `x-ratelimit-remaining: 0` on the last
        // request the budget allowed as well as on the ones it then refuses, so reading
        // the header alone threw away a good answer — and, once refusals were retried,
        // replayed a request that had already taken effect.
        if !status.is_success() && budget_exhausted {
            return Some(Self::Primary);
        }
        // A successful response saying it: GitHub reports a GraphQL rate limit in the
        // `errors` of an HTTP 200, where nothing about the status says so at all.
        if status.is_success()
            && PRIMARY_WORDINGS
                .iter()
                .any(|wording| normalized.contains(wording))
        {
            return Some(Self::Primary);
        }
        None
    }

    /// What this limiter is called where an operator can look it up.
    const fn name(self) -> &'static str {
        match self {
            Self::Primary => "GitHub's primary API rate limit",
            Self::Secondary => "GitHub's secondary rate limit",
        }
    }

    /// What the endpoint an operator would go and check says about this limiter.
    const fn where_to_look(self) -> &'static str {
        match self {
            Self::Primary => {
                "That is the budget `gh api rate_limit` reports, so that endpoint says when it \
                 comes back."
            }
            Self::Secondary => {
                "That limiter is not the primary API budget: `gh api rate_limit` reports the \
                 primary budget and does not report this one, so budget showing there says \
                 nothing about this refusal, and every further attempt extends it."
            }
        }
    }

    /// The next step this limiter actually calls for.
    const fn what_to_do(self) -> &'static str {
        match self {
            Self::Primary => {
                "wait for the reset `gh api rate_limit` reports, then run the command again."
            }
            Self::Secondary => {
                "leave this board alone for a few minutes, then run the command again — or \
                 raise pacing.min_mutation_interval_ms on this source so it writes more slowly."
            }
        }
    }
}

/// One rate-limit refusal, and the wait GitHub asked for if it asked for one.
#[derive(Debug, Clone, Copy)]
struct Limited {
    limiter: Limiter,
    hint: Option<u64>,
}

impl Limited {
    /// What the caller is told once this source has waited as long as it may.
    ///
    /// Both limiters report as [`SourceError::RateLimited`], because that is what
    /// happened: the kind a caller matches on says a rate limit refused this, and nothing
    /// about *which* limiter it was makes it a different kind of failure. What differs is
    /// the operator's next step, and that is what the message carries — a secondary
    /// refusal read as a primary one sends an operator to `gh api rate_limit`, where the
    /// budget looks fine, and then back to retry the very burst that was refused.
    fn exhausted(
        self,
        doing: &str,
        waits: u32,
        waited: Duration,
        needed: Duration,
        budget: Duration,
    ) -> SourceError {
        SourceError::RateLimited {
            retry_after_seconds: self.hint,
            message: Some(format!(
                "{} refused this source while {doing}; it waited {} out over {} and was refused \
                 again, and the next wait of {} would take it past the {} one call may spend \
                 waiting. {} next: {}",
                self.limiter.name(),
                plural(waits, "refusal"),
                seconds(waited),
                seconds(needed),
                seconds(budget),
                self.limiter.where_to_look(),
                self.limiter.what_to_do(),
            )),
        }
    }
}

/// One HTTP attempt's result, with what its response said about the rate limit.
///
/// The two travel together so the record and the outcome are written from the same place:
/// what a response said about the budget is only readable while that response is in hand,
/// and what the attempt *meant* is only decidable once its body has been read.
struct Attempted {
    result: Result<Value, Attempt>,
    limits: accounting::RateLimit,
    /// GitHub's own reported cost for this call, for a document that asked for it.
    reported_cost: Option<u64>,
}

/// One attempt's outcome: an error to report, or a rate limit to wait out.
enum Attempt {
    Failed(SourceError),
    Limited(Limited),
}

fn plural(count: u32, thing: &str) -> String {
    if count == 1 {
        format!("{count} {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

fn seconds(duration: Duration) -> String {
    format!("{:.1}s", duration.as_secs_f64())
}

/// A header GitHub spells as a whole number of seconds, or `None` when this one is not.
///
/// A value that is present and unreadable is deliberately *not* an error. `retry-after` is
/// allowed by HTTP to be a date rather than a count, an intermediary can rewrite either
/// header, and neither is what makes a response a refusal — so the whole cost of one this
/// cannot read is that the refusal carries no hint and the backing-off schedule answers it
/// instead. Refusing the response over the header would turn a readable refusal into an
/// unreadable one, and refusing to *wait* would be the one wrong direction to fail in.
fn whole_seconds(value: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

/// Every mutation this source sends creates content — an issue, a board item, a field of
/// one, a sub-issue link, a dependency — and no query in [`graphql::DOCUMENTS`] does, so
/// what the secondary limiter counts and what the keyword says are the same set. That is
/// what makes the keyword a sound test rather than a convenient one.
fn is_mutation(query: &str) -> bool {
    query.trim_start().starts_with("mutation")
}

/// What this source was doing, for a diagnostic that has to say so.
///
/// Read out of [`graphql::DOCUMENTS`], which is the inventory rather than a copy of it, so
/// a document added without a description is caught by that list's own gate instead of
/// falling through to the vague arm below.
fn operation_description(query: &str) -> &'static str {
    graphql::DOCUMENTS
        .iter()
        .find(|(document, _)| *document == query)
        .map_or("talking to GitHub", |(_, doing)| *doing)
}

/// GitHub's published ceiling on content-generating requests, per minute.
///
/// Pinned in `tests/fixtures/rate-limits.json` and gated against it, because it is
/// GitHub's number rather than this source's: [`MIN_MUTATION_INTERVAL_MS`] is *derived*
/// from it, so a pacing value checked only against itself cannot go stale here.
pub const CONTENT_CREATION_PER_MINUTE: u64 = 80;
/// The same ceiling as GitHub publishes it per hour, which this source does **not** pace
/// at. See [`MIN_MUTATION_INTERVAL_MS`] for why the per-minute bound is the one that
/// governs; it is pinned beside its sibling so the gate would notice either one moving.
pub const CONTENT_CREATION_PER_HOUR: u64 = 500;
/// Shortest interval between two content-creating mutations, in milliseconds.
///
/// GitHub documents two secondary limits on content-generating requests:
/// [`CONTENT_CREATION_PER_MINUTE`] and [`CONTENT_CREATION_PER_HOUR`]. 60000/80 is 750, so
/// a mutation every 750 ms is the fastest rate that cannot exceed the per-minute bound,
/// and that is the bound a copy actually trips: a copy of one plan-sized project is a
/// burst of a few dozen mutations inside a few seconds. The hourly bound works out at one
/// every 7.2 seconds sustained, which no single copy reaches and which, used as the
/// spacing here, would turn an ordinary copy into an hour of waiting — so it is
/// deliberately *not* what this paces at. An installation that wants the hourly bound
/// honoured for a long sequence of copies says so through
/// `pacing.min_mutation_interval_ms`.
pub const MIN_MUTATION_INTERVAL_MS: u64 = 60_000 / CONTENT_CREATION_PER_MINUTE;
/// First wait when a rate-limit refusal carries no hint; each further wait doubles it.
///
/// A doubling schedule from one second reaches a minute in six waits, which is GitHub's
/// own advice for a secondary limit — wait, and wait longer each time — without spending
/// the first minute of a transient refusal doing nothing.
pub const RETRY_BACKOFF_MS: u64 = 1_000;
/// Total time one call may spend waiting out rate limits before it reports a failure.
///
/// Two minutes is long enough to ride out the refusals a paced copy still collects and
/// short enough that a command an operator is watching returns. The bound is what makes
/// the wait a wait rather than a hang: a call refused past it ends in a diagnostic naming
/// the limiter, not in a process nobody can tell from a wedged one.
pub const RETRY_BUDGET_MS: u64 = 120_000;

fn default_token_env() -> String {
    "GH_PROJECTS_TOKEN".to_owned()
}
fn default_endpoint() -> String {
    "https://api.github.com/graphql".to_owned()
}

/// Where one status category lands on this board.
///
/// `null` — an absent value — disables the category for this instance, and using a
/// disabled status is a refusal naming the status and the instance.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum StatusTargetConfig {
    /// The name of a `Status` single-select option already on the board.
    Column(ColumnName),
    /// A closed issue state, whose reason is what tells done from cancelled.
    Closed {
        /// The `IssueClosedStateReason` to close with.
        closed: ClosedState,
    },
}

/// The name of a `Status` single-select option on the board.
///
/// Validated on the way in rather than checked later, so a blank option name — which
/// nothing on a board can be — is a state this type cannot hold.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String")]
pub struct ColumnName(String);

impl ColumnName {
    /// The option name, as the board spells it.
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ColumnName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.trim().is_empty() {
            return Err("a status_mapping option name cannot be blank".to_owned());
        }
        Ok(Self(name))
    }
}

/// The two closed states this product can mean.
///
/// GitHub's `IssueClosedStateReason` also spells `DUPLICATE`, which is neither finished
/// work nor abandoned work, so nothing here ever writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClosedState {
    /// `COMPLETED` — precisely done.
    Completed,
    /// `NOT_PLANNED` — precisely cancelled.
    NotPlanned,
}

impl ClosedState {
    const fn reason(self) -> &'static str {
        match self {
            Self::Completed => "COMPLETED",
            Self::NotPlanned => "NOT_PLANNED",
        }
    }
}

/// Configuration for one GitHub Projects v2 board.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubProjectsConfig {
    /// Login of the user or organization which owns the board.
    pub owner: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates GitHub's owner grammar before private construction.
    /// The project number shown in the board's GitHub URL.
    pub project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` bounds this to a positive GraphQL Int.
    /// `owner/name` of the one repository this source creates its issues in.
    ///
    /// A board has no repository of its own and `createIssue` requires one, so a write
    /// without this is refused naming the field. Reads never need it.
    pub repository: Option<String>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the `owner/name` grammar before private construction.
    /// Environment variable containing a fine-grained token with Projects and Issues
    /// read/write plus Pull requests read-only access for every repository represented on
    /// the board.
    #[serde(default = "default_token_env")]
    pub token_env: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the environment-variable grammar.
    /// GraphQL endpoint. GitHub Enterprise installations may override it.
    #[serde(default = "default_endpoint")]
    pub endpoint: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` converts it to the private validated `Url`.
    /// Per-instance mapping from a status category to where it lands on this board.
    ///
    /// A category this does not mention keeps its shipped default: `backlog` to
    /// "Backlog", `todo` to "Todo", `in-progress` to "In Progress", `done` to closed as
    /// completed, `cancelled` to closed as not planned, and `draft` and `unknown`
    /// disabled.
    #[serde(default)]
    pub status_mapping: BTreeMap<String, Option<StatusTargetConfig>>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` parses each key into a `StatusCategory` and reports an unknown one against this instance.
    /// How fast this source writes, and how long it waits out a rate-limit refusal.
    ///
    /// Every field keeps its shipped default when it is absent, and the defaults are
    /// GitHub's own published limits rather than taste. See [`Pacing`].
    #[serde(default)]
    pub pacing: PacingConfig,
}

/// How fast this source writes, and how long it waits out a rate-limit refusal.
///
/// Configurable because a GitHub Enterprise installation sets its own limits and an
/// operator who has already been refused may want to go slower still — not because the
/// defaults are guesses.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PacingConfig {
    /// Shortest interval between two content-creating mutations, in milliseconds.
    ///
    /// Zero sends them as fast as they are asked for, which is what a fixture server on
    /// loopback wants and what no board on github.com does. At most [`MAX_PACING_MS`].
    pub min_mutation_interval_ms: Option<u64>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `Pacing::resolve` bounds it to `MAX_PACING_MS` before the private validated `Pacing` is built.
    /// First wait when a rate-limit refusal carries no hint, in milliseconds. Each
    /// further wait of the same call doubles it. At most [`MAX_PACING_MS`], and never
    /// zero while there is a budget to spend, because a schedule of zero-length waits
    /// consumes none of it and so never ends.
    pub retry_backoff_ms: Option<u64>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `Pacing::resolve` refuses a non-progressing zero and bounds the rest before the private validated `Pacing` is built.
    /// Total time one call may spend waiting out rate limits, in milliseconds.
    ///
    /// Zero reports the refusal rather than waiting at all. At most [`MAX_PACING_MS`]:
    /// the bound is what makes this a wait rather than a hang.
    pub retry_budget_ms: Option<u64>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `Pacing::resolve` bounds it to `MAX_PACING_MS` before the private validated `Pacing` is built.
}

/// The largest any pacing setting may be, in milliseconds.
///
/// One hour. GitHub's own harshest published bound on content-generating requests works
/// out at one every 7.2 seconds, so an hour is already three orders of magnitude past
/// anything a real limit asks for, and past it the settings stop describing pacing at all:
/// a wait budget beyond it is the unbounded wait this whole mechanism exists to replace,
/// and an interval beyond it is a command that never sends its second mutation. It also
/// keeps the clock arithmetic in [`GitHubProjectsSource::reserve_mutation_slot`] inside
/// what an `Instant` can hold on every platform.
pub const MAX_PACING_MS: u64 = 3_600_000;

/// [`PacingConfig`] with every default resolved and every value checked, which is what the
/// source holds.
#[derive(Debug, Clone, Copy)]
struct Pacing {
    min_mutation_interval: Duration,
    retry_backoff: Duration,
    retry_budget: Duration,
}

impl Pacing {
    /// Resolve one instance's pacing, refusing a configuration that would not pace at all.
    fn resolve(config: PacingConfig, instance: &SourceName) -> Result<Self, SourceError> {
        let bounded = |value: Option<u64>, default: u64, field: &str| match value {
            Some(value) if value > MAX_PACING_MS => Err(SourceError::Config {
                message: format!(
                    "pacing.{field} of source {instance} is {value} ms, and the most any pacing \
                     setting may be is {MAX_PACING_MS} ms — an hour, which is already far past \
                     GitHub's own harshest published limit"
                ),
            }),
            Some(value) => Ok(Duration::from_millis(value)),
            None => Ok(Duration::from_millis(default)),
        };
        let retry_backoff = bounded(
            config.retry_backoff_ms,
            RETRY_BACKOFF_MS,
            "retry_backoff_ms",
        )?;
        let retry_budget = bounded(config.retry_budget_ms, RETRY_BUDGET_MS, "retry_budget_ms")?;
        if retry_backoff.is_zero() && !retry_budget.is_zero() {
            return Err(SourceError::Config {
                message: format!(
                    "pacing.retry_backoff_ms of source {instance} is 0 while \
                     pacing.retry_budget_ms is {} ms; a schedule of zero-length waits spends \
                     none of that budget, so it would retry a refusal forever. Set a backoff of \
                     at least 1 ms, or set retry_budget_ms to 0 to report a refusal without \
                     waiting at all",
                    retry_budget.as_millis()
                ),
            });
        }
        Ok(Self {
            min_mutation_interval: bounded(
                config.min_mutation_interval_ms,
                MIN_MUTATION_INTERVAL_MS,
                "min_mutation_interval_ms",
            )?,
            retry_backoff,
            retry_budget,
        })
    }
}

/// Factory for [`GitHubProjectsSource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn config_schema(&self) -> Schema {
        schema_for!(GitHubProjectsConfig)
    }
    fn build(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        self.build_recording_into(name, config, secrets, Arc::new(Accounting::new()))
    }
}

impl Plugin {
    /// Build a source recording every request it sends into an accounting the caller holds.
    ///
    /// [`SourcePlugin::build`] is this with an accounting of its own, which is what the
    /// registry gets. This is for a caller that is also calling GitHub itself and wants one
    /// session total rather than two — see [`accounting`] and
    /// [`GitHubProjectsSource::recording_into`].
    ///
    /// # Errors
    ///
    /// Exactly [`SourcePlugin::build`]'s, with the same source name in front of each:
    /// [`SourceError::Config`] for configuration this plugin cannot use and
    /// [`SourceError::Auth`] for a credential it cannot find.
    pub fn build_recording_into(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
        ledger: Arc<Accounting>,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: GitHubProjectsConfig =
            serde_json::from_value(config.clone()).map_err(|e| SourceError::Config {
                message: format!("source {name}: {e}"),
            })?;
        let source = GitHubProjectsSource::recording_into(name, config, secrets, ledger).map_err(
            |error| match error {
                SourceError::Config { message } => SourceError::Config {
                    message: format!("source {name}: {message}"),
                },
                SourceError::Auth { message } => SourceError::Auth {
                    message: format!("source {name}: {message}"),
                },
                other => other,
            },
        )?;
        Ok(Box::new(source))
    }
}

/// Where a status category lands on this board, once configuration is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusTarget {
    /// Not usable against this instance.
    Disabled,
    /// The board's `Status` option of this name.
    Column(ColumnName),
    /// A closed issue, with the reason that says which closed it means.
    Closed(ClosedState),
}

/// Every status category, in the order the vocabulary declares them.
///
/// This list mirrors `StatusCategory`, so it carries its own drift gate rather than a
/// reviewer's attention: [`category_position`] is a wildcard-free match, so a variant
/// added to the shared vocabulary fails to compile until it is named there, and this
/// crate's suite reconciles this list against that enum's own derived schema, which is
/// generated from the variants rather than written beside them. The schema is what
/// catches a list left one short — a list checking only the positions it already holds
/// would pass while every mapping indexed by the new position panicked.
pub const CATEGORIES: [StatusCategory; 7] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::InProgress,
    StatusCategory::Done,
    StatusCategory::Cancelled,
    StatusCategory::Unknown,
];

/// Where one category sits in [`CATEGORIES`]; see that list for what this pins.
#[must_use]
pub const fn category_position(category: StatusCategory) -> usize {
    match category {
        StatusCategory::Draft => 0,
        StatusCategory::Backlog => 1,
        StatusCategory::Todo => 2,
        StatusCategory::InProgress => 3,
        StatusCategory::Done => 4,
        StatusCategory::Cancelled => 5,
        StatusCategory::Unknown => 6,
    }
}

/// The spelling a status category is configured and reported under.
fn category_name(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Draft => "draft",
        StatusCategory::Backlog => "backlog",
        StatusCategory::Todo => "todo",
        StatusCategory::InProgress => "in-progress",
        StatusCategory::Done => "done",
        StatusCategory::Cancelled => "cancelled",
        StatusCategory::Unknown => "unknown",
    }
}

/// A shipped default's option name.
///
/// The literals below are this file's own and non-blank, and they are validated by the
/// one constructor a configured name goes through rather than beside it.
fn shipped_column(name: &'static str) -> ColumnName {
    ColumnName::try_from(name.to_owned()).expect("a shipped default names a board option")
}

/// The shipped default for one category, before this instance's configuration.
fn shipped_default(category: StatusCategory) -> StatusTarget {
    match category {
        StatusCategory::Backlog => StatusTarget::Column(shipped_column("Backlog")),
        StatusCategory::Todo => StatusTarget::Column(shipped_column("Todo")),
        StatusCategory::InProgress => StatusTarget::Column(shipped_column("In Progress")),
        StatusCategory::Done => StatusTarget::Closed(ClosedState::Completed),
        StatusCategory::Cancelled => StatusTarget::Closed(ClosedState::NotPlanned),
        StatusCategory::Draft | StatusCategory::Unknown => StatusTarget::Disabled,
    }
}

/// This instance's complete category-to-target mapping, read in both directions.
///
/// One target per category, held at that category's own [`category_position`], so a
/// category missing from the mapping, named twice in it, or filed out of order is a
/// state this type cannot hold rather than one [`Self::target`] has to defend against.
#[derive(Debug, Clone)]
struct StatusMapping {
    targets: [StatusTarget; CATEGORIES.len()],
}

impl StatusMapping {
    fn resolve(
        configured: BTreeMap<String, Option<StatusTargetConfig>>,
        instance: &SourceName,
    ) -> Result<Self, SourceError> {
        let mut overrides: BTreeMap<&'static str, Option<StatusTargetConfig>> = BTreeMap::new();
        for (key, value) in configured {
            let category = CATEGORIES
                .iter()
                .find(|category| category_name(**category) == key)
                .ok_or_else(|| SourceError::Config {
                    message: format!(
                        "status_mapping names {key:?}, which is not a status category of source \
                         {instance}; the categories are {}",
                        CATEGORIES
                            .iter()
                            .map(|category| category_name(*category))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
            overrides.insert(category_name(*category), value);
        }
        // `CATEGORIES[position] == category` for every category — the crate's suite
        // asserts it — so mapping the list in order fills each category's own slot.
        let targets = CATEGORIES.map(|category| match overrides.remove(category_name(category)) {
            None => shipped_default(category),
            Some(None) => StatusTarget::Disabled,
            Some(Some(StatusTargetConfig::Column(option))) => StatusTarget::Column(option),
            Some(Some(StatusTargetConfig::Closed { closed })) => StatusTarget::Closed(closed),
        });
        let mapping = Self { targets };
        for (index, category) in CATEGORIES.into_iter().enumerate() {
            let StatusTarget::Column(option) = mapping.target(category) else {
                continue;
            };
            if let Some(other) = CATEGORIES[..index].iter().find(|earlier| {
                matches!(mapping.target(**earlier), StatusTarget::Column(name)
                    if name.as_str().eq_ignore_ascii_case(option.as_str()))
            }) {
                return Err(SourceError::Config {
                    message: format!(
                        "status_mapping of source {instance} sends both {} and {} to the board \
                         option {:?}; one option cannot read back as two categories",
                        category_name(*other),
                        category_name(category),
                        option.as_str()
                    ),
                });
            }
        }
        Ok(mapping)
    }

    fn target(&self, category: StatusCategory) -> &StatusTarget {
        &self.targets[category_position(category)]
    }

    /// The category a board option name reports, or `None` when nothing maps to it.
    fn category_of(&self, option: &str) -> Option<StatusCategory> {
        CATEGORIES.into_iter().find(|category| {
            matches!(self.target(*category), StatusTarget::Column(name)
                if name.as_str().eq_ignore_ascii_case(option))
        })
    }
}

/// The one repository this source creates issues in.
#[derive(Debug, Clone)]
struct RepositoryTarget {
    owner: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only after `owner/name` validation in `new`.
    name: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only after `owner/name` validation in `new`.
}

impl RepositoryTarget {
    fn parse(value: &str) -> Result<Self, SourceError> {
        let (owner, name) = value.split_once('/').ok_or_else(|| SourceError::Config {
            message: format!(
                "repository must be spelled owner/name; {value:?} names no repository"
            ),
        })?;
        if !valid_github_owner(owner) || !valid_github_repository_name(name) {
            return Err(SourceError::Config {
                message: format!(
                    "repository must be spelled owner/name with a GitHub login and one \
                     repository name; {value:?} is not"
                ),
            });
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    fn origin(&self) -> String {
        format!("github.com/{}/{}", self.owner, self.name)
    }
}

/// A source which reads GitHub afresh for every operation.
pub struct GitHubProjectsSource {
    /// This source's configured name, used both to tell a far end naming this source
    /// from one naming a system it knows nothing about, and to name the instance a
    /// status refusal is about.
    name: SourceName,
    owner: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after full GitHub-owner validation.
    project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after GraphQL-Int validation.
    repository: Option<RepositoryTarget>,
    endpoint: Url,
    token: SecretString,
    credential_name: String, // llmlint: ignore[invalid_states_unrepresentable] Private diagnostic value constructed only after environment-name validation.
    statuses: StatusMapping,
    client: Client,
    /// Every item this source has created since it was built, in the order it created
    /// them.
    ///
    /// GitHub's `projectV2.items` is eventually consistent: an issue added to a board with
    /// `addProjectV2ItemById` is routinely absent from the very next read of that board, so
    /// a copy resolving a dependency on an item it had just created refused it as not
    /// found. A board read is completed from this — an item remembered here and absent from
    /// the read is added back, because the board really does hold it and only the read is
    /// behind.
    ///
    /// It is not a cache of a user's work: nothing is remembered that this process did not
    /// itself just write, it lives and dies with the process, and it is never consulted for
    /// an item this source did not create.
    created: Mutex<Vec<Resolved>>,
    /// How fast this source writes, and how long it waits out a refusal.
    pacing: Pacing,
    /// When the last content-creating mutation finished, or the moment the furthest-out
    /// reserved slot releases the next one, whichever is later — so the one after it can be
    /// spaced from that. See [`MIN_MUTATION_INTERVAL_MS`] for the interval and
    /// [`GitHubProjectsSource::finish_mutation`] for why completion rather than release is
    /// what it is measured from.
    last_mutation: Mutex<Option<Instant>>,
    /// The board as this process last read it, for the length of one command.
    ///
    /// A copy of a project used to re-read the whole board, paged, before writing each of
    /// its items, which is by far the largest part of a copy's request count and none of
    /// its work. Nothing else changes this board while a command runs — this source's own
    /// writes are the only writer — so one read answers them all.
    ///
    /// It is not a store of a user's work and it is not the cache the no-persistence
    /// invariant forbids: it lives and dies with the process exactly as `created` does,
    /// nothing is written down, and [`Self::board`] still completes it from `created`, so
    /// an item this command created and then depends on resolves whether or not GitHub's
    /// own eventually-consistent read has caught up. A write to an item already on the
    /// board updates the entry here too, so what this holds is the last read plus this
    /// process's own writes rather than a snapshot taken before them.
    board_cache: Mutex<Option<Board>>,
    /// The destination repository's node id, resolved once rather than per issue created.
    ///
    /// A repository's node id does not change, and re-reading it for every issue of a copy
    /// spent one request per item on an answer this source already had.
    repository_cache: Mutex<Option<String>>,
    /// What every request this source sends is recorded into.
    ///
    /// Ordinary code path, not a mode: [`Self::send_once`] records into it at the one place
    /// a request leaves this crate, so nothing has to be switched on for a session to be
    /// counted. It is shared rather than owned so a caller accounting for a whole session —
    /// its own schema verification, board lookups, residue sweep and cleanup beside this
    /// source's reads and writes — adds up one accounting instead of two. See
    /// [`accounting`] for what a record carries and what a session's spend is and is not.
    ledger: Arc<Accounting>,
}

impl GitHubProjectsSource {
    /// Validate configuration and capture the named credential without exposing it.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Config`] for a configuration this instance cannot use and
    /// [`SourceError::Auth`] when the named credential is missing or empty.
    pub fn new(
        name: &SourceName,
        config: GitHubProjectsConfig,
        secrets: &dyn SecretResolver,
    ) -> Result<Self, SourceError> {
        Self::recording_into(name, config, secrets, Arc::new(Accounting::new()))
    }

    /// The same, recording every request it sends into an accounting the caller holds too.
    ///
    /// [`Self::new`] is this with an accounting of its own. A caller that is also making
    /// its own calls to GitHub — a lane verifying a schema, sweeping residue or cleaning
    /// up — passes the one it records those into, so the session total accounts for the
    /// whole session rather than for this source's share of it.
    ///
    /// # Errors
    ///
    /// Exactly [`Self::new`]'s: [`SourceError::Config`] for a configuration this instance
    /// cannot use and [`SourceError::Auth`] when the named credential is missing or empty.
    pub fn recording_into(
        name: &SourceName,
        config: GitHubProjectsConfig,
        secrets: &dyn SecretResolver,
        ledger: Arc<Accounting>,
    ) -> Result<Self, SourceError> {
        if !valid_github_owner(&config.owner) {
            return Err(SourceError::Config {
                message: "owner must be 1-39 ASCII letters, digits, or single hyphens, and cannot start or end with a hyphen".into(),
            });
        }
        if config.project_number == 0 || config.project_number > i32::MAX as u32 {
            return Err(SourceError::Config {
                message: format!("project_number must be between 1 and {}", i32::MAX),
            });
        }
        if !valid_environment_name(&config.token_env) {
            return Err(SourceError::Config {
                message: "token_env must be a valid environment-variable name".into(),
            });
        }
        let repository = config
            .repository
            .as_deref()
            .map(RepositoryTarget::parse)
            .transpose()?;
        let endpoint = Url::parse(&config.endpoint).map_err(|e| SourceError::Config {
            message: format!("endpoint is not a valid URL: {e}"),
        })?;
        if endpoint.scheme() != "https"
            && !(endpoint.scheme() == "http"
                && endpoint
                    .host_str()
                    .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "::1"))
        {
            return Err(SourceError::Config {
                message:
                    "endpoint must use HTTPS (HTTP is accepted only for a loopback test server)"
                        .into(),
            });
        }
        let token = secrets.get(&config.token_env).filter(|token| !token.expose_secret().trim().is_empty()).ok_or_else(|| SourceError::Auth {
            message: format!("environment variable {} is missing or empty; set it to a fine-grained GitHub token granting Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board", config.token_env),
        })?;
        Ok(Self {
            name: name.clone(),
            owner: config.owner,
            project_number: config.project_number,
            repository,
            endpoint,
            token,
            credential_name: config.token_env,
            statuses: StatusMapping::resolve(config.status_mapping, name)?,
            client: Client::builder()
                .user_agent("onetaskgraph")
                .build()
                .map_err(|e| SourceError::Config {
                    message: format!("cannot build HTTP client: {e}"),
                })?,
            created: Mutex::new(Vec::new()),
            pacing: Pacing::resolve(config.pacing, name)?,
            last_mutation: Mutex::new(None),
            board_cache: Mutex::new(None),
            repository_cache: Mutex::new(None),
            ledger,
        })
    }

    /// A snapshot of every request this source has sent, and what each cost.
    ///
    /// A value to hold and compare rather than a borrow of the accounting itself, so two
    /// of them can sit side by side. When this source was built with
    /// [`Self::recording_into`] the snapshot is the whole shared session, which is the
    /// point of building it that way.
    #[must_use]
    pub fn accounting(&self) -> accounting::Session {
        self.ledger.snapshot()
    }

    /// Send one GraphQL document, pacing this source's own mutations and waiting out a
    /// rate limit rather than handing it straight back as an error.
    ///
    /// Retrying is safe for every document here, including the mutations, and the reason
    /// is that only a *refusal* is retried: [`Limiter::classify`] rules on a response
    /// GitHub sent, and a request GitHub refused for a rate limit did not run, so nothing
    /// this replays has already taken effect. An outcome this source cannot know — the
    /// send failed, or the body could not be read, so the mutation may well have landed —
    /// is [`Attempt::Failed`] in [`send_once`] and leaves this loop without a second
    /// attempt. A duplicate write would come from replaying one of those, and none is
    /// replayed.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, SourceError> {
        let doing = operation_description(query);
        let mut waited = Duration::ZERO;
        let mut waits = 0_u32;
        let mut backoff = self.pacing.retry_backoff;
        loop {
            if is_mutation(query) {
                let spacing = self.reserve_mutation_slot();
                if !spacing.is_zero() {
                    tokio::time::sleep(spacing).await;
                }
            }
            let attempt = self.send_once(query, &variables).await;
            if is_mutation(query) {
                self.finish_mutation();
            }
            let limited = match attempt {
                Ok(data) => return Ok(data),
                Err(Attempt::Failed(error)) => return Err(error),
                Err(Attempt::Limited(limited)) => limited,
            };
            // GitHub really does send `retry-after: 0`, and retrying at once is the one
            // move that extends a secondary limit, so a hint below the schedule's own next
            // wait is raised to it.
            let wait = match limited.hint {
                Some(hint) => Duration::from_secs(hint).max(backoff),
                None => backoff,
            };
            let remaining = self.pacing.retry_budget.saturating_sub(waited);
            // A wait of nothing spends none of the budget, so it is exhaustion rather
            // than a retry. `Pacing::resolve` rules out every way of configuring one
            // except a budget of zero, where reporting the first refusal is the ask.
            if wait.is_zero() || wait > remaining {
                return Err(limited.exhausted(
                    doing,
                    waits,
                    waited,
                    wait,
                    self.pacing.retry_budget,
                ));
            }
            tokio::time::sleep(wait).await;
            waited += wait;
            waits += 1;
            backoff = backoff.saturating_mul(2);
        }
    }

    /// The next moment a content-creating mutation may leave this source, as a wait from
    /// now.
    ///
    /// The slot is reserved under the lock and the waiting happens outside it, so two
    /// callers take two slots rather than the same one — and no lock is held across an
    /// await.
    ///
    /// The moment it is spaced from is the previous mutation's *completion*, which
    /// [`Self::finish_mutation`] records. See that method for why the release moment on its
    /// own is the wrong thing to measure from.
    fn reserve_mutation_slot(&self) -> Duration {
        if self.pacing.min_mutation_interval.is_zero() {
            return Duration::ZERO;
        }
        // A poisoned lock here costs pacing, not correctness, and refusing the write over
        // it would turn an earlier failure into a second one for no gain.
        let mut last = self
            .last_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        // `checked_add` rather than `+`: `Instant + Duration` panics on overflow, and
        // pacing is not worth a panic even at a bound `MAX_PACING_MS` already rules out.
        let at = last.map_or(now, |previous| {
            previous
                .checked_add(self.pacing.min_mutation_interval)
                .map_or(now, |earliest| earliest.max(now))
        });
        *last = Some(at);
        at.saturating_duration_since(now)
    }

    /// Record that a content-creating mutation has finished, so the next one is spaced
    /// from here rather than from the moment this one was released.
    ///
    /// This source can only choose when a request *departs*; the limiter counts when it
    /// *arrives*, and the two differ by whatever the request spent in transit. Spacing one
    /// departure from the last therefore hands the limiter a gap of the interval less that
    /// transit, so a source pacing at 750 ms can still be seen arriving faster — which is
    /// exactly how a copy paced well inside a board's threshold was refused by it on a
    /// slower machine while passing on a quick one.
    ///
    /// Spacing from completion removes the subtraction rather than budgeting for it. The
    /// previous request had already arrived before its response came back, so its arrival
    /// is no later than this moment, and the next mutation is released at least the
    /// interval after this moment and arrives no earlier than it is released: the gap the
    /// limiter measures is therefore at least the interval, whatever transit costs and on
    /// whatever platform. The price is that a mutation's own round trip no longer counts
    /// towards its spacing, which makes this source slightly slower than the configured
    /// rate rather than slightly faster — the safe side of a limit that punishes being
    /// wrong by refusing reads for the next fifty minutes.
    ///
    /// A failed attempt is recorded too: a request refused by the limiter still arrived,
    /// and one that never left costs only a wait nobody needed.
    fn finish_mutation(&self) {
        if self.pacing.min_mutation_interval.is_zero() {
            return;
        }
        // A poisoned lock here costs pacing, not correctness, exactly as in the reservation.
        let mut last = self
            .last_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        // `max` rather than an assignment: a concurrent caller may already have reserved a
        // slot further out, and completing this request must never pull that slot back in.
        *last = Some(last.map_or(now, |reserved| reserved.max(now)));
    }

    /// One HTTP attempt, classified into an answer, a rate limit to wait out, or a
    /// failure that waiting cannot help — and recorded, whichever of the three it was.
    ///
    /// This is the one place a request leaves this crate, which is why the accounting is
    /// here rather than at each of the callers: a read path added later is counted without
    /// anybody remembering to count it, and `an_accounting_records_every_request_the_board_served`
    /// fails when one is not.
    async fn send_once(&self, query: &str, variables: &Value) -> Result<Value, Attempt> {
        let Attempted {
            result,
            limits,
            reported_cost,
        } = self.attempt(query, variables).await;
        // No `otherwise` name: every document this source sends is one of its own, and the
        // inventory gate on `graphql::DOCUMENTS` is what keeps that true.
        let sending = accounting::Request::graphql(query, variables, None, reported_cost);
        let outcome = match &result {
            Ok(_) => accounting::Outcome::Answered,
            Err(Attempt::Limited(_)) => accounting::Outcome::RateLimited,
            Err(Attempt::Failed(_)) => accounting::Outcome::Refused,
        };
        self.ledger.record(sending.finished(outcome, limits));
        result
    }

    /// The attempt itself, with what its response said about the rate limit alongside.
    ///
    /// The two are returned together rather than recorded here because every one of the
    /// early exits below is a different outcome, and a record written at each of them is a
    /// record one of them can be added without.
    async fn attempt(&self, query: &str, variables: &Value) -> Attempted {
        let mut limits = accounting::RateLimit::default();
        let mut reported_cost = None;
        let result = self
            .attempted(query, variables, &mut limits, &mut reported_cost)
            .await;
        Attempted {
            result,
            limits,
            reported_cost,
        }
    }

    /// One HTTP attempt, filling in what its response said about the rate limit as it goes.
    async fn attempted(
        &self,
        query: &str,
        variables: &Value,
        limits: &mut accounting::RateLimit,
        reported_cost: &mut Option<u64>,
    ) -> Result<Value, Attempt> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.token.expose_secret())
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await
            .map_err(|e| {
                Attempt::Failed(SourceError::Unavailable {
                    message: format!("GitHub GraphQL request failed: {e}"),
                })
            })?;
        let status = response.status();
        let header = |name: &str| whole_seconds(response.headers().get(name));
        *limits = accounting::RateLimit::read(|name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        // Exactly `0` is exhaustion and everything else — a count, an empty value, bytes
        // that are not text at all — is "not known to be exhausted". This never makes a
        // response a refusal on its own: it says which limiter a refusal is attributed to
        // and where its hint comes from, so a value this cannot read costs a hint rather
        // than an answer.
        let exhausted = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            == Some("0");
        // `retry-after` is what GitHub asks for when it asks; when it does not and the
        // primary budget is spent, `x-ratelimit-reset` says when that budget comes back,
        // which is the same question answered as an absolute time. Nothing else here is a
        // hint, and a schedule is what answers a refusal that carries none.
        let hint = header("retry-after").or_else(|| {
            exhausted
                .then(|| header("x-ratelimit-reset"))
                .flatten()
                .map(|reset| reset.saturating_sub(Utc::now().timestamp().max(0).unsigned_abs()))
        });
        // Read before it is parsed, because the evidence which tells a secondary rate
        // limit from a rejected credential is in the body of a response whose status says
        // only "forbidden" — and a non-success response was never parsed at all.
        let body = response.text().await.map_err(|e| {
            Attempt::Failed(SourceError::Unavailable {
                message: format!("GitHub GraphQL response could not be read: {e}"),
            })
        })?;
        if let Some(limiter) = Limiter::classify(status, exhausted, &body) {
            return Err(Attempt::Limited(Limited { limiter, hint }));
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(Attempt::Failed(SourceError::Auth {
                message: format!(
                    "GitHub rejected the configured credential with HTTP {status}; grant it Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board"
                ),
            }));
        }
        if !status.is_success() {
            return Err(Attempt::Failed(SourceError::Unavailable {
                message: format!("GitHub GraphQL returned HTTP {status}"),
            }));
        }
        // GitHub reports what a call cost only when the document asked it to, and no
        // document this source sends does — so this is `None` here and carries the figure
        // for a caller whose own document selects `rateLimit { cost }`. What it must never
        // pick up is a `dryRun` probe's cost, which is some other document's.
        *reported_cost = serde_json::from_str::<Value>(&body)
            .ok()
            .as_ref()
            .and_then(|body| body.pointer("/data/rateLimit/cost"))
            .and_then(Value::as_u64);
        self.answer(&body).map_err(Attempt::Failed)
    }

    /// What one successful HTTP response says, once its GraphQL errors are read.
    fn answer(&self, body: &str) -> Result<Value, SourceError> {
        let body: Value = serde_json::from_str(body).map_err(|e| SourceError::Malformed {
            message: format!("GitHub returned invalid JSON: {e}"),
        })?;
        let errors = body
            .get("errors")
            .map(|value| {
                value.as_array().ok_or_else(|| SourceError::Malformed {
                    message: "GitHub response errors is not an array".into(),
                })
            })
            .transpose()?;
        if let Some(errors) = errors.filter(|errors| !errors.is_empty()) {
            let messages = errors
                .iter()
                .filter_map(|e| e.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            let message = if messages.is_empty() {
                "GitHub returned GraphQL errors".into()
            } else {
                messages
            };
            let normalized = message.to_ascii_lowercase();
            if normalized.contains("resource not accessible") || normalized.contains("scope") {
                return Err(SourceError::Auth {
                    message: format!(
                        "{message}; grant {} Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board",
                        self.credential_name
                    ),
                });
            }
            return Err(SourceError::Refused { message });
        }
        body.get("data")
            .filter(|data| data.is_object())
            .cloned()
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub response has no data object".into(),
            })
    }

    // llmlint: ignore[boundary_inputs_validated] GitHub caps nested connections at 100 and
    // GraphQL cannot independently page them inside the outer item page. This source page is
    // deliberately bounded at that published maximum; the live drift journey exercises it.
    async fn board_page(
        &self,
        items_after: Option<&str>,
        items_first: u32,
    ) -> Result<Value, SourceError> {
        let data = self
            .graphql(
                graphql::BOARD,
                json!({"owner":self.owner,"number":self.project_number,
                       "first":items_first.min(MAX_PAGE_SIZE),"after":items_after,
                       "nestedFirst":NESTED_PAGE_SIZE,"duplicates":true}),
            )
            .await?;
        data.pointer("/owner/projectV2")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub project {}/{} was not found or is not visible to the token",
                    self.owner, self.project_number
                ),
            })
    }

    /// The search that finds the issues of this board, narrowed by `also` when it is
    /// given.
    ///
    /// `project:owner/number` is what scopes a search to one board, and `is:issue` is what
    /// keeps pull requests out of it: GitHub's `ISSUE` search type covers both, and a pull
    /// request is somebody's change rather than a unit of plan. `-has:parent` is *not*
    /// here on purpose — GitHub accepts it and silently ignores it, so a project is told
    /// from a task by the `parent` field each issue carries rather than by the search.
    fn board_search(&self, also: Option<&str>) -> String {
        let scope = format!("project:{}/{} is:issue", self.owner, self.project_number);
        match also {
            Some(also) => format!("{scope} {also}"),
            None => scope,
        }
    }

    /// One issue this source reached directly, as the board item a read of the board would
    /// have produced — or `None` when this board does not hold it.
    ///
    /// The board half of an issue rides along on `Issue.projectItems`, so the value handed
    /// to [`Self::resolve`] is the very shape a `ProjectV2.items` read gives it: the board
    /// item's own id, that item's field values, and the issue as its content. One resolver
    /// for both routes is what makes an issue read through a search, through its own node
    /// id, or through its project's sub-issues report the same title, the same status, the
    /// same labels and the same qualified id.
    ///
    /// An issue with no entry for *this* board is not this source's to report, which is
    /// what keeps an id naming some other repository's issue from being answered as an item
    /// of this board.
    fn resolve_issue(&self, issue: &Value) -> Result<Option<Resolved>, SourceError> {
        if optional_str(issue, "__typename")? != Some("Issue") {
            return Ok(None);
        }
        let memberships = issue
            .get("projectItems")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue is missing projectItems".into(),
            })?;
        let nodes = memberships
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue projectItems.nodes is not an array".into(),
            })?;
        let held = nodes.iter().find(|node| {
            node.pointer("/project/number").and_then(Value::as_u64)
                == Some(u64::from(self.project_number))
        });
        let Some(held) = held else {
            // Only now: an issue whose entry for this board sits past the page asked for
            // would otherwise read as an issue this board does not hold, which is the one
            // wrong answer available here.
            complete_connection(
                memberships,
                "issue board memberships",
                BOARD_ITEMS_PAGE_SIZE,
            )?;
            return Ok(None);
        };
        let item = json!({
            "id": required_str(held, "id")?,
            "fieldValues": held.get("fieldValues"),
            "content": issue,
        });
        self.resolve(&item)
    }

    /// One page of a board-scoped issue search, and where the next page resumes.
    async fn search_page(
        &self,
        search: &str,
        first: u32,
        after: Option<&str>,
    ) -> Result<(Vec<Resolved>, Option<String>), SourceError> {
        let data = self
            .graphql(
                graphql::SEARCH_ISSUES,
                json!({"search":search,"type":"ISSUE","first":first.min(MAX_PAGE_SIZE),
                       "after":after,"nestedFirst":NESTED_PAGE_SIZE,
                       "boardItems":BOARD_ITEMS_PAGE_SIZE,"duplicates":true}),
            )
            .await?;
        let connection = data.get("search").ok_or_else(|| SourceError::Malformed {
            message: "GitHub search response has no search connection".into(),
        })?;
        let mut found = Vec::new();
        for node in connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub search nodes is not an array".into(),
            })?
        {
            if let Some(resolved) = self.resolve_issue(node)? {
                found.push(resolved);
            }
        }
        let info = connection
            .get("pageInfo")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub search connection has no pageInfo".into(),
            })?;
        let next = required_bool(info, "hasNextPage")?
            .then(|| required_str(info, "endCursor"))
            .transpose()?
            .map(str::to_owned);
        if let Some(next) = &next {
            validate_cursor_progress(after, next)?;
        }
        Ok((found, next))
    }

    /// Every issue this board holds, walked to exhaustion, completed with what this run
    /// wrote.
    ///
    /// The completion is not an optimisation and it is not a cache: GitHub's issue search
    /// is an index and is eventually consistent, so an issue this run created seconds ago
    /// is routinely absent from it, and a project listed straight after being written would
    /// otherwise be missing from its own board. What is added back is only what this
    /// process itself wrote, out of [`Self::created`], which lives and dies with the
    /// process.
    async fn board_issues(&self) -> Result<Vec<Resolved>, SourceError> {
        let mut after: Option<String> = None;
        let mut found = Vec::new();
        let search = self.board_search(None);
        loop {
            let (page, next) = self
                .search_page(&search, MAX_PAGE_SIZE, after.as_deref())
                .await?;
            found.extend(page);
            match next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        self.completed_with_written(found, |_| true)
    }

    /// `found`, with everything this run wrote that `keep` accepts and the read did not
    /// report.
    ///
    /// See [`Self::created`] and [`Self::board_issues`] for why a read has to be completed
    /// at all: the search index is behind, and a node read of an item filed moments ago can
    /// be too.
    fn completed_with_written(
        &self,
        mut found: Vec<Resolved>,
        keep: impl Fn(&Resolved) -> bool,
    ) -> Result<Vec<Resolved>, SourceError> {
        for own in self.created()?.iter().filter(|own| keep(own)) {
            if !found.iter().any(|item| item.id == own.id) {
                found.push(own.clone());
            }
        }
        Ok(found)
    }

    /// What resolving one node id reached.
    ///
    /// Three answers rather than an `Option`, because a board *draft* is none of the other
    /// two: it is not an issue, it has no node of its own this source can read the board
    /// half off, and its only home is the board's own item connection — so a read of one
    /// is completed from there rather than reported as nothing.
    async fn reach(&self, id: &NativeId) -> Result<Reached, SourceError> {
        let asked = self
            .graphql(
                graphql::ISSUE,
                json!({"id":id.0,"nestedFirst":NESTED_PAGE_SIZE,
                       "boardItems":BOARD_ITEMS_PAGE_SIZE,"duplicates":true}),
            )
            .await;
        let data = match asked {
            Ok(data) => data,
            // A string that is not a node id at all is not a failure to report: it is an id
            // this board does not hold, which is what every read of one already answers.
            Err(error) if unresolvable_node(&error) => return Ok(Reached::Nothing),
            Err(error) => return Err(error),
        };
        let Some(node) = data.get("node").filter(|value| !value.is_null()) else {
            return Ok(Reached::Nothing);
        };
        if optional_str(node, "__typename")? == Some("DraftIssue") {
            return Ok(Reached::Draft);
        }
        Ok(match self.resolve_issue(node)? {
            Some(item) => Reached::Held(Box::new(item)),
            None => Reached::Nothing,
        })
    }

    /// One item of this board by its own id, whatever kind it is.
    ///
    /// Resolved from the identifier alone: no search, board-wide or otherwise. What this
    /// run wrote is read first, because a node read of an item created moments ago can
    /// still be behind the board field values written onto it — see [`Self::created`].
    async fn item_by_id(&self, id: &NativeId) -> Result<Option<Resolved>, SourceError> {
        if let Some(own) = self.created()?.iter().find(|own| own.id == *id) {
            return Ok(Some(own.clone()));
        }
        match self.reach(id).await? {
            Reached::Held(item) => Ok(Some(*item)),
            Reached::Nothing => Ok(None),
            // The one read that still costs the board: a draft lives nowhere else.
            Reached::Draft => Ok(self
                .board()
                .await?
                .items
                .into_iter()
                .find(|item| item.id == *id)),
        }
    }

    /// Everything filed under one issue of this board, walked to exhaustion — or `None`
    /// when that id names nothing here with a sub-issue relationship to walk.
    ///
    /// `None` and an empty answer are different: `None` is *this is not an issue of this
    /// GitHub*, which is what sends a project selector on to be read as a name, and an
    /// empty vector is a project that holds nothing.
    async fn sub_issues(&self, id: &NativeId) -> Result<Option<Vec<Resolved>>, SourceError> {
        let mut after: Option<String> = None;
        let mut children = Vec::new();
        loop {
            let asked = self
                .graphql(
                    graphql::SUB_ISSUES,
                    json!({"id":id.0,"first":MAX_PAGE_SIZE,"after":after,
                           "nestedFirst":NESTED_PAGE_SIZE,
                           "boardItems":BOARD_ITEMS_PAGE_SIZE,"duplicates":true}),
                )
                .await;
            let data = match asked {
                Ok(data) => data,
                // A string that is not a node id at all is not a failure to report: it is
                // the ordinary answer to a selector naming a project by its name.
                Err(error) if unresolvable_node(&error) => return Ok(None),
                Err(error) => return Err(error),
            };
            let Some(connection) = data
                .pointer("/node/subIssues")
                .filter(|value| !value.is_null())
            else {
                // No such node, or one with no sub-issue relationship — a board draft is
                // the one this board can really hold.
                return Ok(None);
            };
            for node in connection
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub subIssues.nodes is not an array".into(),
                })?
            {
                if let Some(resolved) = self.resolve_issue(node)? {
                    children.push(resolved);
                }
            }
            let info = connection
                .get("pageInfo")
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub subIssues connection has no pageInfo".into(),
                })?;
            let next = required_bool(info, "hasNextPage")?
                .then(|| required_str(info, "endCursor"))
                .transpose()?;
            match next {
                Some(next) => {
                    validate_cursor_progress(after.as_deref(), next)?;
                    after = Some(next.to_owned());
                }
                None => return Ok(Some(children)),
            }
        }
    }

    /// Which issue of this board a project *name* is, or `None` when none is.
    ///
    /// One bounded query which filters on that name at the server, rather than a walk of
    /// every issue the board holds. The name is compared again here: the qualifier narrows
    /// what GitHub sends, and this source decides what it names.
    async fn project_by_name(&self, name: &str) -> Result<Option<NativeId>, SourceError> {
        let search = self.board_search(Some(&title_qualifier(name)));
        let (candidates, _) = self.search_page(&search, MAX_PAGE_SIZE, None).await?;
        Ok(candidates
            .into_iter()
            .find(|item| {
                item.kind == BoardKind::Work(ItemKind::Project)
                    && item.title.eq_ignore_ascii_case(name)
            })
            .map(|item| item.id))
    }

    /// Everything filed under one project of this board: the sub-issues of the issue that
    /// project is.
    ///
    /// Tasks *and* documents, because a document filed under a project is a sub-issue of it
    /// too — the caller keeps the kind it asked for. Nothing about this grows as the board
    /// gains projects, or as another project gains tasks.
    ///
    /// A qualified id names the issue and is asked for its sub-issues directly: one
    /// request, no search of any kind. Only a selector GitHub cannot resolve that way is
    /// read as a project *name*, which costs the one bounded search
    /// [`Self::project_by_name`] makes.
    async fn project_children(&self, selector: &NativeId) -> Result<Vec<Resolved>, SourceError> {
        let (project, children) = match self.sub_issues(selector).await? {
            Some(children) => (selector.clone(), children),
            None => match self.project_by_name(&selector.0).await? {
                Some(project) => {
                    let children = self.sub_issues(&project).await?.unwrap_or_default();
                    (project, children)
                }
                None => return Ok(Vec::new()),
            },
        };
        self.completed_with_written(children, |own| own.parent.as_ref() == Some(&project))
    }

    /// Every item on the board, with the one board identity they all share.
    ///
    /// See [`Self::board_cache`]. The completion from `created` happens on every call
    /// rather than once, which is what the cache could otherwise have broken.
    async fn board(&self) -> Result<Board, SourceError> {
        let cached = self.board_cache()?.clone();
        let mut board = match cached {
            Some(board) => board,
            None => {
                let read = self.read_board().await?;
                *self.board_cache()? = Some(read.clone());
                read
            }
        };
        for own in self.created()?.iter() {
            if !board.items.iter().any(|item| item.id == own.id) {
                board.items.push(own.clone());
            }
        }
        Ok(board)
    }

    /// This process's own view of the board, or the refusal a poisoned lock is.
    fn board_cache(&self) -> Result<std::sync::MutexGuard<'_, Option<Board>>, SourceError> {
        self.board_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's view of the board was left inconsistent by an earlier \
                      failure; next: run the command again"
                    .into(),
            })
    }

    /// Bring this process's own view of the board up to an item it has just written.
    ///
    /// A created item goes to `created`, which is what completes a board read GitHub's own
    /// eventual consistency has left behind. An item that was already there is replaced
    /// where it sits, so a second write of it in the same command reads its real parent
    /// rather than the one it had before the first write.
    ///
    /// "Where it sits" is two places, and missing the first leaves a stale record that
    /// wins: an item this same run created is held in `created` and not in the cached
    /// board, and `board` completes the cached board *from* `created`, so replacing only
    /// the cached copy of such an item replaces nothing and the read still reports the
    /// title it was created with.
    fn remember_written(&self, item: Resolved, created: bool) -> Result<(), SourceError> {
        if created {
            self.created()?.push(item);
            return Ok(());
        }
        {
            let mut own = self.created()?;
            if let Some(held) = own.iter_mut().find(|held| held.id == item.id) {
                *held = item;
                return Ok(());
            }
        }
        if let Some(board) = self.board_cache()?.as_mut()
            && let Some(held) = board.items.iter_mut().find(|held| held.id == item.id)
        {
            *held = item;
        }
        Ok(())
    }

    /// Forget one item this process has just deleted, from both halves of its own view.
    fn forget(&self, id: &NativeId) -> Result<(), SourceError> {
        self.created()?.retain(|own| own.id != *id);
        if let Some(board) = self.board_cache()?.as_mut() {
            board.items.retain(|item| item.id != *id);
        }
        Ok(())
    }

    /// Every page of the board, read from GitHub.
    async fn read_board(&self) -> Result<Board, SourceError> {
        let mut after: Option<String> = None;
        let mut items = Vec::new();
        let mut board;
        loop {
            let page = self.board_page(after.as_deref(), MAX_PAGE_SIZE).await?;
            for item in page
                .pointer("/items/nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items.nodes is not an array".into(),
                })?
            {
                if let Some(resolved) = self.resolve(item)? {
                    items.push(resolved);
                }
            }
            let info = page
                .pointer("/items/pageInfo")
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items have no pageInfo".into(),
                })?;
            let has_next = required_bool(info, "hasNextPage")?;
            let next = has_next
                .then(|| required_str(info, "endCursor"))
                .transpose()?;
            board = page.clone();
            match next {
                Some(next) => {
                    validate_cursor_progress(after.as_deref(), next)?;
                    after = Some(next.to_owned());
                }
                None => break,
            }
        }
        Ok(Board {
            id: required_str(&board, "id")?.to_owned(),
            fields: board.get("fields").cloned().unwrap_or(Value::Null),
            items,
        })
    }

    /// The items this source has created, for completing a board read that is behind.
    fn created(&self) -> Result<std::sync::MutexGuard<'_, Vec<Resolved>>, SourceError> {
        self.created.lock().map_err(|_| SourceError::Unavailable {
            message: "this source's record of what it created in this run was left \
                      inconsistent by an earlier failure; next: run the command again"
                .into(),
        })
    }

    /// One board item as this source reports it, or `None` for content it ignores.
    ///
    /// A pull request is neither a project nor a task — it is somebody's change, not a
    /// unit of plan — and an item whose content the token cannot see has nothing to
    /// report at all.
    fn resolve(&self, item: &Value) -> Result<Option<Resolved>, SourceError> {
        let content = item.get("content").ok_or_else(|| SourceError::Malformed {
            message: "GitHub project item is missing content".into(),
        })?;
        if content.is_null() {
            return Ok(None);
        }
        let content_kind = match required_str(content, "__typename")? {
            "Issue" => ContentKind::Issue,
            "DraftIssue" => ContentKind::DraftIssue,
            _ => return Ok(None),
        };
        let field_values = item
            .get("fieldValues")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item is missing fieldValues".into(),
            })?;
        complete_connection(field_values, "project item field values", NESTED_PAGE_SIZE)?;
        let nodes = field_values
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item fieldValues.nodes is not an array".into(),
            })?;
        if let Some(labels) = content.get("labels") {
            complete_connection(labels, "content labels", NESTED_PAGE_SIZE)?;
        }
        for field_value in nodes {
            if let Some(labels) = field_value.get("labels") {
                complete_connection(labels, "project item field labels", NESTED_PAGE_SIZE)?;
            }
        }
        let (body, slot) = metadata_body(optional_str(content, "body")?.map(str::to_owned))?;
        let parent = optional_str(content.get("parent").unwrap_or(&Value::Null), "id")?
            .map(|id| NativeId(id.to_owned()));
        // A draft has no sub-issues to summarise, and GitHub's schema gives it no field
        // to read one from; it is a task, and never a project.
        let sub_issues = match content_kind {
            ContentKind::Issue => sub_issue_total(content)?,
            ContentKind::DraftIssue => 0,
        };
        let content_id = required_str(content, "id")?;
        let marked = ItemKind::from_metadata(&slot).map_err(|message| SourceError::Malformed {
            message: format!("GitHub issue {content_id}: {message}"),
        })?;
        let raw_title = required_str(content, "title")?;
        // The design prefix is read *first*, before either of the two rules that separate
        // a project from a task. A document is not work whatever sub-issues it has and
        // whatever marker it carries, and reading the prefix later would make a design
        // issue with none of either an empty project.
        let kind = if raw_title.starts_with(DESIGN_TITLE_PREFIX) {
            BoardKind::Document
        } else if parent.is_some() {
            // Being a sub-issue wins outright, and no marker overrides it: an issue filed
            // under a project is that project's task even when it has sub-issues of its
            // own.
            BoardKind::Work(ItemKind::Task)
        } else if sub_issues > 0 || marked == Some(ItemKind::Project) {
            BoardKind::Work(ItemKind::Project)
        } else {
            BoardKind::Work(ItemKind::Task)
        };
        // The title a person wrote, which for a document is the one without the prefix —
        // the same way `content` above is the body without this source's metadata slot.
        let title = match kind {
            BoardKind::Document => raw_title[DESIGN_TITLE_PREFIX.len()..].to_owned(),
            BoardKind::Work(_) => raw_title.to_owned(),
        };
        let own_repository = content
            .pointer("/repository/nameWithOwner")
            .and_then(Value::as_str)
            .map(|origin| Repository::try_from(format!("github.com/{origin}")))
            .transpose()
            .map_err(|message| SourceError::Malformed { message })?;
        let repositories = if slot.contains_key(Repository::METADATA_KEY) {
            Repository::from_metadata(&slot)
                .map_err(|message| SourceError::Malformed { message })?
        } else {
            own_repository.clone().into_iter().collect()
        };
        Ok(Some(Resolved {
            item_id: required_str(item, "id")?.to_owned(),
            id: NativeId(content_id.to_owned()),
            content_kind,
            kind,
            title,
            body: body.filter(|value| !value.is_empty()),
            status: self.status(item, content)?,
            labels: labels(content, nodes)?,
            parent,
            origin: text_field(nodes, ORIGIN_FIELD)?.filter(|value| !value.is_empty()),
            url: optional_str(content, "url")?.map(str::to_owned),
            created_at: optional_time(content, "createdAt")?,
            updated_at: optional_time(content, "updatedAt")?,
            own_repository,
            repositories,
            slot,
        }))
    }

    /// The status one board item reports.
    ///
    /// The closed state decides the category and the `Status` option decides the name, so
    /// a closed issue sitting in a "Shipped" column reports `done` named `Shipped`. A
    /// closed issue whose reason is `DUPLICATE` or `REOPENED` reports `Unknown`: a
    /// duplicate is not finished work, and calling it done is a lie the next copy would
    /// write back. `REOPENED`-while-closed is a state this source can never produce, so
    /// it is read permissively rather than refused — reads are faithful, and refusals
    /// belong on writes.
    fn status(&self, item: &Value, content: &Value) -> Result<Status, SourceError> {
        let nodes = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .expect("resolve validates fieldValues.nodes before mapping status");
        let option = nodes
            .iter()
            .find(|value| value.pointer("/field/name").and_then(Value::as_str) == Some("Status"))
            .map(|value| required_str(value, "name"))
            .transpose()?;
        let state = optional_str(content, "state")?;
        if state == Some("CLOSED") {
            let category = match optional_str(content, "stateReason")? {
                None | Some("COMPLETED") => StatusCategory::Done,
                Some("NOT_PLANNED") => StatusCategory::Cancelled,
                Some(_) => StatusCategory::Unknown,
            };
            let fallback = match category {
                StatusCategory::Done => "Done",
                StatusCategory::Cancelled => "Cancelled",
                _ => "Closed",
            };
            return Ok(Status {
                category,
                name: option.unwrap_or(fallback).to_owned(),
            });
        }
        let name = option.unwrap_or("Open").to_owned();
        Ok(Status {
            category: self
                .statuses
                .category_of(&name)
                .unwrap_or(StatusCategory::Unknown),
            name,
        })
    }

    /// The board Status option this write selects, or the refusal that says why not.
    ///
    /// For a column target the option is what the status *is*, so a board that has no such
    /// option is a refusal naming the status and the instance. For a closed target the
    /// issue's own state carries the category, and the option carries only the name a
    /// reader reports — so an option spelled the way this status is spelled is selected
    /// when the board has one, and nothing is refused when it does not.
    fn column_for(
        &self,
        board: &Board,
        status: &Status,
        target: &StatusTarget,
    ) -> Result<Option<(String, String)>, SourceError> {
        let (wanted, required) = match target {
            StatusTarget::Column(wanted) => (wanted.as_str(), true),
            StatusTarget::Closed(_) => (status.name.as_str(), false),
            StatusTarget::Disabled => return Ok(None),
        };
        let missing = |detail: &str| SourceError::Refused {
            message: format!(
                "status {} of source {} needs the board Status option {wanted:?}, and {detail};                  add that option to the board, or point status_mapping.{} of this source at one                  it has",
                category_name(status.category),
                self.name,
                category_name(status.category)
            ),
        };
        let Some(field) = Board::field(&board.fields, "Status")? else {
            return if required {
                Err(missing("this board has no Status field"))
            } else {
                Ok(None)
            };
        };
        if required_str(field, "__typename")? != "ProjectV2SingleSelectField" {
            return if required {
                Err(missing(
                    "this board's Status field is not a single-select field",
                ))
            } else {
                Ok(None)
            };
        }
        let option = field
            .get("options")
            .and_then(Value::as_array)
            .and_then(|options| {
                options.iter().find(|option| {
                    option
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                })
            });
        match option {
            None if required => Err(missing("this board does not have it")),
            None => Ok(None),
            Some(option) => Ok(Some((
                required_str(field, "id")?.to_owned(),
                required_str(option, "id")?.to_owned(),
            ))),
        }
    }

    /// This instance's target for a category, refusing one it has disabled.
    ///
    /// Nothing here mutates the board's option set to make room for a status. GitHub
    /// documents `UpdateProjectV2FieldInput.singleSelectOptions` as *"provided values
    /// overwrite existing options"*, so no addition is additive and a mistake destroys the
    /// field and every item's status.
    fn resolved_target(&self, category: StatusCategory) -> Result<StatusTarget, SourceError> {
        let target = self.statuses.target(category).clone();
        if target != StatusTarget::Disabled {
            return Ok(target);
        }
        Err(SourceError::Refused {
            message: if category == StatusCategory::Draft {
                format!(
                    "status draft is disabled for source {}: draft is incompatible with this \
                     integration because GitHub draft issues cannot have sub-issues, and this \
                     source stores a project's tasks as its issue's sub-issues",
                    self.name
                )
            } else {
                format!(
                    "status {} is disabled for source {}; set status_mapping.{} of this source \
                     to a board Status option name or to a closed state",
                    category_name(category),
                    self.name,
                    category_name(category)
                )
            },
        })
    }

    async fn set_item_field(
        &self,
        board_id: &str,
        item_id: &str,
        field_id: &str,
        value: Value,
    ) -> Result<(), SourceError> {
        let data = self
            .graphql(
                graphql::UPDATE_FIELD,
                json!({"input":{
                    "projectId":board_id,"itemId":item_id,"fieldId":field_id,"value":value
                }}),
            )
            .await?;
        let returned = data
            .pointer("/updateProjectV2ItemFieldValue/projectV2Item")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub field update returned no project item".into(),
            })?;
        if required_str(returned, "id")? != item_id {
            return Err(SourceError::Malformed {
                message: "GitHub field update returned the wrong project item".into(),
            });
        }
        Ok(())
    }

    async fn native_dependency_ids(&self, id: &NativeId) -> Result<Vec<String>, SourceError> {
        let mut after: Option<String> = None;
        let mut ids = Vec::new();
        loop {
            let data = self
                .graphql(
                    graphql::ISSUE_DEPENDENCIES,
                    json!({"id":id.0,"first":MAX_PAGE_SIZE,"after":after}),
                )
                .await?;
            let connection =
                data.pointer("/node/blockedBy")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency response has no blockedBy connection".into(),
                    })?;
            ids.extend(
                connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency response nodes is not an array".into(),
                    })?
                    .iter()
                    .map(|value| required_str(value, "id").map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            let next = next_cursor(connection)?;
            if let Some(next) = &next {
                validate_cursor_progress(after.as_deref(), &next.0)?;
            }
            after = next.map(|cursor| cursor.0);
            if after.is_none() {
                return Ok(ids);
            }
        }
    }

    async fn dependencies(
        &self,
        id: &NativeId,
        near_kind: ItemKind,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        validate_page(page)?;
        let limit = page.limit.min(MAX_PAGE_SIZE) as usize;
        let cursor = page.cursor.as_ref().map(|c| c.0.as_str());
        let recorded = recorded_offset(cursor, direction)?;
        // Asked for even in the recorded phase, whose page reads nothing from the
        // connection: `__typename` is what says whether this item has a native
        // relationship at all, and that is what decides which far ends the reserved key is
        // allowed to hold.
        let data = self
            .graphql(
                graphql::ISSUE_DEPENDENCIES,
                json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),
                       "after":if recorded.is_some() {None} else {cursor}}),
            )
            .await?;
        let node =
            data.get("node")
                .filter(|v| !v.is_null())
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "GitHub item {} was not found or does not support dependencies",
                        id.0
                    ),
                })?;
        let connection_name = match direction {
            Direction::DependsOn => "blockedBy",
            Direction::DependedOnBy => "blocking",
        };
        // A draft has neither `blockedBy` nor `blocking`, so nothing it depends on can be
        // named natively and the reserved key may hold any far end. An issue's connections
        // hold issues, and this source reads them at the near item's own level.
        let natively_names = (required_str(node, "__typename")? == "Issue").then_some(near_kind);
        if let Some(offset) = recorded {
            return Ok(recorded_page(
                self.recorded_edges(id, near_kind, direction, natively_names)
                    .await?,
                offset,
                limit,
            ));
        }
        if natively_names.is_none() {
            return Ok(recorded_page(
                self.recorded_edges(id, near_kind, direction, natively_names)
                    .await?,
                0,
                limit,
            ));
        }
        let connection = node
            .get(connection_name)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub dependency response is missing its connection".into(),
            })?;
        let nodes = connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub dependency response nodes is not an array".into(),
            })?;
        // `from` depends on `to`, always. GitHub spells the same relationship from either
        // end — `blockedBy` lists what this item waits on, `blocking` lists what waits on
        // it — so the near item is `from` in one direction and `to` in the other.
        let items = nodes
            .iter()
            .map(|value| {
                let related = NativeId(required_str(value, "id")?.into());
                let related_kind = related_kind(value)?;
                let (from, to) = match direction {
                    Direction::DependsOn => (
                        DependencyEndpoint::from_native(id.clone(), near_kind),
                        DependencyEndpoint::from_native(related, related_kind),
                    ),
                    Direction::DependedOnBy => (
                        DependencyEndpoint::from_native(related, related_kind),
                        DependencyEndpoint::from_native(id.clone(), near_kind),
                    ),
                };
                Ok(DependencyEdge {
                    from,
                    to,
                    kind: DependencyKind::Blocks,
                })
            })
            .collect::<Result<Vec<_>, SourceError>>()?;
        let mut next = next_cursor(connection)?;
        if let Some(next) = &next {
            validate_cursor_progress(cursor, &next.0)?;
        }
        if next.is_none()
            && !self
                .recorded_edges(id, near_kind, direction, natively_names)
                .await?
                .is_empty()
        {
            next = Some(Cursor(format!("{RECORDED_CURSOR}0")));
        }
        Ok(Page { items, next })
    }

    /// The edges this item records under [`DependencyEdge::RECORDED_KEY`], which is where
    /// a far end in another source has to live: no GitHub issue relationship can name one.
    ///
    /// Only forwards. The reverse of a recorded edge is derived from the far end, and this
    /// source never writes one down.
    ///
    /// The metadata lives in the item's own body slot, so reading it costs one board scan.
    /// That is why it happens once the native connection is spent rather than on every
    /// page.
    async fn recorded_edges(
        &self,
        id: &NativeId,
        near_kind: ItemKind,
        direction: Direction,
        natively_names: Option<ItemKind>,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        if direction != Direction::DependsOn {
            return Ok(Vec::new());
        }
        let Some(item) = self
            .board()
            .await?
            .items
            .into_iter()
            .find(|item| item.id == *id)
        else {
            return Ok(Vec::new());
        };
        DependencyEdge::recorded(&item.slot, id, near_kind, &self.name, natively_names)
            .map_err(|message| SourceError::Malformed { message })
    }

    /// The configured repository's node id, or the refusal naming the field it needs.
    ///
    /// Resolved once per command; see [`Self::repository_cache`].
    async fn repository_id(&self) -> Result<String, SourceError> {
        if let Some(id) = self.repository_cache()?.clone() {
            return Ok(id);
        }
        let repository = self
            .repository
            .as_ref()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "source {} has no repository configured, and a GitHub Projects board has no \
                 repository of its own to create an issue in; set repository: owner/name on \
                 this source",
                    self.name
                ),
            })?;
        let data = self
            .graphql(
                graphql::REPOSITORY,
                json!({"owner":repository.owner,"name":repository.name}),
            )
            .await?;
        let node = data
            .get("repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub repository {}/{} was not found or is not visible to the token",
                    repository.owner, repository.name
                ),
            })?;
        let id = required_str(node, "id")?.to_owned();
        *self.repository_cache()? = Some(id.clone());
        Ok(id)
    }

    /// This process's own record of the destination repository's node id.
    fn repository_cache(&self) -> Result<std::sync::MutexGuard<'_, Option<String>>, SourceError> {
        self.repository_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's record of the destination repository was left \
                          inconsistent by an earlier failure; next: run the command again"
                    .into(),
            })
    }

    /// Create or update one board item, whichever kind it is.
    async fn write_item(
        &self,
        incoming: &Incoming<'_>,
        target: Option<&NativeId>,
        depends_on: &[DependencyEdge],
    ) -> Result<NativeId, SourceError> {
        // Refused before anything is read or written: a task or a project titled the way
        // this board spells a document would land as an issue this same source reads back
        // as a document, so the field this destination cannot carry is named rather than
        // written and silently reclassified.
        if let Written::Work(kind, _) = incoming.written
            && incoming.title.starts_with(DESIGN_TITLE_PREFIX)
        {
            return Err(SourceError::Refused {
                message: format!(
                    "the title of this {} begins {DESIGN_TITLE_PREFIX:?}, which is how source {} \
                     spells a document, so it would read back as one rather than as a {}; \
                     retitle it, or copy it as a document",
                    kind.marker(),
                    self.name,
                    kind.marker()
                ),
            });
        }
        let board = self.board().await?;
        let status_target = incoming
            .written
            .status()
            .map(|status| self.resolved_target(status.category))
            .transpose()?;
        let column = match (incoming.written.status(), status_target.as_ref()) {
            (Some(status), Some(target)) => self.column_for(&board, status, target)?,
            _ => None,
        };
        let existing = target
            .map(|target| {
                board
                    .items
                    .iter()
                    .find(|item| item.id == *target)
                    .ok_or_else(|| SourceError::Refused {
                        message: format!("GitHub destination item {} was not found", target.0),
                    })
            })
            .transpose()?;
        let content_kind = existing.map_or(ContentKind::Issue, |item| item.content_kind);
        if content_kind == ContentKind::DraftIssue {
            if let (Some(StatusTarget::Closed(_)), Some(status)) =
                (status_target.as_ref(), incoming.written.status())
            {
                return Err(SourceError::Refused {
                    message: format!(
                        "status {} of source {} closes the item's issue, and GitHub draft items \
                         have no open or closed state",
                        category_name(status.category),
                        self.name
                    ),
                });
            }
            if incoming.parent.is_some() {
                return Err(SourceError::Refused {
                    message: "GitHub draft items cannot be a project's sub-issue".into(),
                });
            }
        }
        match existing {
            Some(item) if content_kind == ContentKind::Issue => {
                if item.labels != incoming.labels {
                    return Err(SourceError::Refused {
                        message: "GitHub issue labels differ from the labels being written".into(),
                    });
                }
            }
            _ => {
                if !incoming.labels.is_empty() {
                    return Err(SourceError::Refused {
                        message: "GitHub items created by this destination carry no labels".into(),
                    });
                }
            }
        }

        let own_repository = match existing {
            Some(item) => item.own_repository.clone(),
            None => self
                .repository
                .as_ref()
                .map(|repository| Repository::try_from(repository.origin()))
                .transpose()
                .map_err(|message| SourceError::Config { message })?,
        };
        let (native, fallback) = self
            .partition_edges(&board, incoming.written.kind(), content_kind, depends_on)
            .await?;
        let slot = slot_metadata(incoming, own_repository.as_ref(), &fallback);
        let body = compose_body(incoming.content, &slot)?;
        // Read before anything is created, for the reason the field below is: a value
        // this destination cannot store has to refuse, and refusing after `createIssue`
        // would leave an issue behind that nothing asked for. The engine writes a
        // qualified id here; a caller handing this key anything else is told so rather
        // than having it silently stored as no origin at all.
        // llmlint: ignore[boundary_inputs_validated, changed_behavior_has_e2e] The qualified id's syntax is the engine's and not this plugin's to police: `GlobalId` is deliberately absent from the contract crate because a plugin never sees a qualified id (AGENTS.md), no plugin crate may depend on the engine to parse one, and `docs/metadata.md` says the contents of this key are what no plugin constructs or interprets. What this boundary owns is whether the value is a string its text field can hold, and that is what it checks.
        let origin = match incoming.metadata.get(ORIGIN_KEY) {
            None => "",
            Some(Value::String(origin)) => origin.as_str(),
            Some(other) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "{ORIGIN_KEY} holds a qualified id spelled as a string, and this item's \
                         is {other}"
                    ),
                });
            }
        };
        // Resolved before anything is created: a board that cannot carry the copy origin
        // has to refuse the write, and refusing it after `createIssue` would leave an
        // issue behind that nothing asked for.
        let origin_field = match Board::field(&board.fields, ORIGIN_FIELD)? {
            Some(field) => {
                if required_str(field, "__typename")? != "ProjectV2Field" {
                    return Err(SourceError::Refused {
                        message: format!(
                            "GitHub board source-owned {ORIGIN_FIELD} field is not a text field"
                        ),
                    });
                }
                Some(required_str(field, "id")?.to_owned())
            }
            None if incoming.metadata.contains_key(ORIGIN_KEY) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "GitHub board has no source-owned {ORIGIN_FIELD} text field, and the \
                         item carries {ORIGIN_KEY}; add a text field named {ORIGIN_FIELD} to \
                         the board"
                    ),
                });
            }
            None => None,
        };

        let (content_id, item_id, url) = match existing {
            Some(item) => {
                self.update_existing(item, incoming, &body, status_target.as_ref())
                    .await?;
                (item.id.clone(), item.item_id.clone(), item.url.clone())
            }
            None => {
                self.create_and_file_issue(&board, incoming, &body, status_target.as_ref())
                    .await?
            }
        };

        // Creating an item here is several calls — `createIssue`, `addProjectV2ItemById`,
        // then each board field, the parent and the dependencies — and GitHub can fail at
        // any of them. Everything this source can refuse *before* the first of those is
        // already checked above, so what is left is GitHub itself failing part way. When it
        // does over an item this call created, the issue is taken back: a write that
        // refused must not leave an item behind that nobody asked for, and one that does
        // makes the retry create a second.
        let landed = self
            .finish_write(
                &board,
                incoming,
                &content_id,
                &item_id,
                content_kind,
                existing,
                origin_field.as_deref(),
                origin,
                column,
                &native,
            )
            .await;
        if let Err(error) = landed {
            if existing.is_none() {
                // Best effort, and the write's own failure is what the caller is told: a
                // refusal naming the tidy-up would hide why the write failed at all.
                let _ = self.delete_issue(&content_id).await;
            }
            return Err(error);
        }

        // So the rest of this command reads what it just did rather than what the board
        // said before it. See `remember_written` for which half takes it.
        let remembered = Resolved {
            item_id,
            id: content_id.clone(),
            content_kind,
            kind: incoming.written.kind(),
            title: incoming.title.to_owned(),
            // The visible half of the body this write composed, split back off it the
            // way a read splits it — so what this record reports is what a read of the
            // same issue reports, rather than the person's text with the metadata slot
            // still on the end of it.
            body: metadata_body(body.clone())?.0,
            // A document has no status of its own; what it reads back as is whatever
            // the issue's own state says, which is what a re-read reports.
            status: incoming
                .written
                .status()
                .cloned()
                .unwrap_or_else(|| Status {
                    category: StatusCategory::Unknown,
                    name: "Open".to_owned(),
                }),
            labels: incoming.labels.to_vec(),
            parent: incoming.parent.cloned(),
            origin: (!origin.is_empty()).then(|| origin.to_owned()),
            // In the update path this is the item's own url, read off `existing` where the
            // tuple above was bound, so one expression serves both halves.
            url,
            created_at: existing.and_then(|item| item.created_at),
            updated_at: existing.and_then(|item| item.updated_at),
            own_repository,
            repositories: incoming.repositories.to_vec(),
            slot,
        };
        self.remember_written(remembered, existing.is_none())?;
        Ok(content_id)
    }

    /// Everything a write does after the item exists: its board fields, its parent, and
    /// its dependencies.
    ///
    /// Split out of `write_item` so there is one place a failure past the point of no
    /// return is caught, rather than a tidy-up repeated at each `?` above.
    // llmlint: ignore[suppressions_justified] This is the tail of `write_item` lifted out
    // so there is one place a failure past the point of no return is caught, and its
    // arguments are exactly the values that tail already had in scope. Bundling them into a
    // struct would describe no concept — it would be "the arguments of this function" — and
    // would put the whole of `write_item`'s locals behind one more indirection.
    #[allow(clippy::too_many_arguments)]
    async fn finish_write(
        &self,
        board: &Board,
        incoming: &Incoming<'_>,
        content_id: &NativeId,
        item_id: &str,
        content_kind: ContentKind,
        existing: Option<&Resolved>,
        origin_field: Option<&str>,
        origin: &str,
        column: Option<(String, String)>,
        native: &[String],
    ) -> Result<(), SourceError> {
        if let Some(field_id) = origin_field {
            self.set_item_field(&board.id, item_id, field_id, json!({"text":origin}))
                .await?;
        }

        if let Some((field_id, option_id)) = column {
            self.set_item_field(
                &board.id,
                item_id,
                &field_id,
                json!({"singleSelectOptionId":option_id}),
            )
            .await?;
        }

        if content_kind == ContentKind::Issue {
            self.reparent(
                existing.and_then(|item| item.parent.clone()),
                content_id,
                incoming.parent,
            )
            .await?;
            // A document takes part in no dependency graph, so writing one neither reads
            // nor changes the issue's own `blockedBy` relationships. Reconciling them
            // against the empty list a document write carries would *delete* whatever
            // relationships a person had made on that issue, which is a write nobody
            // asked for.
            if incoming.written.kind() != BoardKind::Document {
                self.reconcile_blocked_by(content_id, native).await?;
            }
        }
        Ok(())
    }

    /// Delete one issue, which takes its board item with it.
    async fn delete_issue(&self, id: &NativeId) -> Result<(), SourceError> {
        let data = self
            .graphql(graphql::DELETE_ISSUE, json!({"input":{"issueId":id.0}}))
            .await?;
        data.pointer("/deleteIssue/repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue deletion returned no repository".into(),
            })?;
        self.forget(id)?;
        Ok(())
    }

    /// Remove one item this copy created, so a copy that could not finish leaves the board
    /// as it found it.
    ///
    /// Deleting the issue takes its board item with it, so there is no second mutation to
    /// keep in step. An id the board does not hold is not an error: the item is already
    /// gone, which is the state this asks for.
    async fn delete_item(&self, id: &NativeId) -> Result<(), SourceError> {
        let board = self.board().await?;
        let Some(item) = board.items.iter().find(|item| item.id == *id) else {
            return Ok(());
        };
        if item.content_kind == ContentKind::DraftIssue {
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub item {} is a draft, and this source removes an item by deleting \
                     its issue; next: remove it from the board by hand",
                    id.0
                ),
            });
        }
        let data = self
            .graphql(graphql::DELETE_ISSUE, json!({"input":{"issueId":id.0}}))
            .await?;
        data.pointer("/deleteIssue/repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue deletion returned no repository".into(),
            })?;
        self.forget(id)?;
        Ok(())
    }

    /// Which far ends this item's own `blockedBy` relationship holds, and which it cannot.
    async fn partition_edges(
        &self,
        board: &Board,
        near_kind: BoardKind,
        near_content: ContentKind,
        depends_on: &[DependencyEdge],
    ) -> Result<(Vec<String>, Vec<DependencyEdge>), SourceError> {
        let mut native = Vec::new();
        let mut fallback = Vec::new();
        for edge in depends_on {
            let same_source = edge
                .to
                .source()
                .is_none_or(|source| source == self.name.as_str());
            // A qualified id's source segment runs to its *first* colon — `GlobalId` and
            // `DependencyEndpoint::source` both read it that way — and a native id may hold
            // colons of its own, so the far end is everything after that one separator.
            // Splitting at the last would truncate `work:urn:task:7` to `7`.
            let far_id = if edge.to.is_qualified() {
                edge.to
                    .id()
                    .split_once(':')
                    .map_or(edge.to.id(), |(_, native)| native)
            } else {
                edge.to.id()
            };
            let far = if same_source {
                Some(
                    board
                        .items
                        .iter()
                        .find(|item| item.id.0 == far_id)
                        .ok_or_else(|| SourceError::Refused {
                            message: format!("GitHub dependency item {far_id} was not found"),
                        })?,
                )
            } else {
                None
            };
            // The caller says which kind the far end is, and this board holds the far end
            // itself, so a disagreement is settled here rather than stored: recorded, the
            // wrong kind would read back as a cross-level edge that never existed; written
            // natively, it would name a relationship of a different level than the caller
            // asked for.
            //
            // A far end this board holds as a *document* fails the same comparison and is
            // refused by the same sentence: `ItemKind` has no document variant because
            // nothing may point at one, so no caller can name it correctly and the refusal
            // is the only honest answer.
            if let Some(disagreeing) = far.filter(|far| far.kind != BoardKind::Work(edge.to.kind)) {
                return Err(SourceError::Refused {
                    message: format!(
                        "GitHub dependency item {far_id} is a {} of this board, and this item \
                         names it as a {}; record the kind it is",
                        disagreeing.kind.describes(),
                        edge.to.kind.marker()
                    ),
                });
            }
            // A draft has neither `blockedBy` nor `blocking`, so no edge of one is native
            // however the far end is spelled — and one classified native here would be
            // written nowhere at all, because a draft's native reconciliation never runs.
            let native_here = near_content == ContentKind::Issue
                && far.is_some_and(|far| {
                    far.content_kind == ContentKind::Issue
                        && BoardKind::Work(edge.to.kind) == near_kind
                });
            if native_here {
                native.push(far_id.to_owned());
            } else {
                fallback.push(edge.clone());
            }
        }
        Ok((native, fallback))
    }

    async fn update_existing(
        &self,
        item: &Resolved,
        incoming: &Incoming<'_>,
        body: &Option<String>,
        status_target: Option<&StatusTarget>,
    ) -> Result<(), SourceError> {
        let title = incoming.written_title();
        let (operation, input, pointer) = match item.content_kind {
            ContentKind::DraftIssue => (
                graphql::UPDATE_DRAFT,
                json!({"draftIssueId":item.id.0,"title":title,"body":body}),
                "/updateProjectV2DraftIssue/draftIssue",
            ),
            ContentKind::Issue => (
                graphql::UPDATE_ISSUE,
                json!({"id":item.id.0,"title":title,"body":body,
                       "stateInput":state_input(status_target)}),
                "/updateIssue/issue",
            ),
        };
        let data = self.graphql(operation, json!({"input":input})).await?;
        let returned = data
            .pointer(pointer)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub item update returned no item".into(),
            })?;
        if required_str(returned, "id")? != item.id.0 {
            return Err(SourceError::Malformed {
                message: "GitHub item update returned the wrong item".into(),
            });
        }
        Ok(())
    }

    /// Creates one issue, files it on the board, and closes it when the status says so.
    ///
    /// Three calls rather than one: `createIssue` needs a repository and answers with an
    /// issue that is on no board, `addProjectV2ItemById` is what puts it there, and a
    /// closed status is a state of the issue rather than a field of the board item.
    /// Creates the issue, files it on the board, and reports what a read of it would say:
    /// its content id, its board item id, and the web address GitHub gave it.
    ///
    /// The address comes back here because this is the only place it is known before
    /// GitHub's own board read catches up — an item this run created answers the reads
    /// that follow it out of the record below, and one remembered without its address
    /// would report no location for the rest of the run.
    async fn create_and_file_issue(
        &self,
        board: &Board,
        incoming: &Incoming<'_>,
        body: &Option<String>,
        status_target: Option<&StatusTarget>,
    ) -> Result<(NativeId, String, Option<String>), SourceError> {
        let repository_id = self.repository_id().await?;
        let data = self
            .graphql(
                graphql::CREATE_ISSUE,
                json!({"input":{
                    "repositoryId":repository_id,"title":incoming.written_title(),"body":body
                }}),
            )
            .await?;
        let created = data
            .pointer("/createIssue/issue")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue creation returned no issue".into(),
            })?;
        let content_id = NativeId(required_str(created, "id")?.to_owned());
        // Optional although GitHub's schema makes it non-null: the issue exists by now, so
        // a response without it is not worth failing a landed write over — the item simply
        // reports no location until the board read catches up, which is what it did before.
        let url = optional_str(created, "url")?.map(str::to_owned);
        // The issue exists from here on, so a failure filing it on the board takes it
        // back: an issue in the repository that is on no board is an item nobody asked for
        // and nothing here would find again.
        let added = match self
            .graphql(
                graphql::ADD_TO_BOARD,
                json!({"input":{"projectId":board.id,"contentId":content_id.0}}),
            )
            .await
        {
            Ok(added) => added,
            Err(error) => {
                let _ = self.delete_issue(&content_id).await;
                return Err(error);
            }
        };
        let item = added
            .pointer("/addProjectV2ItemById/item")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub board addition returned no project item".into(),
            })?;
        if let Some(StatusTarget::Closed(_)) = status_target {
            let closed = self
                .graphql(
                    graphql::UPDATE_ISSUE,
                    json!({"input":{"id":content_id.0,"stateInput":state_input(status_target)}}),
                )
                .await?;
            let returned =
                closed
                    .pointer("/updateIssue/issue")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub item update returned no item".into(),
                    })?;
            if required_str(returned, "id")? != content_id.0 {
                return Err(SourceError::Malformed {
                    message: "GitHub item update returned the wrong item".into(),
                });
            }
        }
        Ok((content_id, required_str(item, "id")?.to_owned(), url))
    }

    /// Move one issue under the project it now belongs to, or out of the one it left.
    async fn reparent(
        &self,
        held: Option<NativeId>,
        child: &NativeId,
        wanted: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        if held.as_ref() == wanted {
            return Ok(());
        }
        if let Some(held) = &held {
            self.sub_issue(graphql::REMOVE_SUB_ISSUE, held, child, "removeSubIssue")
                .await?;
        }
        if let Some(wanted) = wanted {
            self.sub_issue(graphql::ADD_SUB_ISSUE, wanted, child, "addSubIssue")
                .await?;
        }
        Ok(())
    }

    async fn sub_issue(
        &self,
        operation: &str,
        parent: &NativeId,
        child: &NativeId,
        root: &str,
    ) -> Result<(), SourceError> {
        let data = self
            .graphql(
                operation,
                json!({"input":{"issueId":parent.0,"subIssueId":child.0}}),
            )
            .await?;
        let issue =
            data.pointer(&format!("/{root}/issue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub sub-issue update returned no issue".into(),
                })?;
        let sub =
            data.pointer(&format!("/{root}/subIssue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub sub-issue update returned no sub-issue".into(),
                })?;
        if required_str(issue, "id")? != parent.0 || required_str(sub, "id")? != child.0 {
            return Err(SourceError::Malformed {
                message: "GitHub sub-issue update returned the wrong issues".into(),
            });
        }
        Ok(())
    }

    async fn reconcile_blocked_by(
        &self,
        content_id: &NativeId,
        native: &[String],
    ) -> Result<(), SourceError> {
        let current = self.native_dependency_ids(content_id).await?;
        for (operation, far_id) in current
            .iter()
            .filter(|id| !native.contains(id))
            .map(|id| (graphql::REMOVE_BLOCKED_BY, id))
            .chain(
                native
                    .iter()
                    .filter(|id| !current.contains(id))
                    .map(|id| (graphql::ADD_BLOCKED_BY, id)),
            )
        {
            let data = self
                .graphql(
                    operation,
                    json!({"input":{"issueId":content_id.0,"blockingIssueId":far_id}}),
                )
                .await?;
            let root = if operation == graphql::ADD_BLOCKED_BY {
                "addBlockedBy"
            } else {
                "removeBlockedBy"
            };
            let issue =
                data.pointer(&format!("/{root}/issue"))
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency update returned no issue".into(),
                    })?;
            let blocker = data
                .pointer(&format!("/{root}/blockingIssue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub dependency update returned no blocking issue".into(),
                })?;
            if required_str(issue, "id")? != content_id.0 || required_str(blocker, "id")? != far_id
            {
                return Err(SourceError::Malformed {
                    message: "GitHub dependency update returned the wrong issues".into(),
                });
            }
        }
        Ok(())
    }
}

/// What resolving one node id reached; see [`GitHubProjectsSource::reach`].
enum Reached {
    /// An issue this board holds, resolved into everything this source reports about it.
    Held(Box<Resolved>),
    /// Nothing this board holds: no such node, or a node on some other board.
    Nothing,
    /// A board draft, which exists only inside the board's own item connection.
    Draft,
}

/// What GitHub says when a string is not a node id it can resolve.
///
/// Matched because it is the ordinary answer to a project selector naming a project by its
/// *name*, and reporting that as a failure would make naming one impossible. It is read
/// off the refusal GitHub sent, never guessed from the shape of the string: this source
/// does not define the syntax of a GitHub node id and would be wrong about it.
const UNRESOLVABLE_NODE: &str = "could not resolve to a node";

/// Whether this refusal is GitHub saying the id names no node at all.
fn unresolvable_node(error: &SourceError) -> bool {
    matches!(error, SourceError::Refused { message }
        if message.to_ascii_lowercase().contains(UNRESOLVABLE_NODE))
}

/// One project name, as a search qualifier which filters on it at the server.
///
/// Quoted so the whole title is one phrase rather than a bag of words, with the two
/// characters GitHub's own quoting grammar gives a meaning inside a quoted phrase escaped
/// the way it documents. A title matched here is still compared for equality afterwards:
/// the qualifier narrows what the server sends, and this source decides what it names.
fn title_qualifier(name: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("in:title \"{escaped}\"")
}

/// The board, and every item on it this source reports.
#[derive(Clone)]
struct Board {
    id: String,
    fields: Value,
    items: Vec<Resolved>,
}

impl Board {
    fn field<'a>(fields: &'a Value, name: &str) -> Result<Option<&'a Value>, SourceError> {
        complete_connection(fields, "project fields", NESTED_PAGE_SIZE)?;
        let nodes = fields
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project fields.nodes is not an array".into(),
            })?;
        Ok(nodes
            .iter()
            .find(|field| field.get("name").and_then(Value::as_str) == Some(name)))
    }
}

/// One board item, resolved into everything this source reports about it.
#[derive(Clone)]
struct Resolved {
    item_id: String,
    id: NativeId,
    content_kind: ContentKind,
    kind: BoardKind,
    title: String,
    body: Option<String>,
    status: Status,
    labels: Vec<Label>,
    parent: Option<NativeId>,
    // llmlint: ignore[invalid_states_unrepresentable] The write side's reason, read back: this is the engine's qualified id, taken out of a board text field and handed on untouched. A newtype here would have this plugin define the syntax of an id `docs/metadata.md` says no plugin ever constructs or interprets.
    origin: Option<String>,
    url: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    own_repository: Option<Repository>,
    repositories: Vec<Repository>,
    slot: BTreeMap<String, Value>,
}

impl Resolved {
    /// The metadata a caller sees: their own keys, plus the copy origin this source keeps
    /// in a field of its own, and none of the three keys that are only an encoding.
    fn metadata(&self) -> BTreeMap<String, Value> {
        let mut metadata = self.slot.clone();
        metadata.remove(Repository::METADATA_KEY);
        metadata.remove(DependencyEdge::RECORDED_KEY);
        metadata.remove(ItemKind::METADATA_KEY);
        if let Some(origin) = &self.origin {
            metadata.insert(ORIGIN_KEY.to_owned(), Value::String(origin.clone()));
        }
        metadata
    }

    /// Where this item is, as a link a reader can open.
    ///
    /// A board is a hosted place and every issue on it has a web address, so that address
    /// is what "where is this?" means here — and [`Location::Url`] is what says which kind
    /// of place it is, so a reader knows to open it rather than to read a file out. It
    /// does not replace or derive from `url`: the field goes on reporting exactly what it
    /// reported before, and this says what that address *is*.
    ///
    /// An item GitHub gave no `url` for — a draft has none — reports no location at all
    /// rather than a third variant, which is the contract's "the source did not say". An
    /// issue this run created is not one of those: its address comes back from the
    /// creating mutation, so it is somewhere a reader can open from the moment it exists
    /// rather than from whenever the board read catches up.
    fn location(&self) -> Option<Location> {
        self.url.clone().map(Location::Url)
    }

    fn task(&self) -> Task {
        Task {
            id: self.id.clone(),
            title: self.title.clone(),
            content: self.body.clone(),
            status: self.status.clone(),
            labels: self.labels.clone(),
            project: self.parent.clone(),
            url: self.url.clone(),
            location: self.location(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
        }
    }

    fn project(&self) -> Project {
        Project {
            id: self.id.clone(),
            title: self.title.clone(),
            content: self.body.clone(),
            status: self.status.clone(),
            labels: self.labels.clone(),
            url: self.url.clone(),
            location: self.location(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
        }
    }

    /// The same issue as a document: the project it is filed under, and no status and no
    /// dependencies, because a document is not work.
    fn document(&self) -> Document {
        Document {
            id: self.id.clone(),
            title: self.title.clone(),
            content: self.body.clone(),
            project: self.parent.clone(),
            labels: self.labels.clone(),
            url: self.url.clone(),
            location: self.location(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
        }
    }
}

/// What one write is, and the status that comes with being it.
///
/// One value rather than a [`BoardKind`] beside an `Option<Status>`: a document has no
/// status and a task or a project always has one, so "a document carrying a status" and
/// "a task carrying none" are states a write cannot be in rather than states every use
/// site below has to defend against.
enum Written<'a> {
    /// A document, which is not work and so has no status at all.
    Document,
    /// A task or a project, and the status it is being written with.
    Work(ItemKind, &'a Status),
}

impl Written<'_> {
    /// Which of the board's three kinds this write is.
    const fn kind(&self) -> BoardKind {
        match self {
            Self::Document => BoardKind::Document,
            Self::Work(kind, _) => BoardKind::Work(*kind),
        }
    }

    /// The status this write carries. A document carries none, so a write of one says
    /// nothing about the issue's open or closed state and selects no board `Status`
    /// option.
    const fn status(&self) -> Option<&Status> {
        match self {
            Self::Document => None,
            Self::Work(_, status) => Some(status),
        }
    }
}

/// The item being written, in the one shape all three write methods reach.
struct Incoming<'a> {
    written: Written<'a>,
    /// The title a person wrote. A document's goes onto the issue with
    /// [`DESIGN_TITLE_PREFIX`] put back, so a round trip returns the title that went in.
    title: &'a str,
    content: Option<&'a str>,
    labels: &'a [Label],
    metadata: &'a BTreeMap<String, Value>,
    repositories: &'a [Repository],
    parent: Option<&'a NativeId>,
}

impl Incoming<'_> {
    /// The title this write puts on the issue.
    fn written_title(&self) -> String {
        match self.written {
            Written::Document => format!("{DESIGN_TITLE_PREFIX}{}", self.title),
            Written::Work(..) => self.title.to_owned(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    DraftIssue,
    Issue,
}

/// What one board issue is: a document, or the work an [`ItemKind`] names.
///
/// A type of this source's own rather than an `ItemKind` with a third variant, because
/// `ItemKind` names what a dependency endpoint points at and nothing may point at a
/// document — the contract keeps a document out of that enum deliberately. Holding the
/// board's three answers in one value is what makes every place that asks "which is this?"
/// answer all three, rather than a `document: bool` beside a `kind` that means nothing for
/// two thirds of the board.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoardKind {
    /// An issue whose title begins [`DESIGN_TITLE_PREFIX`].
    Document,
    /// Every other issue, and every draft.
    Work(ItemKind),
}

impl BoardKind {
    /// How a refusal names this kind to the person reading it.
    const fn describes(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Work(kind) => kind.marker(),
        }
    }
}

/// Whether `labels` satisfies `filter`, matching by name, case-insensitively.
///
/// This is the local Markdown source's `labels_match`, spelled the same way on purpose:
/// the shared cross-source journeys assert one answer to one question, so two sources
/// that disagree about what "carries the label bug" means fail them.
fn labels_match(labels: &[Label], filter: &LabelFilter) -> bool {
    let holds = |name: &String| {
        labels
            .iter()
            .any(|label| label.name.eq_ignore_ascii_case(name))
    };
    (filter.any_of.is_empty() || filter.any_of.iter().any(holds))
        && filter.all_of.iter().all(holds)
        && !filter.none_of.iter().any(holds)
}

/// Whether `category` is one of `statuses`. An empty list is unfiltered rather than
/// "keeps nothing", which is what lets a `Vec<StatusCategory>` spell no filter at all.
fn status_matches(category: StatusCategory, statuses: &[StatusCategory]) -> bool {
    statuses.is_empty() || statuses.contains(&category)
}

/// Whether `title`/`content` satisfies `query`, matching case-insensitively.
///
/// `content` is the item's own prose — the body with this source's trailing metadata
/// comment already taken off — so a search never matches an encoding the author of the
/// issue never wrote.
fn text_matches(title: &str, content: Option<&str>, query: &TextQuery) -> bool {
    let terms = query.terms.to_lowercase();
    let in_title = title.to_lowercase().contains(&terms);
    let in_content = content.is_some_and(|body| body.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}

/// Whether `task` satisfies `query`, with `project` deciding the project predicate.
///
/// The project predicate is passed separately because a read narrowed to one project has
/// already answered it by asking *that project* for its own items — and re-applying it
/// there would compare the caller's selector, which may be a project's **name**, against
/// the id of the project that name resolved to, and keep nothing. Every other read passes
/// `query.project` and applies it here, which is what keeps `projects` a predicate this
/// source really does apply.
fn task_matches(task: &Task, query: &TaskQuery, project: &ProjectFilter) -> bool {
    labels_match(&task.labels, &query.labels)
        && status_matches(task.status.category, &query.statuses)
        && match project {
            ProjectFilter::Any => true,
            ProjectFilter::Orphans => task.project.is_none(),
            ProjectFilter::Is(id) => task.project.as_ref() == Some(id),
        }
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&task.title, task.content.as_deref(), text))
}

fn project_matches(project: &Project, query: &ProjectQuery) -> bool {
    labels_match(&project.labels, &query.labels)
        && status_matches(project.status.category, &query.statuses)
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&project.title, project.content.as_deref(), text))
}

/// The same three predicates a task query carries, minus the status filter.
///
/// A document is not work, so it has no status for one to compare against and the query
/// type carries none. The project predicate is the same one — a design issue filed under a
/// project issue is in that project, and one filed under nothing is in none — so it is
/// spelled the same way here rather than answered differently.
fn document_matches(document: &Document, query: &DocumentQuery, project: &ProjectFilter) -> bool {
    labels_match(&document.labels, &query.labels)
        && match project {
            ProjectFilter::Any => true,
            ProjectFilter::Orphans => document.project.is_none(),
            ProjectFilter::Is(id) => document.project.as_ref() == Some(id),
        }
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&document.title, document.content.as_deref(), text))
}

#[async_trait::async_trait]
impl TaskSource for GitHubProjectsSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
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
            max_page_size: MAX_PAGE_SIZE,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        let board = self.board_page(None, 1).await?;
        Ok(Health {
            reachable: true,
            detail: Some(format!(
                "reading GitHub project {}/{} ({})",
                self.owner,
                self.project_number,
                required_str(&board, "title")?
            )),
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self
            .item_by_id(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
            .map(|item| item.task()))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self
            .item_by_id(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Project))
            .map(|item| item.project()))
    }
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        validate_page(page)?;
        // A read narrowed to one project asks that project for its own tasks, so nothing
        // about it costs what the rest of the board holds. Every other task read is a
        // question about the whole board and is answered by reading it.
        let (held, membership) = match &query.project {
            ProjectFilter::Is(project) => (
                self.project_children(project).await?,
                // Answered by where these items came from; see `task_matches`.
                &ProjectFilter::Any,
            ),
            ProjectFilter::Any | ProjectFilter::Orphans => {
                (self.board().await?.items, &query.project)
            }
        };
        // Filtered before paged: a page of a filtered result is a page of the survivors,
        // never the survivors of a page.
        let tasks = held
            .iter()
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
            .map(Resolved::task)
            .filter(|task| task_matches(task, query, membership))
            .collect();
        Ok(offset_page(
            tasks,
            numeric_cursor(page.cursor.as_ref())?,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        validate_page(page)?;
        // The projects a board holds are found by an issue search scoped to that board,
        // never by walking the board's own item connection: what tells a project from a
        // task is the `parent` each issue carries, which costs nothing to read.
        let projects = self
            .board_issues()
            .await?
            .iter()
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Project))
            .map(Resolved::project)
            .filter(|project| project_matches(project, query))
            .collect();
        Ok(offset_page(
            projects,
            numeric_cursor(page.cursor.as_ref())?,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        Ok(self
            .item_by_id(id)
            .await?
            .filter(|item| item.kind == BoardKind::Document)
            .map(|item| item.document()))
    }
    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        validate_page(page)?;
        // Narrowed to one project, this is the same sub-issue read a task list scoped to
        // that project makes — a document filed under a project is a sub-issue of it too,
        // and which of them come back is the kind this caller asked for.
        let (held, membership) = match &query.project {
            ProjectFilter::Is(project) => (
                self.project_children(project).await?,
                // Answered by where these items came from; see `task_matches`.
                &ProjectFilter::Any,
            ),
            ProjectFilter::Any | ProjectFilter::Orphans => {
                (self.board().await?.items, &query.project)
            }
        };
        // Filtered before paged, exactly as a task read is: a page of a filtered result is
        // a page of the survivors, never the survivors of a page.
        let documents = held
            .iter()
            .filter(|item| item.kind == BoardKind::Document)
            .map(Resolved::document)
            .filter(|document| document_matches(document, query, membership))
            .collect();
        Ok(offset_page(
            documents,
            numeric_cursor(page.cursor.as_ref())?,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        validate_page(page)?;
        let offset = numeric_cursor(page.cursor.as_ref())?;
        let mut labels = self
            .board()
            .await?
            .items
            .into_iter()
            .flat_map(|item| item.labels)
            .fold(Vec::new(), |mut all, label| {
                if !all.iter().any(|x: &Label| x.id == label.id) {
                    all.push(label);
                }
                all
            });
        labels.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.0.cmp(&b.id.0)));
        Ok(offset_page(
            labels,
            offset,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(id, ItemKind::Task, direction, page).await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(id, ItemKind::Project, direction, page)
            .await
    }

    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.write_item(
            &Incoming {
                written: Written::Work(ItemKind::Task, &write.item.status),
                title: &write.item.title,
                content: write.item.content.as_deref(),
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: write.item.project.as_ref(),
            },
            write.target.as_ref(),
            &write.depends_on,
        )
        .await
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.write_item(
            &Incoming {
                written: Written::Work(ItemKind::Project, &write.item.status),
                title: &write.item.title,
                content: write.item.content.as_deref(),
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: None,
            },
            write.target.as_ref(),
            &write.depends_on,
        )
        .await
    }

    /// Create or update one document, which is one issue titled the way this board spells
    /// a document.
    ///
    /// Everything else is exactly a task write: caller metadata goes to the same canonical
    /// JSON slot at the end of the body and comes back with its JSON types intact, a key
    /// or a field this board cannot carry is refused by name rather than dropped, a target
    /// naming an issue this board does not hold is refused rather than created, and an
    /// issue this call created is taken back when the rest of the write fails.
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        // A document takes part in no dependency graph, so there is no far end to write
        // natively and none to record: a caller naming one is told so rather than having it
        // stored under the reserved key, where a later read would report an edge the
        // contract says cannot exist.
        if !write.depends_on.is_empty() {
            return Err(SourceError::Refused {
                message: format!(
                    "this write names {} dependencies for a document, and a document takes \
                     part in no dependency graph; next: put the dependency on the task or \
                     project the document is about",
                    write.depends_on.len()
                ),
            });
        }
        self.write_item(
            &Incoming {
                written: Written::Document,
                title: &write.item.title,
                content: write.item.content.as_deref(),
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: write.item.project.as_ref(),
            },
            write.target.as_ref(),
            &[],
        )
        .await
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_item(id).await
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_item(id).await
    }

    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_item(id).await
    }
}

/// Where the recorded tail of a dependency walk resumes; see
/// [`GitHubProjectsSource::recorded_edges`].
const RECORDED_CURSOR: &str = "onetaskgraph.depends_on:";

/// The board text field this source keeps a copy's origin in.
///
/// Named after the key it holds, and held to that name by the guard below rather than by
/// a reader noticing.
const ORIGIN_FIELD: &str = "onetaskgraph.origin";

/// The metadata key that field holds.
///
/// The engine owns this key and spells it once as `GlobalId::ORIGIN_KEY`; a plugin never
/// constructs or interprets the qualified id it carries. This source names it only to
/// route it — a short, typed value belongs in a typed field rather than in the body slot
/// a caller's own prose shares.
///
/// Restated rather than imported, because no plugin crate may depend on the engine. What
/// keeps the two spellings one contract is `scripts/check-origin-key-spelling.sh`, a
/// target in `check`: it reads the engine's own literal and fails naming the file and the
/// line when a plugin's parts from it either way. Drift here has one symptom — a copy
/// that creates a second item every run instead of finding the one it wrote — and that is
/// too late to learn it.
const ORIGIN_KEY: &str = "onetaskgraph.origin";

/// Where a recorded tail resumes, refusing a cursor no walk in `direction` reported.
///
/// The reserved key holds forward edges and nothing else — the reverse of a recorded edge
/// is derived from the far end, never written down on the near item — so only a forward
/// walk ever reports one of these cursors. A reverse read carrying one is resuming a walk
/// it did not come from, and it is told so rather than answered with an empty page that
/// reads as a walk which ended.
fn recorded_offset(
    cursor: Option<&str>,
    direction: Direction,
) -> Result<Option<usize>, SourceError> {
    cursor
        .and_then(|cursor| cursor.strip_prefix(RECORDED_CURSOR))
        .map(|offset| {
            if direction != Direction::DependsOn {
                return Err(SourceError::Config {
                    message: format!(
                        "{RECORDED_CURSOR}{offset} resumes recorded forward edges, which a \
                         reverse dependency read never issues; resume it in the direction \
                         that reported it"
                    ),
                });
            }
            offset.parse().map_err(|_| SourceError::Config {
                message: format!("{RECORDED_CURSOR}{offset} is not a recorded-edge cursor"),
            })
        })
        .transpose()
}

fn recorded_page(edges: Vec<DependencyEdge>, offset: usize, limit: usize) -> Page<DependencyEdge> {
    let mut page = offset_page(edges, offset, limit.max(1));
    page.next = page
        .next
        .map(|cursor| Cursor(format!("{RECORDED_CURSOR}{}", cursor.0)));
    page
}

/// The kind of one issue reached through a dependency connection.
///
/// The same questions the board scan asks, over the fields the dependency document
/// selects, and in the same order: the design prefix first, then a sub-issue is a task,
/// then anything with sub-issues or the marker is a project.
///
/// # Errors
///
/// A far end this board holds as a document is refused rather than reported. The two
/// answers that are not refusals would both be wrong: reporting it as a task names an id
/// no task read of this source can find, and reporting it as a project names one no
/// project read can. There is no third value to return — `ItemKind` has no document
/// variant, because nothing may point at a document — so the relationship itself is what
/// the person is told about.
fn related_kind(value: &Value) -> Result<ItemKind, SourceError> {
    let id = required_str(value, "id")?;
    if required_str(value, "title")?.starts_with(DESIGN_TITLE_PREFIX) {
        return Err(SourceError::Refused {
            message: format!(
                "GitHub issue {id} is a document of this board — its title begins \
                 {DESIGN_TITLE_PREFIX:?} — and nothing may depend on a document or be depended \
                 on by one; next: remove that issue's blocking relationship on this board"
            ),
        });
    }
    let parent = optional_str(value.get("parent").unwrap_or(&Value::Null), "id")?;
    if parent.is_some() {
        return Ok(ItemKind::Task);
    }
    let (_, slot) = metadata_body(optional_str(value, "body")?.map(str::to_owned))?;
    let marked = ItemKind::from_metadata(&slot).map_err(|message| SourceError::Malformed {
        message: format!("GitHub issue {id}: {message}"),
    })?;
    let sub_issues = sub_issue_total(value)?;
    Ok(if sub_issues > 0 || marked == Some(ItemKind::Project) {
        ItemKind::Project
    } else {
        ItemKind::Task
    })
}

/// The `IssueStateUpdateInput` one status target asks for.
///
/// `stateInput` and `state` are mutually exclusive on `UpdateIssueInput`, and only this
/// one is ever sent. A non-terminal status always asks for `OPEN`, which is what reopens
/// a currently-closed issue: without that the item would read back `Unknown` and a copy
/// would report a change forever. A document has no status at all, and asks for neither.
fn state_input(target: Option<&StatusTarget>) -> Value {
    match target {
        Some(StatusTarget::Closed(reason)) => {
            json!({"value":"CLOSED","stateReason":reason.reason()})
        }
        Some(StatusTarget::Column(_) | StatusTarget::Disabled) => json!({"value":"OPEN"}),
        // A document has no status, so a write of one says nothing about the issue's open
        // or closed state rather than forcing it open: `stateInput` is what carries that
        // instruction, and an explicit null asks for no change to it.
        None => Value::Null,
    }
}

/// The metadata one write stores in the item's body slot.
///
/// The typed fields travel as themselves, so the three reserved keys are rebuilt here
/// rather than carried: the kind marker so an empty project stays readable, the
/// repository list only when it is not exactly the issue's own repository, and the far
/// ends no relationship here can name.
fn slot_metadata(
    incoming: &Incoming<'_>,
    own_repository: Option<&Repository>,
    fallback: &[DependencyEdge],
) -> BTreeMap<String, Value> {
    let mut metadata = incoming.metadata.clone();
    metadata.remove(ORIGIN_KEY);
    match incoming.written.kind() {
        BoardKind::Work(kind) => metadata.insert(
            ItemKind::METADATA_KEY.to_owned(),
            Value::String(kind.marker().to_owned()),
        ),
        // A document is told by its title, so it carries no kind marker: that key names
        // what a dependency endpoint points at, and nothing may point at a document.
        BoardKind::Document => metadata.remove(ItemKind::METADATA_KEY),
    };
    let derivable = own_repository
        .map(|own| incoming.repositories == [own.clone()])
        .unwrap_or(incoming.repositories.is_empty());
    if derivable {
        metadata.remove(Repository::METADATA_KEY);
    } else {
        metadata.insert(
            Repository::METADATA_KEY.to_owned(),
            Value::Array(
                incoming
                    .repositories
                    .iter()
                    .map(|repository| Value::String(repository.as_str().to_owned()))
                    .collect(),
            ),
        );
    }
    if fallback.is_empty() {
        metadata.remove(DependencyEdge::RECORDED_KEY);
    } else {
        metadata.insert(
            DependencyEdge::RECORDED_KEY.to_owned(),
            Value::Array(
                fallback
                    .iter()
                    .map(|edge| json!({"id":edge.to.id(),"kind":edge.to.kind}))
                    .collect(),
            ),
        );
    }
    metadata
}

fn labels(content: &Value, field_values: &[Value]) -> Result<Vec<Label>, SourceError> {
    let direct = optional_nodes(content.get("labels"), "content labels")?;
    let field = field_values
        .iter()
        .find_map(|value| value.get("labels"))
        .map(|labels| optional_nodes(Some(labels), "field labels"))
        .transpose()?
        .flatten();
    let labels = direct
        .into_iter()
        .flatten()
        .chain(field.into_iter().flatten())
        .map(|v| {
            Ok(Label {
                id: NativeId(required_str(v, "id")?.to_owned()),
                name: required_str(v, "name")?.to_owned(),
                color: optional_str(v, "color")?.map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, SourceError>>()?
        .into_iter()
        .fold(Vec::new(), |mut labels, label| {
            if !labels.iter().any(|x: &Label| x.id == label.id) {
                labels.push(label);
            }
            labels
        });
    Ok(labels)
}

fn text_field(field_values: &[Value], name: &str) -> Result<Option<String>, SourceError> {
    let Some(node) = field_values
        .iter()
        .find(|node| node.pointer("/field/name").and_then(Value::as_str) == Some(name))
    else {
        return Ok(None);
    };
    Ok(optional_str(node, "text")?.map(str::to_owned))
}

fn valid_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && !owner.contains("--")
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// GitHub's repository-name grammar: 1-100 ASCII letters, digits, `-`, `_` or `.`, and
/// neither of the two names a path segment already means.
fn valid_github_repository_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// How many sub-issues one issue has.
///
/// `Issue.subIssuesSummary` is `SubIssuesSummary!` and its `total` is `Int!`, so an
/// absent or non-integer one is a response this source cannot read — and reading it as
/// zero would classify a project as a task, which is exactly the mistake the marker
/// exists to keep from happening quietly.
fn sub_issue_total(issue: &Value) -> Result<u64, SourceError> {
    let summary = issue
        .get("subIssuesSummary")
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue is missing subIssuesSummary".into(),
        })?;
    summary
        .get("total")
        .and_then(Value::as_u64)
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue subIssuesSummary.total is not an unsigned integer".into(),
        })
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing string field {field}"),
        })
}

/// The slot's delimiters, which `docs/metadata.md` settles once for every source that
/// needs one — Linear spells them too, in its own description field.
///
/// Restated rather than shared, because a plugin crate depends on the contract crate and
/// nothing else of this workspace. `scripts/check-metadata-slot-encoding.sh`, a target in
/// `check`, is what keeps the two one encoding: drift is otherwise quiet, since each
/// source round-trips its own writes perfectly well under its own spelling.
const METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const METADATA_CLOSE: &str = "\n-->";

/// The visible body and the metadata slot at the end of it.
///
/// The encoding is the one `docs/metadata.md` settles for Linear, which is where its
/// reasons are. Only a comment at the very end is a slot; one in the middle is a person's
/// own content and is left alone.
fn metadata_body(
    body: Option<String>,
) -> Result<(Option<String>, BTreeMap<String, Value>), SourceError> {
    let Some(body) = body else {
        return Ok((None, BTreeMap::new()));
    };
    let Some(start) = body.rfind(METADATA_OPEN) else {
        return Ok((Some(body), BTreeMap::new()));
    };
    let encoded_start = start + METADATA_OPEN.len();
    let Some(relative_end) = body[encoded_start..].find(METADATA_CLOSE) else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in GitHub issue body".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    if !body[encoded_end + METADATA_CLOSE.len()..].trim().is_empty() {
        return Ok((Some(body), BTreeMap::new()));
    }
    let metadata = serde_json::from_str(&body[encoded_start..encoded_end]).map_err(|error| {
        SourceError::Malformed {
            message: format!(
                "invalid canonical JSON in GitHub issue onetaskgraph metadata slot: {error}"
            ),
        }
    })?;
    let visible = body[..start].trim_end();
    Ok(((!visible.is_empty()).then(|| visible.to_owned()), metadata))
}

fn compose_body(
    content: Option<&str>,
    metadata: &BTreeMap<String, Value>,
) -> Result<Option<String>, SourceError> {
    let visible = content.unwrap_or_default();
    if metadata.is_empty() {
        return Ok((!visible.is_empty()).then(|| visible.to_owned()));
    }
    let encoded = serde_json::to_string(metadata).map_err(|error| SourceError::Malformed {
        message: error.to_string(),
    })?;
    Ok(Some(if visible.is_empty() {
        format!("{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
    } else {
        format!("{visible}\n\n{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
    }))
}

fn required_bool(value: &Value, field: &str) -> Result<bool, SourceError> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing boolean field {field}"),
        })
}
fn optional_str<'a>(value: &'a Value, field: &str) -> Result<Option<&'a str>, SourceError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub response field {field} is not a string or null"),
            }),
    }
}
fn optional_nodes<'a>(
    connection: Option<&'a Value>,
    name: &str,
) -> Result<Option<&'a Vec<Value>>, SourceError> {
    match connection {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .get("nodes")
            .and_then(Value::as_array)
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub {name}.nodes is not an array"),
            }),
    }
}
fn complete_connection(connection: &Value, name: &str, size: u32) -> Result<(), SourceError> {
    let page_info = connection
        .get("pageInfo")
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub {name} has no pageInfo"),
        })?;
    if required_bool(page_info, "hasNextPage")? {
        return Err(SourceError::Malformed {
            message: format!(
                "GitHub {name} exceeds the supported nested connection size of {size}"
            ),
        });
    }
    Ok(())
}
fn optional_time(value: &Value, field: &str) -> Result<Option<DateTime<Utc>>, SourceError> {
    optional_str(value, field)?
        .map(|timestamp| {
            timestamp.parse().map_err(|error| SourceError::Malformed {
                message: format!("GitHub response field {field} is not a timestamp: {error}"),
            })
        })
        .transpose()
}
fn validate_page(page: &PageRequest) -> Result<(), SourceError> {
    if page.limit == 0 {
        Err(SourceError::Config {
            message: "page limit must be at least 1".into(),
        })
    } else {
        Ok(())
    }
}
fn next_cursor(connection: &Value) -> Result<Option<Cursor>, SourceError> {
    let page = connection
        .get("pageInfo")
        .filter(|value| value.is_object())
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub connection is missing pageInfo".into(),
        })?;
    if required_bool(page, "hasNextPage")? {
        let cursor = required_str(page, "endCursor")?;
        validate_cursor_progress(None, cursor)?;
        Ok(Some(Cursor(cursor.into())))
    } else {
        Ok(None)
    }
}
fn validate_cursor_progress(previous: Option<&str>, next: &str) -> Result<(), SourceError> {
    if next.is_empty() || previous == Some(next) {
        Err(SourceError::Malformed {
            message: "GitHub pagination cursor is empty or did not advance".into(),
        })
    } else {
        Ok(())
    }
}
fn numeric_cursor(cursor: Option<&Cursor>) -> Result<usize, SourceError> {
    cursor.map_or(Ok(0), |c| {
        c.0.parse().map_err(|_| SourceError::Config {
            message: "page cursor is invalid".into(),
        })
    })
}
fn offset_page<T>(mut items: Vec<T>, offset: usize, limit: usize) -> Page<T> {
    if offset > items.len() {
        return Page::last(vec![]);
    }
    let tail = items.split_off(offset);
    let mut selected = tail;
    let next = (selected.len() > limit).then(|| Cursor((offset + limit).to_string()));
    selected.truncate(limit);
    Page {
        items: selected,
        next,
    }
}
