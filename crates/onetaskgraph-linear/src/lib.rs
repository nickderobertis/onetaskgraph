//! A read/write source over Linear's published GraphQL API.
//!
//! Linear `Issue` maps to [`Task`], `Project` to [`Project`], `Document` to [`Document`],
//! `IssueLabel` and `ProjectLabel` to [`Label`], and an issue's `WorkflowState.name` and a
//! project's `ProjectStatus.name` are preserved as the status's name while the source's
//! `status_mapping` — the one grammar every source that names its statuses is configured with,
//! [`StatusMapping`] — decides its category. Issue `relations`/`inverseRelations` and
//! project relations provide native dependency traversal in both directions.
//!
//! Label, workflow-state, project, and orphan filters are sent in the
//! `issues(filter:)`/`projects(filter:)` variables. Pagination uses Relay `first` and
//! `after`.
//!
//! Every issue, project and document reports its own Linear web address as its
//! [`Location`], as a link rather than a path — the counterpart of a folder of Markdown
//! reporting the path of the file behind an item. It does not replace the `url` field
//! those types already carry; it is the same address said in the shape a reader can act on.
//!
//! # What this source declares, field by field
//!
//! One verdict per field of [`Capabilities`]. A field is *supported and proven* when this
//! source applies it and a shared journey drives it against the real binary; the shared
//! table is `crates/onetaskgraph/tests/e2e/fixtures.rs`, the journeys are beside it, and
//! `every_row_declares_exactly_what_its_plugin_reports` is what keeps this list and
//! [`capabilities`](TaskSource::capabilities) from parting.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven.** `issues(filter:{project:{id:{eq:…}}})`. |
//! | `documents` | **Supported and proven.** Linear's own first-class `Document`, read through `documents(first:,after:,filter:)` and `document(id:)`, written through `documentCreate`/`documentUpdate` and taken back by `documentDelete`. See the ruling below on what a Linear document cannot hold. |
//! | `comments` | **Supported and proven,** as the issue's own comments: read oldest first through `issue(id:){comments(last:,before:)}`, added with `commentCreate`, edited with `commentUpdate` and removed with `commentDelete` — each of the last two only once `comment(id:)` has placed the comment on that very issue. See the ruling below on the order and on the author. |
//! | `priority` | **Supported,** as Linear's own `Issue.priority`: read on every issue, written by `issueCreate`/`issueUpdate` through `IssueCreateInput.priority`/`IssueUpdateInput.priority`, and set on its own by an `issueUpdate` carrying nothing else. See the ruling below on the scale. |
//! | `filter_by_priority` | **Supported and proven.** `issues(filter:{priority:{in:[…]}})` over Linear's own `0`–`4` scale, confirmed against each issue read. |
//! | `filter_by_comment_activity` | **Supported and proven.** `comments:{some:{or:[{createdAt:{gte:…}},{updatedAt:{gte:…}}]}}` — the issues with a comment created or last edited at or after the instant, over the same two fields a comment read reports. |
//! | `filter_by_metadata` | **Supported and proven.** `description:{contains:"\"<value>\""}` for each match — the value as every JSON encoder writes it, which a slot holding it contains however it spaces or spells its keys — and every candidate confirmed over the parsed slot, so prose carrying the phrase and a slot holding another value are both kept out. A value with a character an encoder may escape is not sent, and the confirmation decides alone. |
//! | `filter_by_origin` | **Supported and proven,** on the same terms, for the slot's `onetaskgraph.origin`. |
//! | `orphan_tasks` | **Supported and proven.** `issues(filter:{project:{null:true}})`. |
//! | `filter_by_label` | **Supported and proven.** `labels:{some:{name:{eqIgnoreCase:…}}}` for what an item must carry — one per label, gathered under `or:` where any one of them will do — and `labels:{every:{name:{neqIgnoreCase:…}}}` for what it must not. Linear's `StringComparator` has no case-insensitive list operator; see the note beside `filter`. |
//! | `filter_by_status` | **Supported and proven,** and spelled twice. An issue narrows by its workflow state's name, `state:{name:{eqIgnoreCase:…}}`; a project by its project status's name, `status:{name:{eqIgnoreCase:…}}` — a different member of a different filter over a different vocabulary — each by the names its kind's half of `status_mapping` gives, and `unknown` by every name that half does not give. See the ruling below. |
//! | `search_title` | **Supported and proven.** A task query's text is `title:{containsIgnoreCase:…}`, every candidate confirmed by the contract's case-insensitive substring rule; a project or document query's text is applied by that same rule over the page Linear answered. |
//! | `search_content` | **Supported and proven,** on the same terms, `description:{containsIgnoreCase:…}`, confirmed over the visible content — the trailing metadata slot Linear's comparator also reads is not part of what the rule confirms. A `title-or-content` search sends the two under one `or`. |
//! | `task_dependencies` | **Supported and proven,** in both directions: `relations` and `inverseRelations`. |
//! | `project_dependencies` | **Supported and proven,** in both directions, by the project relations of the same shape. Linear types every one of them `dependency`; see the ruling below on the edge that has no spelling here. |
//! | `max_page_size` | **Supported and proven.** 100; every read pages with Relay `first`/`after`. Linear's connection maximum is 250 and its complexity budget is the tighter bound — see [`MAX_PAGE_SIZE`]. |
//!
//! ## Ruling: the follow-up searches are native, and what each rests on
//!
//! Six predicates a follow-up search sends — metadata, origin, comment activity, priority,
//! and the two text searches — are each sent to Linear as a narrowing of `issues(filter:)`
//! and confirmed in process before a row is returned. Each narrowing is a candidate set that
//! cannot miss a row the contract's predicate keeps, which is what makes sending it sound;
//! the confirmation is what makes the answer exact. Every member below is pinned in
//! `tests/fixtures/schema.graphql`, and each rests on one observation of the real API,
//! against the scratch team `TES` on 2026-10-02, which `drive_follow_ups` in `tests/live.rs`
//! asserts again — the comparators by name, and every narrowing through this source with a
//! decoy whose prose carries the searched phrases:
//!
//! - **`IssueFilter.description.contains` reads the whole stored description, the metadata
//!   slot included, and is case-sensitive.** An issue whose slot held
//!   `"caller.key":"needle-…"` was returned for `contains` of that exact phrase, and of the
//!   `"onetaskgraph.origin":"…"` pair beside it; the same description's prose, upper-cased,
//!   was returned for `containsIgnoreCase` and *not* for `contains`. So a metadata match or an
//!   origin is sent as its value in quotes, `"<value>"` — the bytes any JSON encoder writes a
//!   string as, which a slot holding it contains whether it is the code span this source
//!   writes, the multi-line slot it wrote before, or one a person spaced by hand — and
//!   confirmed over the parsed slot. A value holding a character an encoder may escape is not
//!   sent, and the confirmation decides alone.
//! - **`IssueFilter.title.containsIgnoreCase` and `description.containsIgnoreCase` match
//!   regardless of case** — the title `… Alpha Title` was returned for `alpha TITLE`, and not
//!   for `contains` of it. They are the two text searches; the content search is confirmed
//!   over the visible content, because the comparator also reads the slot.
//! - **`IssueFilter.priority.in` narrows by Linear's own number** — an issue at `2` was
//!   returned for `in:[2]` and not for `in:[3]`.
//! - **`IssueFilter.comments.some` with `createdAt`/`updatedAt` `gte` narrows by a comment's
//!   own times** — an issue whose comment had been edited a moment earlier was returned for an
//!   instant before the edit and not for one a day later, and editing the comment moved its
//!   `updatedAt` while leaving `createdAt`. The comment read reports those same two fields,
//!   so the filter and the contract's rule read one value.
//!
//! ## Ruling: what Linear does to an HTML comment, settled
//!
//! A follow-up tool marks what it writes with HTML comments, and this source keeps its own
//! metadata in one, so what survives is a fact this crate records rather than assumes.
//! Observed on 2026-10-02 against the scratch team `TES`, each written and read back by id,
//! and asserted again — the probe text and its stored form exactly — by `drive_follow_ups` in
//! `tests/live.rs`, a leg of its `real_linear_applies_every_declared_capability_and_leaves_no_residue`:
//!
//! - **A comment's `body` keeps every HTML comment byte for byte** — on one line or across
//!   several, a bare `-->` closing line, domain-like text and JSON included.
//! - **An issue's `description` and a document's `content` do not.** Linear stores both as
//!   Markdown and normalizes the text *inside* an HTML comment exactly as it normalizes prose:
//!   a domain-like token or a URL is autolinked — `example.com` comes back
//!   `[example.com](<http://example.com>)`, and a key such as `caller.live` likewise — `[` and
//!   `]` come back `\[` and `\]`, `~` comes back `\~`, `_y_` comes back `*y*`, the backslash
//!   of `\"` is dropped, a backslash before a letter is doubled, and a line opening `-->` comes
//!   back `\-->`. Text with none of those in it — `{"k":"v"}`, `caller.key`, `sha256:…`,
//!   `gh:I_kwDO…` — comes back as written. An autolink can run on past the token, swallowing
//!   what follows it up to the next delimiter.
//! - **One thing in those two fields comes back byte for byte: a code span.** An HTML comment
//!   on one line whose payload is inside backticks — ``<!-- probe `{…}` -->`` — came back
//!   identical with every one of the payloads above inside it, and re-writing what Linear
//!   handed back changed nothing more. So this source writes its own slot that way (see
//!   `METADATA_OPEN_SPAN`), and a marker meant to survive an issue's description or a
//!   document's content belongs in one too.
//!
//! ## Ruling: a Linear document carries no label, and that is Linear's
//!
//! Unlike the two searches above, this one *is* a property of the remote service. The
//! types of Linear's published schema carrying a `labels` field are `Issue`, `Project`,
//! `Team`, `Initiative` and `Organization`; `Document` is not among them, re-observed
//! 2026-09-01 and pinned in `tests/fixtures/schema.graphql`. So this source reports a
//! document's labels as none and **refuses by name** a document write carrying one, rather
//! than dropping it or standing a slot up beside a first-class type. The shared journey
//! table's row says so, and the shared document journeys drive that claim.
//!
//! Two predicates therefore reach a fetched page rather than the `documents(filter:)`
//! variables, and both are still *applied* — which is what `Native` means here, and why
//! the declaration stays honest. Labels, for the reason above. And orphans, because
//! `DocumentFilter.project` is a `ProjectFilter` where `IssueFilter.project` is a
//! `NullableProjectFilter`: only the nullable one carries `null:`, so Linear cannot be
//! asked for the documents belonging to no project. The page-by-page walk asks for only
//! what is still owed, so neither predicate can make a read return more than the caller
//! asked for, and neither can drop a document the walk already fetched.
//!
//! ## Ruling: a comment is read backwards, and its author is Linear's to record
//!
//! **The order.** The contract owes a task's comments oldest first, across pages, and Linear's
//! `Issue.comments` takes no sort direction — only `orderBy`, whose members are `createdAt`
//! (the default) and `updatedAt`. Linear's pagination documentation says results are "ordered
//! by `createdAt`" and that "to get most recently updated resources, you can alternatively
//! order by `updatedAt`", which reads that ordering as newest first. So this source walks the
//! connection from its far end: `last` with `before`, each page reversed, the next page's
//! cursor being `startCursor` while `hasPreviousPage` holds. Reversing within a page and
//! walking backwards across them is what makes the whole walk oldest first rather than each
//! page alone. **That direction is inferred from the documentation's wording rather than
//! observed against the real API,** which is the one reading here a live run has not yet
//! confirmed; if Linear is found to list oldest first, the correction is this walk's
//! direction and nothing else.
//!
//! **The author.** Linear records the user whose credential made the request as a comment's
//! author, and this source authenticates with an API key. `CommentCreateInput.createAsUser`
//! exists but is, in Linear's own words, "only available to OAuth applications creating
//! comments in `actor=app` mode", which a key is not. So a comment carrying an author is
//! **refused before any request is sent**, naming why and what to do instead, rather than
//! posted under a name other than the one it was given. An author read back is the user's
//! `displayName`, which Linear keeps unique within a workspace, and is absent when Linear
//! names no user — a comment an integration or a bot wrote.
//!
//! **What "no such comment" means.** An edit or a removal first asks `comment(id:)` which
//! issue the comment is on, and answers "no such comment" — no mutation sent — unless it is
//! the task's own issue: a comment on another issue, on no issue at all, or trashed, is not a
//! comment this task has. The body is Linear's `body`, which its schema describes as markdown
//! derived from a rich-text document, so what an add or an edit answers with is what Linear
//! now holds rather than an echo of what was sent.
//!
//! ## Ruling: a project's filter is not an issue's, and neither is its status
//!
//! Linear's `IssueFilter` and `ProjectFilter` read as one filter over two kinds of row.
//! They are two input types, and this source built one object for both until 2026-09-04,
//! which put two members into `projects(filter:)` that Linear does not have there. It
//! refused the first outright — `Field "team" is not defined by type "ProjectFilter". Did
//! you mean "lead"?` — and would have refused the second next.
//!
//! A project has no team; it has the teams it is accessible from, so the configured team
//! reaches `accessibleTeams:{some:{key:{eqIgnoreCase:…}}}`. And a project's status is not
//! an issue's state: the counterpart of `IssueFilter.state` is `ProjectFilter.status`,
//! while `ProjectFilter.state` exists and is a bare `StringComparator` over something else.
//! The two do not even share a vocabulary — a project's statuses are the workspace's, Hello
//! Patient's `Idea`, `Proposal`, `Planned`, `Completed` among them, where an issue's states are
//! the team's — which is why `status_mapping` names each kind's statuses separately, and why a
//! filter spelled in the other level's names matches nothing while being refused by nothing.
//!
//! **Neither of those could be caught by reading a document, and that is the general
//! lesson.** A filter is built at runtime and handed over as `$filter`, so it appears in no
//! operation this crate declares, and the two pinned-schema checks that parse those
//! operations could not see it — Linear was the only reader, one refusal per round trip.
//! `every_variables_object_this_source_sends_conforms_to_the_pinned_schema` closes that:
//! it drives this source's whole surface, records what really went out, and walks every
//! variables object against the pinned type of the argument it stands at.
//!
//! ## Ruling: a Linear project relation is always an ordering
//!
//! This one is Linear's too, and the validator says so in as many words. Asked on
//! 2026-09-04 for a project relation typed `related` — and separately `blocks` and
//! `dependsOn` — the real API refused each with `Argument Validation Error` and
//! `constraints: {"isEnum": "type must be one of the following values: dependency"}`. That
//! enumeration has one member and it is a timeline dependency, which is why the input
//! carries an anchor at each end at all.
//!
//! So a project edge carrying no ordering has nowhere here to land, and this source
//! **refuses it by name** before the write rather than sending a value Linear will reject
//! or quietly promoting it to a dependency it does not mean. `DependencyKind::Related`
//! keeps its issue-level spelling, `related`, because `IssueRelationCreateInput` really
//! does take it: the two relations are different relations with different vocabularies,
//! and each level's read accepts only its own.
//!
//! Which end of a project relation waits is carried by the two anchors and not by the two
//! id slots — measured, not reasoned, from Linear's own `ProjectFilter.hasBlockedByRelations`
//! against relations written both ways round. `tests/fixtures/README.md` records the whole
//! probe, and `write_relations` records why the pair this source sends is the oriented one.
//!
//! Caller metadata is canonical JSON in a trailing
//! ``<!-- onetaskgraph.metadata `…` -->`` Markdown comment in the item's description, on one
//! line with the JSON in a code span — the one spelling Linear keeps byte for byte, see the
//! ruling above; the multi-line spelling this source wrote before is still read. The visible
//! description is returned unchanged without that slot. Writes put the same canonical
//! encoding back beside the visible description, and use Linear issue/project relations for
//! same-source dependencies. Only cross-source far ends use the reserved
//! `onetaskgraph.depends_on` metadata key.
//!
//! ## Ruling: a status is the name `status_mapping` gives the item's kind, and nothing else
//!
//! Linear has no built-in names: a team's workflow states and a workspace's project statuses
//! are whatever the people who own them called them, and a type says nothing about which of
//! several states of it a category means — `Todo` and `Queued` are both `unstarted`. So the
//! source's `status_mapping` ([`StatusMapping`]) is the whole of what a status is written as
//! and read by: a task's names are the configured team's workflow states, a project's the
//! workspace's project statuses.
//!
//! **A write** of a category is the name the mapping gives the kind of the item being written —
//! `set_task_status`, the targeted update and `write_task` for a task, so `task create` and every
//! copy; `write_project` for a project, whose own status name plays no part. A write the mapping
//! gives that kind no name for — a category it does not mention, one set to `null`, one a
//! per-kind object leaves out — is refused before any request, naming the source, the kind, the
//! category and the key to set; one whose name that kind's vocabulary does not hold is refused
//! naming the name, after the resolution and before any mutation. Nothing falls back by type, or
//! by the name a status was called where it came from, and a source with no mapping refuses
//! every status write.
//!
//! **A read** of an item at a name its kind's mapping gives is that category, under the name;
//! every other name reads as `unknown`, under its own name, whatever its type — the review
//! states only people write, `Triage` among them. `filter_by_status` returns exactly the items
//! that read as each category asked for: those at the name the kind's mapping gives it, and for
//! `unknown` every item at a name that mapping does not give at all. Each is confirmed in process
//! as well, so a row reading as another category is never returned.
//!
//! ## Ruling: the resolution is read once per source instance, and a status write reads nothing
//!
//! A Linear key is shared by every manager of a host, so what a status write costs is counted at
//! Linear's endpoint, and `crates/onetaskgraph-linear/budgets.yaml` holds it. A source resolves
//! the configured team's id, its workflow states and the workspace's project statuses in one
//! request — [`graphql::RESOLUTION`] — the first time a write or `sources fields` needs them, and
//! holds the answer for its own lifetime: per instance, never per process, never shared between
//! sources. Building a source sends nothing. A mapped name the held answer lacks is looked for
//! once more in a fresh read, so one added in Linear since is found; nothing a failed call
//! answered is held, and a write that fails while carrying a held id drops the answer, so the
//! next reads afresh rather than sending that id again. A name `sources fields --apply` creates is
//! added to what is held.
//!
//! `set_task_status`, and a targeted update naming a status and nothing else, are one
//! `issueUpdate` and no read: [`graphql::ISSUE_UPDATE_READ`] selects the whole issue, which is
//! what the answered status — and the engine keeping a delivered task in step — need. So writing
//! the category an issue already reads as sends the same state again, setting `unknown` on an
//! issue at a name the mapping does not give moves it to the mapped `unknown` name, and an issue
//! Linear does not hold is no such task from the mutation's own refusal — `Entity not found` —
//! rather than from a read. That spelling is the one Linear documents for its own input
//! validation; a live run has not yet re-observed it here. A source scoped to one project is no
//! exception: its status write goes to the issue it names wherever that issue is filed — see
//! the scope's ruling below. A targeted update naming anything else
//! keeps its one read of the issue: the metadata slot is merged into the description Linear
//! holds, and writing it without that read would overwrite whatever a person wrote there since.
//! A whole rewrite of an issue or a project reads the relations it replaces in its own answer
//! ([`graphql::ISSUE_REWRITE`], [`graphql::PROJECT_REWRITE`]) rather than in a read of its own.
//!
//! ## Ruling: `sources fields --apply` creates every name the mapping needs
//!
//! For parity with a GitHub Projects board, whose `--apply` adds the `Status` options it lacks:
//! `--apply` creates each name the mapping gives a task that the team lacks, as a workflow state
//! (`workflowStateCreate`), and each name it gives a project that the workspace lacks, as a
//! project status (`projectStatusCreate`) — a bare name in both. The type of what it creates is
//! its category's, by this fixed table:
//!
//! | Category | Workflow state | Project status |
//! | --- | --- | --- |
//! | `backlog`, `draft` | `backlog` | `backlog` |
//! | `todo`, `queued` | `unstarted` | `planned` |
//! | `in-progress`, `unknown` | `started` | `started` |
//! | `done` | `completed` | `completed` |
//! | `cancelled` | `canceled` | `canceled` |
//!
//! Both create inputs also require a colour, and a project status a place in the workspace's
//! flow, and nothing in a mapping says either: every name is created in Linear's neutral grey,
//! `#95a2b3`, and a project status after the workspace's last. Nothing that exists is renamed, retyped or deleted, and a name present
//! under another type is reported with its type and left as it is. A create Linear refuses stops
//! the run, and the report names what it created before it. Whether to run it against a
//! workspace is the operator's decision.
//!
//! ## Ruling: `project` scopes a source to one project
//!
//! With `project` set, every issue read carries `project:{id:{eq:…}}` beside the team, a
//! project read carries `id:{eq:…}` and a document read the same project, and a read by id
//! of anything filed elsewhere answers as no such item — so a content, a metadata or a comment
//! write to it, each of which reads the item first, is answered the same way.
//!
//! **A status-only write is the exception, and goes to the item it names wherever that item is
//! filed.** `task status set`, and a targeted update naming a status and nothing else, are one
//! `issueUpdate` to the issue named, with no read before it — so a scoped source meets the same
//! request budgets as an unscoped one — and answer the issue as it now reads, even when it is
//! filed in another project of the team. Linear has no update conditional on where an issue is
//! filed, so holding a status write to the scope would cost the read the budget refuses; the
//! item was named outright, and the scope is what this source reads, lists and creates rather
//! than where a status it is asked to set may land. Reads, listings and creation stay scoped: a
//! task or a document written with no project is placed in that one, and one naming another is
//! refused naming both. A project write other than to that project itself is refused before any
//! request: a project this source created would be one none of its reads could find.
//!
//! ## Ruling: a narrow metadata write moves only the slot, and a task carries delivery
//!
//! `set_task_metadata`, `set_project_metadata` and `set_document_metadata` read the item and
//! send one update of its long-form field — `description`, `description` and `content` —
//! that differs from what Linear holds only inside the trailing metadata slot: every byte
//! above it is kept as it was. A key already holding the value sends nothing. The answer is
//! the item read back. `set_task_rendering` and `set_document_rendering` replace the content
//! and the slot's `onetaskgraph.template` entry together in one such update, every other slot
//! entry kept; this source keeps no template answers. So a copy's `onetaskgraph.copies` link
//! is recorded on a Linear item rather than reported unrecorded.
//!
//! A task's `delivers` and `delivered_by` live in that same slot under
//! `onetaskgraph.delivers` and `onetaskgraph.delivered_by`, each a list of qualified ids,
//! with the shape and the rules the GitHub Projects plugin keeps: neither may name the task
//! itself or name one task twice, which is refused by name before anything is sent; a write
//! lands the typed lists in place of any caller metadata of those names; and
//! `set_delivered_by` is one update of the slot. A project or a document naming either key is
//! refused, because only a task delivers or is delivered.
//!
//! ## Ruling: a priority is Linear's own, and content shares a field with the slot
//!
//! A task's priority is `Issue.priority`, on Linear's scale: `0` none, `1` urgent, `2` high,
//! `3` normal — this contract's `medium` — and `4` low. Linear declares the field `Float!`
//! while `IssueCreateInput.priority` and `IssueUpdateInput.priority` are `Int`, so a read
//! accepts `2` and `2.0` alike and refuses anything that is not one of the five as a
//! malformed response naming the field. A copy sends it on a create and on an update, `0`
//! included, so a task moved back to no priority is not left holding its old one.
//! `set_task_priority` reads the issue first — no such issue, or a trashed one, is `None`
//! with nothing written — then sends `issueUpdate` with `priority` alone and answers with
//! the priority the mutation's own payload reports.
//!
//! `set_task_content` sends `issueUpdate` with `description` alone, and that description is
//! the given content followed by the issue's metadata slot exactly as it was stored, so the
//! slot, and every key in it, is untouched. What a later read reports as the content is the
//! given bytes, trailing whitespace included: a read of an issue carrying a slot takes off only
//! the one blank line that sets the slot off, and a write whose content would not read back as
//! itself is refused before it is sent.
//!
//! Fixture provenance is recorded in `tests/fixtures/README.md`. The live journey in
//! `tests/live.rs` drives every field of the table above against Linear itself: it builds its own fixture
//! on the scratch team `LINEAR_WRITE_TEAM` names — two projects, one issue filed under
//! each, one filed under neither, two labels and two workflow states — because that shape
//! is what tells an honoured predicate from an ignored one, and a workspace where every
//! issue carries the label answers a filter the same way either way. Everything the lane
//! creates it deletes whether its assertions passed or failed, and it clears residue named
//! the way it names its own before it starts. A failed live cleanup is reported as a test
//! failure and may require manual deletion from that scratch team.
#![deny(missing_docs)]

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind,
    DependencySupport, Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label,
    LabelFilter, Location, MetadataKey, NativeId, NewComment, Page, PageRequest, Priority, Project,
    ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName, SourcePlugin,
    Status, StatusCategory, StatusMapping, StatusName, Support, Task, TaskQuery, TaskRef,
    TaskSource, TaskUpdate, TaskUpdateOutcome, TextFields, TextQuery, UpdatedField, WriteSupport,
};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

/// The plugin kind a `linear` source's `plugin:` field names.
pub const KIND: &str = "linear";

/// The largest page this source will ask Linear for, and the capability it declares.
///
/// **Not Linear's connection maximum, which is 250, because a connection maximum is not
/// the only thing bounding a page.** Linear also scores each document for complexity and
/// refuses one over 10000 with HTTP 400 and `The query is too complex.` — and the
/// `projects` document this source sends scores 17475 at `first: 250`, because its nested
/// `labels` connection, which names no `first` of its own, is charged Linear's default of
/// 50 per node. Measured against the real API on 2026-09-04: the largest `first` that
/// document is accepted at is **143**, exactly, and the filter it carries adds nothing.
/// The `issues` document is accepted at 250, so this is the tighter of the two and a
/// single declared maximum has to be the tighter one.
///
/// 100 rather than 143 because 143 is the cliff. A field added to either selection moves
/// it, and a page size chosen at the edge of a budget nobody here controls fails in the
/// live lane rather than in a check. This leaves 30% of the budget spare.
///
/// Nothing offline can hold this: complexity is scored by Linear's own runtime and appears
/// in no schema, so `every_variables_object_this_source_sends_conforms_to_the_pinned_schema`
/// cannot see it. What guards it is the live journey, which walks a real `projects` page at
/// exactly this size.
pub const MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_ENDPOINT: &str = "https://api.linear.app/graphql";

/// Exact GraphQL query documents issued by this plugin.
///
/// Fixture servers consume these constants so their recognized contract cannot drift
/// from the production requests.
pub mod graphql {
    /// Linear's missing-issue error, reconciled with the service by the live status journey.
    pub const ISSUE_NOT_FOUND_MESSAGE: &str = "Entity not found: Issue";
    /// Its user-facing alternative; the loopback fixture shares this contract.
    pub const ISSUE_NOT_FOUND_PRESENTABLE: &str = "Could not find referenced Issue.";

    /// Check the authenticated viewer.
    pub const VIEWER: &str = "query { viewer { id } }";
    /// Fetch one issue.
    pub const ISSUE: &str = "query($id:String!){ issue(id:$id){ id identifier title description url createdAt updatedAt archivedAt state{name type} priority labels{nodes{id name color}} project{id} } }";
    /// Fetch one project.
    pub const PROJECT: &str = "query($id:String!){ project(id:$id){ id name description url createdAt updatedAt archivedAt status{name type} labels{nodes{id name color}} } }";
    /// List issues.
    pub const ISSUES: &str = "query($first:Int!,$after:String,$filter:IssueFilter){ issues(first:$first,after:$after,filter:$filter){ nodes{id identifier title description url createdAt updatedAt state{name type} priority labels{nodes{id name color}} project{id}} pageInfo{hasNextPage endCursor} } }";
    /// List projects.
    pub const PROJECTS: &str = "query($first:Int!,$after:String,$filter:ProjectFilter){ projects(first:$first,after:$after,filter:$filter){ nodes{id name description url createdAt updatedAt status{name type} labels{nodes{id name color}}} pageInfo{hasNextPage endCursor} } }";
    /// List issue labels.
    pub const LABELS: &str = "query($first:Int,$after:String){ issueLabels(first:$first,after:$after){ nodes{id name color} pageInfo{hasNextPage endCursor} } }";
    /// Fetch issue dependency relations.
    pub const ISSUE_RELATIONS: &str = "query($id:String!,$first:Int!,$after:String){ issue(id:$id){ description relations(first:$first,after:$after){nodes{id type relatedIssue{id}} pageInfo{hasNextPage endCursor}} inverseRelations(first:$first,after:$after){nodes{id type issue{id}} pageInfo{hasNextPage endCursor}} } }";
    /// Fetch project dependency relations.
    pub const PROJECT_RELATIONS: &str = "query($id:String!,$first:Int!,$after:String){ project(id:$id){ description relations(first:$first,after:$after){nodes{id type relatedProject{id}} pageInfo{hasNextPage endCursor}} inverseRelations(first:$first,after:$after){nodes{id type project{id}} pageInfo{hasNextPage endCursor}} } }";
    /// Everything a status write resolves, in one request: the configured team's id, that
    /// team's workflow states, and the workspace's project statuses, each with its type.
    ///
    /// One document rather than three lookups, because a status write's cost is counted at
    /// Linear's endpoint on a key every manager of a workspace shares: a source sends this once
    /// and holds the answer for its own lifetime (see the ruling on the resolution cache in this
    /// crate's module documentation). `Team.states` and `Query.projectStatuses` are each read
    /// as one page of Linear's connection maximum, 250, and refused rather than read short when
    /// either says it has more — a team holds a few dozen states at most, and a workspace a
    /// few dozen project statuses, so a resolution that does not fit one page is not one this
    /// source guesses at — and `projectStatuses` takes no `filter`: Linear refuses one outright with `Unknown argument "filter" on field
    /// "Query.projectStatuses"`, which is why a project status is matched by name here, locally.
    /// `position` is read for one reason: a project status `sources fields --apply` creates is
    /// placed after the workspace's last.
    ///
    /// `teams` is read at `first:2` and never at Linear's default page of 50, because Linear
    /// scores a connection's selection once per node its page may hold: at the default, the 250
    /// states asked of each of fifty possible teams scored this document 30905 against Linear's
    /// limit of 10000, and the live journey's first status write was refused `Query too complex`.
    /// Two rather than one so a key matching more than one team is still seen, and refused, by
    /// the exactly-one rule that reads this answer. `states` and `projectStatuses` stay at 250,
    /// whole or refused, as above: a name a page cut off would read as missing.
    ///
    /// Under Linear's documented model — a property 0.1, an object 1, and a connection's
    /// children multiplied by its `first`, or 50 without one — the refused document scores
    /// 50 × (1 + 0.1 + 250 × (1.3 + 1.1)) + 250 × (1.4 + 1.1) = 30680, and this one
    /// 2 × 601.1 + 625 = 1827.2. Linear's own figures run 225 above the model on both
    /// documents it has reported here (30905 for the refused one, 17475 for [`PROJECTS`] at
    /// 250), and one point more for every node of every connection at every depth — 752 here —
    /// still leaves this under 2580, about a quarter of the limit. Like [`super::MAX_PAGE_SIZE`],
    /// nothing offline can hold this — complexity appears in no schema — and the live journey
    /// is what guards it.
    pub const RESOLUTION: &str = "query($key:String!){ teams(first:2,filter:{key:{eqIgnoreCase:$key}}){nodes{id states(first:250){nodes{id name type} pageInfo{hasNextPage}}}} projectStatuses(first:250){nodes{id name type position} pageInfo{hasNextPage}} }";
    /// Create a workflow state on the configured team, for `sources fields --apply`.
    pub const WORKFLOW_STATE_CREATE: &str = "mutation($input:WorkflowStateCreateInput!){ workflowStateCreate(input:$input){success workflowState{id name type}} }";
    /// Create a workspace project status, for `sources fields --apply`.
    pub const PROJECT_STATUS_CREATE: &str = "mutation($input:ProjectStatusCreateInput!){ projectStatusCreate(input:$input){success status{id name type position}} }";
    /// Resolve an issue-label display name.
    pub const ISSUE_LABEL: &str =
        "query($name:String!){ issueLabels(filter:{name:{eqIgnoreCase:$name}}){nodes{id}} }";
    /// Resolve a project-label display name.
    pub const PROJECT_LABEL: &str =
        "query($name:String!){ projectLabels(filter:{name:{eqIgnoreCase:$name}}){nodes{id}} }";
    /// Create an issue.
    pub const ISSUE_CREATE: &str =
        "mutation($input:IssueCreateInput!){ issueCreate(input:$input){success issue{id}} }";
    /// Update an issue.
    pub const ISSUE_UPDATE: &str = "mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success issue{id}} }";
    /// Update an issue and read back, in the same request, everything a task is read as.
    ///
    /// What a status write and a targeted update answer with: the selection is [`ISSUE`]'s, so
    /// the task they report — its status, and the `delivers` the engine keeps in step with it —
    /// is what Linear holds after the write, with no read before it and none after.
    pub const ISSUE_UPDATE_READ: &str = "mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success issue{ id identifier title description url createdAt updatedAt archivedAt state{name type} priority labels{nodes{id name color}} project{id} }} }";
    /// Set an issue's priority on its own, and read back the priority Linear now holds.
    ///
    /// The same `issueUpdate` as [`ISSUE_UPDATE`], selecting `priority` in the payload
    /// because a narrow priority write answers with what the source reads back rather than
    /// an echo of what it sent. A document of its own rather than a wider [`ISSUE_UPDATE`],
    /// so every other issue write keeps asking for exactly what it reads.
    pub const ISSUE_PRIORITY_UPDATE: &str = "mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success issue{id priority}} }";
    /// Rewrite an issue whole, and read back in the same request the first page of the
    /// relations it holds — the ones a whole write replaces.
    ///
    /// A whole write of an existing item replaces every relation it holds with the ones it was
    /// given, so it has to know which it holds; selected here, that is the update's own answer
    /// rather than a read of its own, and a re-write with no relations to replace is the one
    /// request. The relations are read after the update, which moves none of them.
    pub const ISSUE_REWRITE: &str = "mutation($id:String!,$input:IssueUpdateInput!,$first:Int!){ issueUpdate(id:$id,input:$input){success issue{id relations(first:$first){nodes{id type relatedIssue{id}} pageInfo{hasNextPage endCursor}}}} }";
    /// Rewrite a project whole, and read back the first page of its relations, on the terms of
    /// [`ISSUE_REWRITE`].
    pub const PROJECT_REWRITE: &str = "mutation($id:String!,$input:ProjectUpdateInput!,$first:Int!){ projectUpdate(id:$id,input:$input){success project{id relations(first:$first){nodes{id type relatedProject{id}} pageInfo{hasNextPage endCursor}}}} }";
    /// Create a project.
    pub const PROJECT_CREATE: &str =
        "mutation($input:ProjectCreateInput!){ projectCreate(input:$input){success project{id}} }";
    /// Update a project.
    pub const PROJECT_UPDATE: &str = "mutation($id:String!,$input:ProjectUpdateInput!){ projectUpdate(id:$id,input:$input){success project{id}} }";
    /// Create a native issue dependency.
    pub const ISSUE_RELATION_CREATE: &str = "mutation($input:IssueRelationCreateInput!){ issueRelationCreate(input:$input){success issueRelation{id}} }";
    /// Create a native project dependency.
    pub const PROJECT_RELATION_CREATE: &str = "mutation($input:ProjectRelationCreateInput!){ projectRelationCreate(input:$input){success projectRelation{id}} }";
    /// Delete a native issue dependency before replacing its full edge set.
    pub const ISSUE_RELATION_DELETE: &str =
        "mutation($id:String!){ issueRelationDelete(id:$id){success} }";
    /// Delete a native project dependency before replacing its full edge set.
    pub const PROJECT_RELATION_DELETE: &str =
        "mutation($id:String!){ projectRelationDelete(id:$id){success} }";
    /// Delete an issue, so a copy that could not finish can take back what it created.
    pub const ISSUE_DELETE: &str = "mutation($id:String!){ issueDelete(id:$id){success} }";
    /// Delete a project, for the same reason and on the same terms.
    pub const PROJECT_DELETE: &str = "mutation($id:String!){ projectDelete(id:$id){success} }";
    /// Fetch one document.
    pub const DOCUMENT: &str = "query($id:String!){ document(id:$id){ id title content url createdAt updatedAt archivedAt project{id} } }";
    /// List documents.
    ///
    /// `first` is an `Int` rather than an `Int!` because that is what Linear's `documents`
    /// connection declares, unlike its `issues` one.
    pub const DOCUMENTS: &str = "query($first:Int,$after:String,$filter:DocumentFilter){ documents(first:$first,after:$after,filter:$filter){ nodes{id title content url createdAt updatedAt project{id}} pageInfo{hasNextPage endCursor} } }";
    /// Create a document.
    pub const DOCUMENT_CREATE: &str = "mutation($input:DocumentCreateInput!){ documentCreate(input:$input){success document{id}} }";
    /// Update a document.
    pub const DOCUMENT_UPDATE: &str = "mutation($id:String!,$input:DocumentUpdateInput!){ documentUpdate(id:$id,input:$input){success document{id}} }";
    /// Delete a document, so a copy that could not finish can take back what it created.
    pub const DOCUMENT_DELETE: &str = "mutation($id:String!){ documentDelete(id:$id){success} }";
    /// One page of an issue's comments, walked backwards.
    ///
    /// `last`/`before` rather than `first`/`after`, and `pageInfo{hasPreviousPage
    /// startCursor}` rather than its forward pair, because Linear lists a connection newest
    /// first and the contract owes the oldest first — see the ruling on comments in this
    /// crate's module documentation. `archivedAt` is selected for the reason every by-id read
    /// here selects it: a trashed issue is not an issue this source holds.
    pub const ISSUE_COMMENTS: &str = "query($id:String!,$last:Int,$before:String){ issue(id:$id){ archivedAt comments(last:$last,before:$before){ nodes{id body url createdAt updatedAt user{displayName}} pageInfo{hasPreviousPage startCursor} } } }";
    /// Place one comment: which issue it is on, if any.
    ///
    /// `$id` is a nullable `String` because that is what `Query.comment` declares — it also
    /// takes a `hash` instead — and a variable has to be exactly its argument's type.
    pub const COMMENT: &str = "query($id:String){ comment(id:$id){ id archivedAt issue{id} } }";
    /// Add a comment to an issue.
    pub const COMMENT_CREATE: &str = "mutation($input:CommentCreateInput!){ commentCreate(input:$input){success comment{id body url createdAt updatedAt user{displayName}}} }";
    /// Replace a comment's body.
    pub const COMMENT_UPDATE: &str = "mutation($id:String!,$input:CommentUpdateInput!){ commentUpdate(id:$id,input:$input){success comment{id body url createdAt updatedAt user{displayName}}} }";
    /// Remove a comment.
    pub const COMMENT_DELETE: &str = "mutation($id:String!){ commentDelete(id:$id){success} }";
}

use graphql::{
    DOCUMENT, DOCUMENTS, ISSUE, ISSUE_RELATIONS, ISSUES, LABELS, PROJECT, PROJECT_RELATIONS,
    PROJECTS, VIEWER,
};

/// One `linear` source's configuration.
///
/// It names the credential's environment variable, never its value. Serializable so the
/// schema it is published under carries each member's default, which is what a configuration
/// that leaves the member out means.
#[derive(Debug, Clone, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LinearConfig {
    /// Environment variable resolved by the host.
    #[schemars(with = "String")]
    api_key_env: EnvName,
    /// Linear team key/id used to narrow reads and required for item writes.
    team: Option<Team>,
    /// GraphQL endpoint override, primarily for fixture servers.
    #[schemars(with = "String")]
    endpoint: Endpoint,
    /// Which workflow state of the configured team a task's status category is, and which of
    /// the workspace's project statuses a project's is — the shared `status_mapping` grammar.
    ///
    /// Linear has no built-in names, so this is the whole of what a status is written as and
    /// read by: a category is written as exactly the name its kind maps it to, and a status
    /// write it gives no name for is refused before any mutation, naming the key to set; an
    /// item at a name its kind maps reads as that category under the name, and every other
    /// name reads as `unknown`. A source without it reads every item as `unknown` and refuses
    /// every status write. Two categories mapped to one name of one kind are refused when this
    /// configuration is read.
    status_mapping: StatusMapping,
    /// The id of one Linear project of the configured team, scoping this source to it.
    ///
    /// When set, every task read is narrowed to that project's issues, project and document
    /// reads return only that project and its documents, a task or a document written with
    /// no project is placed in it, and one naming another project is refused. Absent, the
    /// source reads and writes team-wide.
    project: Option<ProjectScope>,
}

/// The id of one Linear project, which a scoped source holds alone.
#[derive(Debug, Clone, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(rename = "LinearProjectId", extend("minLength" = 1))]
struct ProjectScope(String);
impl From<ProjectScope> for String {
    fn from(value: ProjectScope) -> Self {
        value.0
    }
}
impl TryFrom<String> for ProjectScope {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            Err("a project id cannot be blank".into())
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(try_from = "String", into = "String")]
struct EnvName(String);
impl From<EnvName> for String {
    fn from(value: EnvName) -> Self {
        value.0
    }
}
impl TryFrom<String> for EnvName {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let mut bytes = value.bytes();
        if bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_uppercase())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            Ok(Self(value))
        } else {
            Err("must be an uppercase environment-variable name".into())
        }
    }
}
/// A Linear team's key or id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(rename = "LinearTeam", extend("minLength" = 1))]
struct Team(String);
impl From<Team> for String {
    fn from(value: Team) -> Self {
        value.0
    }
}
impl TryFrom<String> for Team {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            Err("must not be empty".into())
        } else {
            Ok(Self(value))
        }
    }
}
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(try_from = "String", into = "String")]
struct Endpoint(String);
impl From<Endpoint> for String {
    fn from(value: Endpoint) -> Self {
        value.0
    }
}
impl TryFrom<String> for Endpoint {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let url = reqwest::Url::parse(&value).map_err(|e| e.to_string())?;
        if matches!(url.scheme(), "http" | "https") {
            Ok(Self(value))
        } else {
            Err("must use http or https".into())
        }
    }
}

impl Default for LinearConfig {
    fn default() -> Self {
        Self {
            api_key_env: EnvName("LINEAR_API_KEY".into()),
            team: None,
            endpoint: Endpoint(DEFAULT_ENDPOINT.into()),
            status_mapping: StatusMapping::default(),
            project: None,
        }
    }
}

/// An item kind as this source's configuration and reports spell it.
const fn kind_word(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Task => "task",
        ItemKind::Project => "project",
    }
}

/// What a name of `kind` is called in Linear, for a message.
const fn vocabulary_word(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Task => "workflow state",
        ItemKind::Project => "project status",
    }
}

/// A `WorkflowState.type` a workflow state `sources fields --apply` creates is given — the five
/// Linear's `WorkflowStateCreateInput.type` documents taking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreatedStateType {
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

impl CreatedStateType {
    /// The type as Linear spells it.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Unstarted => "unstarted",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }
}

/// A `ProjectStatusType` a project status `sources fields --apply` creates is given — a member
/// of that enum, which `paused` is too and which nothing here creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreatedStatusType {
    Backlog,
    Planned,
    Started,
    Completed,
    Canceled,
}

impl CreatedStatusType {
    /// The type as Linear spells it.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Planned => "planned",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }
}

/// The `WorkflowState.type` a workflow state `sources fields --apply` creates for `category`
/// is given, and the `ProjectStatusType` a project status it creates is given.
///
/// One row per category, fixed: a category is created as the type whose meaning is nearest
/// it, and `draft`, `queued` and `unknown` — which no type of either vocabulary means — as the
/// nearest neighbour's. An exhaustive match, so a category the contract adds fails to compile
/// here rather than being created as something unstated.
const fn created_types(category: StatusCategory) -> (CreatedStateType, CreatedStatusType) {
    match category {
        StatusCategory::Backlog | StatusCategory::Draft => {
            (CreatedStateType::Backlog, CreatedStatusType::Backlog)
        }
        StatusCategory::Todo | StatusCategory::Queued => {
            (CreatedStateType::Unstarted, CreatedStatusType::Planned)
        }
        StatusCategory::InProgress | StatusCategory::Unknown => {
            (CreatedStateType::Started, CreatedStatusType::Started)
        }
        StatusCategory::Done => (CreatedStateType::Completed, CreatedStatusType::Completed),
        StatusCategory::Cancelled => (CreatedStateType::Canceled, CreatedStatusType::Canceled),
    }
}

/// The type, as Linear spells it, a name of `kind` that `sources fields --apply` creates for
/// `category` is given.
const fn created_type(category: StatusCategory, kind: ItemKind) -> &'static str {
    let (state, status) = created_types(category);
    match kind {
        ItemKind::Task => state.as_str(),
        ItemKind::Project => status.as_str(),
    }
}

/// The colour every workflow state and project status `sources fields --apply` creates is
/// given: Linear's own neutral grey, because both create inputs require one and nothing in a
/// status mapping says which colour a name should be. A person recolours it in Linear.
const CREATED_COLOR: &str = "#95a2b3";

/// One workflow state of the configured team, or one project status of the workspace.
#[derive(Debug, Clone)]
struct Held {
    id: NativeId,
    name: StatusName,
    /// Its `WorkflowState.type` or `ProjectStatusType`, verbatim: Linear adds types this
    /// source must not refuse (`duplicate` among them).
    // llmlint: ignore[invalid_states_unrepresentable] Linear's own open `String!` vocabulary, held verbatim for a report; an enum here would refuse a type Linear adds.
    kind: String,
}

impl Held {
    /// One workflow state or project status, as Linear answered it.
    fn read(node: &Value) -> Result<Self, SourceError> {
        Ok(Self {
            id: NativeId(backend_id(node, "id")?.into()),
            name: held_name(node)?,
            kind: str_at(node, "type")?.to_owned(),
        })
    }
}

/// A project status's place in the workspace's project flow, as Linear answered it.
fn position_of(node: &Value) -> Result<f64, SourceError> {
    node.get("position")
        .and_then(Value::as_f64)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing number field position".into(),
        })
}

/// One name `sources fields --apply` created, as Linear answered its create.
#[derive(Debug, Clone)]
enum Created {
    /// A workflow state of the team.
    State(Held),
    /// A project status of the workspace, at its place in the workspace's project flow.
    Status { held: Held, position: f64 },
}

impl Created {
    fn held(&self) -> &Held {
        match self {
            Self::State(held) | Self::Status { held, .. } => held,
        }
    }
}

/// The name of one workflow state or project status Linear answered with, which no name in
/// Linear is blank.
fn held_name(node: &Value) -> Result<StatusName, SourceError> {
    StatusName::try_from(str_at(node, "name")?.to_owned()).map_err(|refused| {
        SourceError::Malformed {
            message: format!("Linear answered a status name this source cannot hold: {refused}"),
        }
    })
}

/// What a status write resolves: the configured team's id, its workflow states and the
/// workspace's project statuses.
///
/// Read in one request and held by one source instance for its own lifetime — never by the
/// process, never shared between sources, and kept across [`TaskSource::end_command`] — so
/// every status write after the first sends its mutation alone. A name not found in it is
/// looked for once more in a fresh read, so a name added in Linear after it was read is found;
/// nothing a failed call answered is held.
#[derive(Debug, Clone)]
struct Vocabulary {
    team: NativeId,
    states: Vec<Held>,
    statuses: Vec<Held>,
    /// Where the workspace's project flow ends: the greatest `position` among its project
    /// statuses, zero when it holds none. A project status `sources fields --apply` creates is
    /// placed after it.
    last_position: f64,
}

impl Vocabulary {
    fn read(data: &Value, source: &SourceName, team: &str) -> Result<Self, SourceError> {
        let teams = data
            .pointer("/teams/nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "missing teams.nodes".into(),
            })?;
        let found = match teams.as_slice() {
            [found] => found,
            other => {
                return Err(SourceError::Refused {
                    message: format!(
                        "source {source} cannot resolve the configured team {team:?}: found {} \
                         matches; next: set team to the key of exactly one team this \
                         credential can see",
                        other.len()
                    ),
                });
            }
        };
        let nodes = |pointer: &str| {
            data.pointer(pointer)
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: format!(
                        "missing {}",
                        pointer.trim_start_matches('/').replace('/', ".")
                    ),
                })
        };
        let team = NativeId(backend_id(found, "id")?.into());
        // Read whole or not at all: a name on a page this did not read would be refused as
        // missing, or a second name of that spelling would go unseen.
        for (more, held) in [
            (
                "/teams/nodes/0/states/pageInfo/hasNextPage",
                "workflow states of its team",
            ),
            (
                "/projectStatuses/pageInfo/hasNextPage",
                "project statuses of its workspace",
            ),
        ] {
            let more = data.pointer(more).and_then(Value::as_bool).ok_or_else(|| {
                SourceError::Malformed {
                    message: format!(
                        "missing boolean {}",
                        more.trim_start_matches('/').replace('/', ".")
                    ),
                }
            })?;
            if more {
                return Err(SourceError::Refused {
                    message: format!(
                        "source {source} reads the {held} in one page of 250, and Linear holds \
                         more; next: archive the ones no longer used, so the rest fit one page"
                    ),
                });
            }
        }
        let statuses = nodes("/projectStatuses/nodes")?;
        Ok(Self {
            team,
            states: nodes("/teams/nodes/0/states/nodes")?
                .iter()
                .map(Held::read)
                .collect::<Result<_, _>>()?,
            statuses: statuses.iter().map(Held::read).collect::<Result<_, _>>()?,
            last_position: statuses
                .iter()
                .map(position_of)
                .try_fold(0.0_f64, |last, position| Ok(last.max(position?)))?,
        })
    }

    /// Hold one name `sources fields --apply` created beside the ones read.
    fn add(&mut self, created: Created) {
        match created {
            Created::State(held) => self.states.push(held),
            Created::Status { held, position } => {
                self.statuses.push(held);
                self.last_position = self.last_position.max(position);
            }
        }
    }

    fn of(&self, kind: ItemKind) -> &[Held] {
        match kind {
            ItemKind::Task => &self.states,
            ItemKind::Project => &self.statuses,
        }
    }

    /// The one name of `kind` matching `name` ignoring case, `None` when there is none, and a
    /// refusal naming every id when there are several: a write must not guess between them.
    fn find(
        &self,
        kind: ItemKind,
        name: &str,
        source: &SourceName,
    ) -> Result<Option<&Held>, SourceError> {
        let matched = self
            .of(kind)
            .iter()
            .filter(|held| held.name.matches(name))
            .collect::<Vec<_>>();
        match matched.as_slice() {
            [] => Ok(None),
            [held] => Ok(Some(held)),
            several => Err(SourceError::Refused {
                message: format!(
                    "source {source} cannot resolve {} {name:?}: found {} matches with ids {:?}",
                    vocabulary_word(kind),
                    several.len(),
                    several
                        .iter()
                        .map(|held| held.id.0.as_str())
                        .collect::<Vec<_>>()
                ),
            }),
        }
    }
}

/// The Linear plugin factory.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn config_schema(&self) -> Schema {
        schema_for!(LinearConfig)
    }
    fn build(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: LinearConfig =
            serde_json::from_value(config.clone()).map_err(|e| SourceError::Config {
                message: format!("source {name}: {e}"),
            })?;
        Ok(Box::new(LinearSource::new(name, config, secrets)?))
    }
}

/// What `onetaskgraph sources fields` reports for a `linear` source: every name its
/// `status_mapping` gives a task, checked against the configured team's workflow states, and
/// every name it gives a project, checked against the workspace's project statuses.
///
/// With `--apply`, each name a kind's vocabulary lacks is created first — a workflow state on
/// the team, a project status in the workspace — of the type its category derives (see the
/// crate's ruling on `sources fields --apply`), and the report names what it created. Nothing that exists is renamed,
/// retyped or deleted. A create Linear refuses stops the run: [`Self::refused`] names it, and
/// every name created before it is reported created.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct StatusNamesReport {
    /// The configured source name.
    pub source: SourceName,
    /// The configured team, as `team` names it.
    team: Team,
    /// Every name `status_mapping` gives a task, then every name it gives a project, each in
    /// category order. A bare name is reported once for each kind. Empty when it names none.
    pub names: Vec<MappedStatusName>,
    /// The create Linear refused, which stopped an `--apply`; absent when nothing was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<RefusedCreate>,
}

impl StatusNamesReport {
    /// The configured team, as `team` names it.
    #[must_use]
    pub fn team(&self) -> &str {
        &self.team.0
    }
}

/// A name `sources fields --apply` could not create, and why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct RefusedCreate {
    /// Whether it was a workflow state (`task`) or a project status (`project`).
    pub kind: ItemKind,
    /// The name it would have created.
    pub name: StatusName,
    /// What Linear said.
    pub message: String,
}

/// One name a `status_mapping` gives one kind, and whether that kind's vocabulary has it.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedStatusName {
    kind: ItemKind,
    category: StatusCategory,
    name: StatusName,
    found: Found,
}

/// Whether a kind's vocabulary has a name of the one the mapping gives, and whether this run
/// put it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// It has it, of this type — a `WorkflowState.type` (`backlog`, `unstarted`, `started`,
    /// `completed`, `canceled`, `triage`, …) or a `ProjectStatusType` (`backlog`, `planned`,
    /// `started`, `paused`, `completed`, `canceled`) — reported verbatim because Linear adds
    /// types this report must not refuse.
    // llmlint: ignore[invalid_states_unrepresentable] Linear's own open `String!` vocabulary, reported verbatim; an enum here would refuse a type Linear adds, which a report must not.
    Present(String),
    /// It has no name of that spelling.
    Missing,
    /// It had none, and `sources fields --apply` created it, of this type.
    // llmlint: ignore[invalid_states_unrepresentable] As `Present`: Linear's own open `String!` vocabulary, reported verbatim as Linear answered the create.
    Created(String),
}

impl MappedStatusName {
    /// Whether the name is a task's workflow state or a project's project status.
    #[must_use]
    pub fn kind(&self) -> ItemKind {
        self.kind
    }

    /// The category the mapping sends to the name.
    #[must_use]
    pub fn category(&self) -> StatusCategory {
        self.category
    }

    /// The name, as the mapping spells it.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Whether the vocabulary has it, and of which type.
    #[must_use]
    pub fn found(&self) -> &Found {
        &self.found
    }

    /// Whether this run created it.
    #[must_use]
    pub fn created(&self) -> bool {
        matches!(self.found, Found::Created(_))
    }

    /// The type a name of this category is created as — what a present name of another type
    /// differs from.
    #[must_use]
    pub fn expected_type(&self) -> &'static str {
        created_type(self.category, self.kind)
    }
}

/// [`MappedStatusName`] as it is written: `present`, and the name's `type` where it is.
///
/// The wire shape of the report, spelled once for its serialization and its schema, so the
/// public type can hold only the combinations [`Found`] allows.
#[derive(serde::Serialize, schemars::JsonSchema)]
#[schemars(rename = "MappedStatusName")]
struct MappedStatusNameWire<'a> {
    /// `task` for a workflow state of the team, `project` for a project status of the workspace.
    kind: ItemKind,
    /// The category the mapping sends to the name.
    category: StatusCategory,
    /// The name, as the mapping spells it.
    name: &'a StatusName,
    /// Whether that kind's vocabulary has a name of that spelling, ignoring case.
    present: bool,
    /// The name's type in Linear — a `WorkflowState.type` for a task, a `ProjectStatusType`
    /// for a project — absent when it is missing.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    found_type: Option<&'a str>,
    /// The type `--apply` creates a missing name of this category and kind as; a present name
    /// of another type is reported and left as it is.
    expected_type: &'static str,
    /// Whether this run created the name; absent when it did not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    // Kept in the schema as `"default": false` although the JSON leaves `false` out, so both
    // SDKs model an absent `created` as `false` rather than as a member that must be there.
    #[schemars(!skip_serializing_if)]
    created: bool,
}

impl<'a> From<&'a MappedStatusName> for MappedStatusNameWire<'a> {
    fn from(mapped: &'a MappedStatusName) -> Self {
        let found_type = match &mapped.found {
            Found::Present(kind) | Found::Created(kind) => Some(kind.as_str()),
            Found::Missing => None,
        };
        Self {
            kind: mapped.kind,
            category: mapped.category,
            name: &mapped.name,
            present: found_type.is_some(),
            found_type,
            expected_type: mapped.expected_type(),
            created: mapped.created(),
        }
    }
}

impl serde::Serialize for MappedStatusName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        MappedStatusNameWire::from(self).serialize(serializer)
    }
}

impl schemars::JsonSchema for MappedStatusName {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        MappedStatusNameWire::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> Schema {
        MappedStatusNameWire::json_schema(generator)
    }
}

/// Report every name one `linear` source's `status_mapping` gives each kind, as present in
/// that kind's vocabulary with its type or missing from it — and, with `apply`, create every
/// missing one first.
///
/// # Errors
///
/// [`SourceError::Config`] or [`SourceError::Auth`] for a source that cannot be built, a
/// refusal for one with no `team`, and whatever else Linear could not answer. A create Linear
/// refuses is not an error here: it is [`StatusNamesReport::refused`], beside what was created.
pub async fn status_names(
    name: &SourceName,
    config: LinearConfig,
    secrets: &dyn SecretResolver,
    apply: bool,
) -> Result<StatusNamesReport, SourceError> {
    LinearSource::new(name, config, secrets)?
        .status_names(apply)
        .await
}

struct LinearSource {
    client: reqwest::Client,
    endpoint: Endpoint,
    key: SecretString,
    team: Option<Team>,
    /// This source's configured name, kept for one comparison: a far end recorded as
    /// `<this name>:<native>` is a Linear item Linear itself relates, so the reserved key
    /// is refused for it exactly as a bare id of the same kind is.
    name: SourceName,
    /// Which name each status category is, for a task and for a project.
    statuses: StatusMapping,
    /// The one Linear project this source is scoped to, when it is.
    project: Option<ProjectScope>,
    /// What a status write resolves, once it has been read; see [`Vocabulary`].
    vocabulary: std::sync::Mutex<Option<std::sync::Arc<Vocabulary>>>,
}

impl LinearSource {
    fn new(
        name: &SourceName,
        config: LinearConfig,
        secrets: &dyn SecretResolver,
    ) -> Result<Self, SourceError> {
        for kind in [ItemKind::Task, ItemKind::Project] {
            StatusMapping::distinct(
                name,
                kind,
                config
                    .status_mapping
                    .names(kind)
                    .map(|(category, mapped)| (category, mapped.as_str())),
            )?;
        }
        let key = secrets
            .get(&config.api_key_env.0)
            .filter(|v| !v.expose_secret().trim().is_empty())
            .ok_or_else(|| SourceError::Auth {
                message: format!("set environment variable {}", config.api_key_env.0),
            })?;
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint: config.endpoint,
            key,
            team: config.team,
            name: name.clone(),
            statuses: config.status_mapping,
            project: config.project,
            vocabulary: std::sync::Mutex::new(None),
        })
    }
}
#[derive(Clone, Copy)]
enum WriteKind {
    Task,
    Project,
}
/// What a whole write already knows of the relations its item holds.
enum HeldRelations<'a> {
    /// None: the item was just created.
    None,
    /// The first page of them, as the rewrite's own answer reported it.
    Page(&'a Value),
    /// Nothing yet, so they are read.
    Unread,
}
enum Lookup<'a> {
    IssueLabel(&'a str),
    ProjectLabel(&'a str),
}
impl Lookup<'_> {
    fn query(&self) -> &'static str {
        match self {
            Self::IssueLabel(_) => graphql::ISSUE_LABEL,
            Self::ProjectLabel(_) => graphql::PROJECT_LABEL,
        }
    }
    fn connection(&self) -> &'static str {
        match self {
            Self::IssueLabel(_) => "issueLabels",
            Self::ProjectLabel(_) => "projectLabels",
        }
    }
    fn diagnostic(&self) -> String {
        match self {
            Self::IssueLabel(name) | Self::ProjectLabel(name) => format!("label {name:?}"),
        }
    }
    fn variables(&self) -> Value {
        match self {
            Self::IssueLabel(name) | Self::ProjectLabel(name) => json!({"name":name}),
        }
    }
}
#[derive(Clone, Copy)]
enum MutationRoot {
    IssueCreate,
    IssueUpdate,
    ProjectCreate,
    ProjectUpdate,
    IssueRelationCreate,
    ProjectRelationCreate,
    IssueRelationDelete,
    ProjectRelationDelete,
    IssueDelete,
    ProjectDelete,
    DocumentCreate,
    DocumentUpdate,
    DocumentDelete,
    CommentCreate,
    CommentUpdate,
    CommentDelete,
    WorkflowStateCreate,
    ProjectStatusCreate,
}
impl MutationRoot {
    fn as_str(self) -> &'static str {
        match self {
            Self::IssueCreate => "issueCreate",
            Self::IssueUpdate => "issueUpdate",
            Self::ProjectCreate => "projectCreate",
            Self::ProjectUpdate => "projectUpdate",
            Self::IssueRelationCreate => "issueRelationCreate",
            Self::ProjectRelationCreate => "projectRelationCreate",
            Self::IssueRelationDelete => "issueRelationDelete",
            Self::ProjectRelationDelete => "projectRelationDelete",
            Self::IssueDelete => "issueDelete",
            Self::ProjectDelete => "projectDelete",
            Self::DocumentCreate => "documentCreate",
            Self::DocumentUpdate => "documentUpdate",
            Self::DocumentDelete => "documentDelete",
            Self::CommentCreate => "commentCreate",
            Self::CommentUpdate => "commentUpdate",
            Self::CommentDelete => "commentDelete",
            Self::WorkflowStateCreate => "workflowStateCreate",
            Self::ProjectStatusCreate => "projectStatusCreate",
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    // llmlint: ignore[invalid_states_unrepresentable] One transport envelope carries eight distinct GraphQL data shapes; each operation immediately validates its own complete mapper into typed plugin-api values, so malformed external data cannot cross the plugin boundary and a union here would duplicate every query response solely inside transport code.
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GqlError>,
}
#[derive(Deserialize)]
struct GqlError {
    message: String,
    // Held raw rather than typed, for two reasons. Linear puts the whole of *why* it
    // refused in here — `message` is a category name like `Argument Validation Error`,
    // which named neither the field nor the value when the live project-relation write
    // was refused by it — so a refusal carries this verbatim and a reader diagnoses from
    // it. And a typed shape with a required `code` fails the whole envelope's
    // deserialization when Linear sends extensions without one, turning a refusal this
    // source could explain into an unexplained malformed response.
    extensions: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlExtensions {
    code: GqlErrorCode,
    retry_after: Option<u64>,
}
impl GqlError {
    /// The rate-limit shape of [`Self::extensions`], when it has one.
    fn coded(&self) -> Option<GqlExtensions> {
        self.extensions
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
    /// Everything Linear said about this refusal, on one line and cut to [`SAID_LIMIT`].
    ///
    /// Linear's own sentence comes first, then the raw envelope, because only the first
    /// of those two is short enough to survive [`SAID_LIMIT`] on its merits. `message` is
    /// a category name — `Argument Validation Error` — and the sentence naming the field
    /// and the values it would have taken is `extensions.userPresentableMessage`, one of
    /// several keys in an envelope whose `validationErrors` echoes the whole rejected
    /// input back. Observed against the real API on 2026-09-04, a `projectRelationCreate`
    /// refusal rendered past the cut, and the echo is what got cut.
    ///
    /// That the sentence itself did not was luck: this build of `serde_json` renders an
    /// object's keys sorted, and `userPresentableMessage` happens to sort ahead of
    /// `validationErrors`. Nobody chose that — Linear sends the echo first — and any key
    /// Linear adds sorting between the two would move the sentence behind an echo longer
    /// than the whole limit, as would turning `preserve_order` on. Leading with it makes
    /// what a reader diagnoses from independent of both.
    fn said(&self) -> String {
        let Some(extensions) = &self.extensions else {
            return elided(&self.message);
        };
        match extensions
            .get("userPresentableMessage")
            .and_then(Value::as_str)
            .filter(|sentence| !sentence.is_empty())
        {
            Some(sentence) => elided(&format!("{}: {sentence} {extensions}", self.message)),
            None => elided(&format!("{}: {extensions}", self.message)),
        }
    }
}
#[derive(Deserialize)]
enum GqlErrorCode {
    #[serde(rename = "RATELIMITED", alias = "RATE_LIMITED")]
    RateLimited,
    #[serde(other)]
    Other,
}

/// One GraphQL error Linear answered a request with, before it is made a [`SourceError`].
struct Refusal(GqlError);

impl Refusal {
    /// Whether Linear refused because the issue the request addressed is not there.
    ///
    /// Linear answers a mutation addressing an id it does not hold with an errored response —
    /// `Entity not found: Issue`, whose `userPresentableMessage` reads `Could not find
    /// referenced Issue.` — rather than a null payload. Both spellings are recognised, so a
    /// rewording of either one alone still reads as what it is; an entity of any other kind,
    /// and anything else, is a refusal. The live capability journey sends this mutation
    /// to a nonexistent issue and requires `None`, so changed service wording fails the lane.
    fn entity_missing(&self) -> bool {
        let lowered = self.0.message.to_ascii_lowercase();
        // The Issue the mutation addressed, and nothing it merely refers to: a state the input
        // names that Linear does not hold is a refusal of the write, never no such task.
        lowered.trim_end() == graphql::ISSUE_NOT_FOUND_MESSAGE.to_ascii_lowercase()
            || lowered.starts_with(&format!(
                "{} ",
                graphql::ISSUE_NOT_FOUND_MESSAGE.to_ascii_lowercase()
            ))
            || self
                .0
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("userPresentableMessage"))
                .and_then(Value::as_str)
                .is_some_and(|said| {
                    said.to_ascii_lowercase().starts_with(
                        &graphql::ISSUE_NOT_FOUND_PRESENTABLE
                            .trim_end_matches('.')
                            .to_ascii_lowercase(),
                    )
                })
    }

    fn into_error(self) -> SourceError {
        SourceError::Refused {
            message: self.0.said(),
        }
    }
}

/// How much of a failed response's body a refusal carries.
///
/// Enough for Linear's own error envelope, which is one or two sentences naming the field
/// or argument it would not accept, and short enough that a proxy's HTML error page does
/// not become the whole message.
const SAID_LIMIT: usize = 400;

/// `said` made safe to put in a message: one line of printable text, cut to [`SAID_LIMIT`].
///
/// A failed response's body is whatever answered — Linear's error envelope, or an HTML
/// page from a proxy in front of it — and this message is written to a terminal. So every
/// control character goes, escape sequences with them, and each run of whitespace becomes
/// one space: a body cannot move the cursor, repaint the line or hide the rest of the
/// diagnostic behind itself. Cut by characters rather than bytes, because slicing UTF-8
/// mid-codepoint would panic inside the path that exists to explain a failure.
fn elided(said: &str) -> String {
    let mut printable = String::new();
    let mut spaced = true;
    for character in said.chars() {
        if character.is_control() || character.is_whitespace() {
            if !spaced {
                printable.push(' ');
                spaced = true;
            }
            continue;
        }
        printable.push(character);
        spaced = false;
    }
    let printable = printable.trim_end();
    if printable.chars().count() <= SAID_LIMIT {
        return printable.to_owned();
    }
    let kept: String = printable.chars().take(SAID_LIMIT).collect();
    format!("{kept}…")
}

impl LinearSource {
    async fn send(&self, query: &str, variables: Value) -> Result<Value, SourceError> {
        self.answer(query, variables)
            .await?
            .map_err(|refusal| refusal.into_error())
    }

    /// Send a mutation addressing one item by id, answering `None` when Linear refuses it
    /// because there is no such item.
    ///
    /// What lets a status write answer "no such task" without reading the issue first: Linear
    /// reports an id naming nothing as an errored response rather than a null payload, and
    /// [`Refusal::entity_missing`] is how that refusal is told apart from every other.
    async fn send_to_held(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Option<Value>, SourceError> {
        match self.answer(query, variables).await? {
            Ok(data) => Ok(Some(data)),
            Err(refusal) if refusal.entity_missing() => Ok(None),
            Err(refusal) => Err(refusal.into_error()),
        }
    }

    // llmlint: ignore[invalid_states_unrepresentable] This private generic transport accepts only variables constructed immediately at typed TaskSource call sites, never untrusted input; per-operation response mappers validate every external field before returning public values.
    async fn answer(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Result<Value, Refusal>, SourceError> {
        let response = self
            .client
            .post(&self.endpoint.0)
            .header("Authorization", self.key.expose_secret())
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await
            .map_err(|e| SourceError::Unavailable {
                message: e.to_string(),
            })?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        if status.as_u16() == 429 {
            return Err(SourceError::RateLimited {
                retry_after_seconds: retry,
                // Linear has one rate limiter and the status is the whole of what it said,
                // so there is nothing to add beyond the kind — which is what an absent
                // message means.
                message: None,
            });
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(SourceError::Auth {
                message: "Linear rejected the configured credential".into(),
            });
        }
        if !status.is_success() {
            // Linear puts its GraphQL error envelope in the *body* of a 400, so the status
            // alone names the whole call and nothing about what Linear objected to. The
            // body is Linear's answer to this request and holds no credential; it is cut
            // because a proxy in front of Linear can answer with a page.
            let text = response.text().await.unwrap_or_default();
            // An id naming nothing is the one refusal a caller acts on rather than reports,
            // so it reads the same whichever status Linear sends it under.
            if let Some(missing) = serde_json::from_str::<Envelope>(&text)
                .ok()
                .and_then(|body| body.errors.into_iter().next())
                .map(Refusal)
                .filter(Refusal::entity_missing)
            {
                return Ok(Err(missing));
            }
            let said = elided(&text);
            return Err(SourceError::Unavailable {
                message: if said.is_empty() {
                    format!("Linear returned HTTP {status}")
                } else {
                    format!("Linear returned HTTP {status}: {said}")
                },
            });
        }
        let body: Envelope = response.json().await.map_err(|e| SourceError::Malformed {
            message: e.to_string(),
        })?;
        if let Some(error) = body.errors.into_iter().next() {
            if let Some(extensions) = error
                .coded()
                .filter(|extensions| matches!(extensions.code, GqlErrorCode::RateLimited))
            {
                return Err(SourceError::RateLimited {
                    retry_after_seconds: extensions.retry_after.or(retry),
                    message: None,
                });
            }
            return Ok(Err(Refusal(error)));
        }
        body.data.map(Ok).ok_or_else(|| SourceError::Malformed {
            message: "GraphQL response has no data".into(),
        })
    }

    // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] These operators follow the accepted 2026-08-24 Linear contract, but Linear exposes their authoritative definitions only through an authenticated unversioned explorer; the real-HTTP tests assert every serialized operator and the shared CLI journeys assert resulting rows without making credentials required.
    /// The label predicates, which really are spelled the same at both levels.
    ///
    /// `IssueFilter.labels` is an `IssueLabelCollectionFilter` and `ProjectFilter.labels`
    /// is a `ProjectLabelCollectionFilter` — two types — but `some`, `every` and a `name`
    /// of `StringComparator` are members of both, so one spelling satisfies each. That is
    /// the whole of what the two filters have in common, and everything else about them is
    /// built separately for the reason recorded on the two builders below.
    ///
    /// "At least one of these" is a disjunction of `eqIgnoreCase` rather than one
    /// case-insensitive list operator, because Linear has no such operator. This source
    /// sent `labels:{some:{name:{inIgnoreCase:[…]}}}` until Linear refused it outright,
    /// HTTP 400, on the first read of the live lane that ever reached a label filter:
    ///
    /// ```text
    /// Variable "$filter" got invalid value { inIgnoreCase: […] } at
    /// "filter.and[1].labels.some.name"; Field "inIgnoreCase" is not defined by
    /// type "StringComparator". Did you mean "eqIgnoreCase" or "neqIgnoreCase"?
    /// ```
    ///
    /// That refusal is also the evidence for the replacement: Linear named the two members
    /// of `StringComparator` closest to what it was sent, and `eqIgnoreCase` is one of
    /// them — the same operator `all_of` below has always sent and the live lane has always
    /// exercised. `in` exists there too and would need no `or`, but it is case-sensitive,
    /// so `any_of` would stop agreeing with `all_of` and `none_of` and with what the table
    /// at the top of this file says this source does.
    fn label_parts(labels: &onetaskgraph_plugin_api::LabelFilter) -> Vec<Value> {
        let mut parts = Vec::new();
        if !labels.any_of.is_empty() {
            parts.push(json!({"or": labels
                .any_of
                .iter()
                .map(|name| json!({"labels": {"some": {"name": {"eqIgnoreCase": name}}}}))
                .collect::<Vec<_>>()}));
        }
        for name in &labels.all_of {
            parts.push(json!({"labels": {"some": {"name": {"eqIgnoreCase": name}}}}));
        }
        for name in &labels.none_of {
            parts.push(json!({"labels": {"every": {"name": {"neqIgnoreCase": name}}}}));
        }
        parts
    }
    fn narrowed(mut parts: Vec<Value>) -> Value {
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            json!({"and": parts})
        }
    }
    /// The filter this source sends to `issues(filter:)`.
    ///
    /// **`IssueFilter` and `ProjectFilter` are different input types, and one builder for
    /// both is what put two wrong fields on the wire.** They read as though they were the
    /// same filter over different rows — the label member really is spelled alike, and the
    /// `and`/`or` are identical — and a single builder producing one object for both
    /// connections had shipped `team` and the issue's `state` shape into `projects(filter:)`
    /// since long before this branch. Linear refused the first outright:
    ///
    /// ```text
    /// Variable "$filter" got invalid value { team: { key: [Object] } };
    /// Field "team" is not defined by type "ProjectFilter". Did you mean "lead"?
    /// ```
    ///
    /// So there are two builders, and each names its own type's members. Adding a predicate
    /// means deciding twice, on purpose, rather than once by accident.
    fn issue_filter(&self, query: &TaskQuery) -> Result<Value, SourceError> {
        let mut parts = self.issue_scope();
        parts.extend(Self::label_parts(&query.labels));
        if !query.statuses.is_empty() {
            parts.extend(self.status_narrowing(ItemKind::Task, &query.statuses));
        }
        match &query.project {
            ProjectFilter::Orphans => parts.push(json!({"project": {"null": true}})),
            ProjectFilter::Is(id) => parts.push(json!({"project": {"id": {"eq": id.0}}})),
            ProjectFilter::Any => {}
        }
        // The narrowings below are each confirmed in process by `confirms` before a row is
        // returned: Linear's comparators are a candidate set, and the contract's predicate is
        // what decides. None of them can drop a row the predicate keeps — see the module
        // documentation's ruling on the follow-up searches for what each one rests on.
        if !query.priorities.is_empty() {
            parts.push(json!({"priority": {"in": query
                .priorities
                .iter()
                .map(|priority| linear_priority(*priority))
                .collect::<Vec<_>>()}}));
        }
        if let Some(since) = query.commented_since {
            let since = since.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
            parts.push(json!({"comments": {"some": {"or": [
                {"createdAt": {"gte": since}},
                {"updatedAt": {"gte": since}},
            ]}}}));
        }
        for wanted in &query.metadata {
            parts.extend(slot_phrase(wanted.value()));
        }
        if let Some(origin) = &query.origin {
            parts.extend(slot_phrase(origin));
        }
        if let Some(text) = &query.text {
            let title = json!({"title": {"containsIgnoreCase": text.terms}});
            let content = json!({"description": {"containsIgnoreCase": text.terms}});
            parts.push(match text.fields {
                TextFields::Title => title,
                TextFields::Content => content,
                TextFields::TitleOrContent => json!({"or": [title, content]}),
            });
        }
        Ok(Self::narrowed(parts))
    }

    /// What every issue read of this source is narrowed to before any predicate: the
    /// configured team, and the project this source is scoped to, when it is.
    fn issue_scope(&self) -> Vec<Value> {
        let mut parts = Vec::new();
        if let Some(team) = &self.team {
            parts.push(json!({"team": {"key": {"eqIgnoreCase": team.0}}}));
        }
        if let Some(project) = &self.project {
            parts.push(json!({"project": {"id": {"eq": project.0}}}));
        }
        parts
    }

    /// The status narrowing for `statuses` over one kind's items, through this instance's
    /// mapping — `state` on an issue, `status` on a project, each compared by name.
    ///
    /// Each category asks for exactly what reads as it: the items at the name its kind maps it
    /// to, and — for `unknown` — every item at a name that kind's mapping does not name at
    /// all. A category the kind has no name for asks for nothing, because nothing reads as it.
    /// `None` is the one narrowing that is everything: `unknown` over a kind whose mapping
    /// names nothing, where every item reads as `unknown`.
    ///
    /// Compared ignoring case, as a read matches a name: `eqIgnoreCase` for a mapped name and
    /// `neqIgnoreCase` for every name an `unknown` excludes, because `in` and `nin` are the
    /// case-sensitive list operators and a state spelled `TODO` reads as a mapping's `Todo`.
    fn status_narrowing(&self, kind: ItemKind, statuses: &[StatusCategory]) -> Option<Value> {
        let member = match kind {
            ItemKind::Task => "state",
            ItemKind::Project => "status",
        };
        let named = |name: &str, operator: &str| json!({ (member): {"name": {(operator): name}}});
        let mut alternatives = Vec::new();
        for category in statuses {
            if let Ok(name) = self.statuses.name_for(*category, kind) {
                alternatives.push(named(name.as_str(), "eqIgnoreCase"));
            }
            if *category == StatusCategory::Unknown {
                let unmapped: Vec<Value> = self
                    .statuses
                    .names(kind)
                    .map(|(_, name)| named(name.as_str(), "neqIgnoreCase"))
                    .collect();
                if unmapped.is_empty() {
                    return None;
                }
                alternatives.push(Self::narrowed(unmapped));
            }
        }
        Some(match alternatives.len() {
            // Nothing asked for has a name of this kind: a name no item is at, which matches
            // nothing and is refused by nothing.
            0 => json!({ (member): {"name": {"in": Vec::<&str>::new()}}}),
            1 => alternatives.pop().expect("one alternative"),
            _ => json!({ "or": alternatives }),
        })
    }

    /// Whether one task this source read satisfies every predicate of `query` that a
    /// narrowing above only approximates — the metadata matches, the origin, the text and the
    /// priorities — by the contract's own statement of each.
    fn confirms(query: &TaskQuery, task: &Task) -> bool {
        (query.statuses.is_empty() || query.statuses.contains(&task.status.category))
            && query.metadata_matches(&task.metadata)
            && query.origin_matches(&task.metadata)
            && (query.priorities.is_empty() || query.priorities.contains(&task.priority))
            && query
                .text
                .as_ref()
                .is_none_or(|text| text_holds(&task.title, task.content.as_deref(), text))
    }

    /// The filter this source sends to `projects(filter:)`.
    ///
    /// Two members differ from [`Self::issue_filter`] and both are Linear's doing; see that
    /// builder for why they are written out twice rather than shared.
    ///
    /// **A project has no `team`.** It has the teams it is accessible from, and
    /// `ProjectFilter.accessibleTeams` is a `TeamCollectionFilter`, so the same team key
    /// reaches it under `some:`. `leadTeam` is the other team-shaped member and is a
    /// different set — one designated team rather than every team the project is in — so
    /// narrowing by it would drop projects the configured team really does hold.
    ///
    /// **A project's status is not an issue's state.** An issue's is a `WorkflowState` of the
    /// team, reached through `IssueFilter.state`; a project's is a `ProjectStatus` of the
    /// workspace, reached through `ProjectFilter.status` — `ProjectFilter.state` exists and is
    /// *not* it: that member is a bare `StringComparator` over a different thing. The two are
    /// different vocabularies of names, which is why `status_mapping` names each kind's
    /// separately, and why each is narrowed by its own kind's names
    /// ([`Self::status_narrowing`]).
    fn project_filter(
        &self,
        labels: &onetaskgraph_plugin_api::LabelFilter,
        statuses: &[StatusCategory],
    ) -> Value {
        let mut parts = self.project_scope();
        parts.extend(Self::label_parts(labels));
        if !statuses.is_empty() {
            parts.extend(self.status_narrowing(ItemKind::Project, statuses));
        }
        Self::narrowed(parts)
    }

    /// What every project read of this source is narrowed to: the projects the configured
    /// team can reach, and the one project this source is scoped to, when it is.
    fn project_scope(&self) -> Vec<Value> {
        let mut parts = Vec::new();
        if let Some(team) = &self.team {
            parts.push(json!({"accessibleTeams": {"some": {"key": {"eqIgnoreCase": team.0}}}}));
        }
        if let Some(project) = &self.project {
            parts.push(json!({"id": {"eq": project.0}}));
        }
        parts
    }

    /// Whether an item filed under `project` is one this source holds: always, unless it is
    /// scoped to one project and this is not filed under it.
    fn in_scope(&self, project: Option<&NativeId>) -> bool {
        self.project
            .as_ref()
            .is_none_or(|scope| project.is_some_and(|project| project.0 == scope.0))
    }

    /// The project a task or a document written with `project` is filed under: that one, or
    /// the scope when it names none — and a refusal naming both when it names another.
    fn filed_in(
        &self,
        project: Option<&NativeId>,
        what: &str,
    ) -> Result<Option<String>, SourceError> {
        match (&self.project, project) {
            (None, project) => Ok(project.map(|id| id.0.clone())),
            (Some(scope), None) => Ok(Some(scope.0.clone())),
            (Some(scope), Some(project)) if project.0 == scope.0 => Ok(Some(scope.0.clone())),
            (Some(scope), Some(project)) => Err(SourceError::Refused {
                message: format!(
                    "source {} is scoped to the Linear project {} and cannot hold a {what} in                      the project {}; next: write it with no project, or with {}, or to a                      source scoped to {}",
                    self.name, scope.0, project.0, scope.0, project.0
                ),
            }),
        }
    }
    // llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

    /// The refusal of a project write to `target` — `None` for a new project — when this
    /// source is scoped to another: it holds that project and no other, so it writes no other,
    /// because a project it created would be one none of its reads could find. Asked before
    /// any request is sent.
    fn project_out_of_scope(&self, target: Option<&NativeId>) -> Option<SourceError> {
        let scope = self.project.as_ref()?;
        if target.is_some_and(|target| target.0 == scope.0) {
            return None;
        }
        Some(SourceError::Refused {
            message: format!(
                "source {} is scoped to the Linear project {} and holds no other project, so it \
                 cannot write {}; next: copy the project to a source with no `project`, or copy \
                 its tasks here",
                self.name,
                scope.0,
                target.map_or_else(
                    || "a new one".to_owned(),
                    |target| format!("the project {}", target.0)
                ),
            ),
        })
    }

    async fn one_id(&self, lookup: Lookup<'_>) -> Result<NativeId, SourceError> {
        let data = self.send(lookup.query(), lookup.variables()).await?;
        let connection = lookup.connection();
        let nodes = data
            .get(connection)
            .and_then(|v| v.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {connection}.nodes"),
            })?;
        match nodes.as_slice() {
            [] => Err(SourceError::Refused {
                message: format!(
                    "source {} cannot resolve {}: found 0 matches",
                    self.name,
                    lookup.diagnostic()
                ),
            }),
            [node] => Ok(NativeId(backend_id(node, "id")?.to_owned())),
            nodes => {
                let ids = nodes
                    .iter()
                    .map(|node| backend_id(node, "id"))
                    .collect::<Result<Vec<_>, _>>()?;
                Err(SourceError::Refused {
                    message: format!(
                        "source {} cannot resolve {}: found {} matches with ids {ids:?}",
                        self.name,
                        lookup.diagnostic(),
                        nodes.len()
                    ),
                })
            }
        }
    }

    /// What a status write resolves, as this instance holds it — read in one request the first
    /// time anything needs it, or again when `fresh` asks — and whether this call read it.
    ///
    /// Only an answer that read completely is held, so a failed or malformed read leaves the
    /// next call reading again rather than resolving against nothing.
    async fn vocabulary(
        &self,
        fresh: bool,
    ) -> Result<(std::sync::Arc<Vocabulary>, bool), SourceError> {
        if !fresh && let Some(held) = self.held_vocabulary() {
            return Ok((held, false));
        }
        let team = self.team.as_ref().ok_or_else(|| SourceError::Refused {
            message: format!(
                "source {} needs config.team before it can write a Linear item or a status",
                self.name
            ),
        })?;
        let data = self
            .send(graphql::RESOLUTION, json!({"key": team.0}))
            .await?;
        let read = std::sync::Arc::new(Vocabulary::read(&data, &self.name, &team.0)?);
        *self
            .vocabulary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(read.clone());
        Ok((read, true))
    }

    fn held_vocabulary(&self) -> Option<std::sync::Arc<Vocabulary>> {
        self.vocabulary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Drop what this instance holds, after a write carrying one of its ids failed: the id may
    /// be what Linear refused, and the next write reads afresh rather than sending it again.
    fn forget_vocabulary(&self) {
        *self
            .vocabulary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// Hold one name `sources fields --apply` created beside the ones read, so the writes after
    /// it resolve it without another read.
    fn remember(&self, created: &Created) {
        let mut guard = self
            .vocabulary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(vocabulary) = guard.as_mut() {
            std::sync::Arc::make_mut(vocabulary).add(created.clone());
        }
    }

    async fn team_id(&self) -> Result<NativeId, SourceError> {
        Ok(self.vocabulary(false).await?.0.team.clone())
    }

    /// The id and the name Linear holds of the name `category` is for `kind` — refused before
    /// any request when the mapping gives that kind no name for it, and refused naming the name
    /// when the kind's vocabulary lacks it even after one fresh read.
    async fn status_id(
        &self,
        category: StatusCategory,
        kind: ItemKind,
    ) -> Result<(NativeId, String), SourceError> {
        let name = self
            .statuses
            .name_for(category, kind)
            .map_err(|why| why.refusal(&self.name, category, kind))?;
        let (vocabulary, fresh) = self.vocabulary(false).await?;
        if let Some(held) = vocabulary.find(kind, name.as_str(), &self.name)? {
            return Ok((held.id.clone(), held.name.as_str().to_owned()));
        }
        if !fresh {
            let (vocabulary, _) = self.vocabulary(true).await?;
            if let Some(held) = vocabulary.find(kind, name.as_str(), &self.name)? {
                return Ok((held.id.clone(), held.name.as_str().to_owned()));
            }
        }
        let category_key = category_word(category);
        let kind_key = kind_word(kind);
        let held_by = match kind {
            ItemKind::Task => format!(
                "team {}",
                self.team.as_ref().map_or("(none)", |team| team.0.as_str())
            ),
            ItemKind::Project => "this workspace".to_owned(),
        };
        Err(SourceError::Refused {
            message: format!(
                "source {} maps the {kind_key} status {category_key} to the {} {:?}, which {held_by} \
                 does not have; next: run `onetaskgraph sources fields {} --apply` to create it, \
                 or point status_mapping.{category_key}.{kind_key} of this source at a {} \
                 {held_by} has",
                self.name,
                vocabulary_word(kind),
                name.as_str(),
                self.name,
                vocabulary_word(kind),
            ),
        })
    }

    async fn label_ids(
        &self,
        labels: &[Label],
        kind: WriteKind,
    ) -> Result<Vec<NativeId>, SourceError> {
        let mut ids = Vec::with_capacity(labels.len());
        for label in labels {
            ids.push(
                self.one_id(if matches!(kind, WriteKind::Project) {
                    Lookup::ProjectLabel(&label.name)
                } else {
                    Lookup::IssueLabel(&label.name)
                })
                .await?,
            );
        }
        Ok(ids)
    }
    fn write_description(
        &self,
        content: Option<&str>,
        metadata: &std::collections::BTreeMap<String, Value>,
        repositories: &[Repository],
        edges: &[DependencyEdge],
        kind: WriteKind,
    ) -> Result<Option<String>, SourceError> {
        Self::long_form(
            content,
            metadata,
            repositories,
            self.recorded_ends(edges, kind),
        )
    }

    /// The far ends of `edges` no relation of this workspace can name — another level, or
    /// another source — as the reserved key records them.
    fn recorded_ends(&self, edges: &[DependencyEdge], kind: WriteKind) -> Vec<Value> {
        edges
            .iter()
            .filter(|edge| {
                edge.to.kind
                    != match kind {
                        WriteKind::Task => ItemKind::Task,
                        WriteKind::Project => ItemKind::Project,
                    }
                    || edge
                        .to
                        .id()
                        .split_once(':')
                        .is_some_and(|(source, _)| source != self.name.as_str())
            })
            .map(|edge| json!({"id":edge.to.id(),"kind":edge.to.kind}))
            .collect()
    }

    /// Every forward edge `id` holds, relations and recorded far ends alike, walked to
    /// exhaustion.
    async fn forward_edges(&self, id: &NativeId) -> Result<Vec<DependencyEdge>, SourceError> {
        let mut edges = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .dependencies(
                    ISSUE_RELATIONS,
                    DependencyRoot::Issue,
                    id,
                    Direction::DependsOn,
                    &PageRequest {
                        cursor,
                        limit: MAX_PAGE_SIZE,
                    },
                )
                .await?;
            edges.extend(page.items);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(edges),
            }
        }
    }

    /// Apply one targeted update to one issue; see [`TaskSource::update_task`].
    ///
    /// **An update naming a status and nothing else reads nothing first:** it is one
    /// `issueUpdate` carrying the `stateId` its category's task name resolves to, whose own
    /// selection answers the task — see [`Self::status_written`]. Writing a category the issue
    /// already reads as therefore sends the same state again, and an issue at a name the
    /// mapping does not name, which reads as `unknown`, moves to the mapped `unknown` name.
    ///
    /// **Any other update reads the issue once**, then sends one `issueUpdate` carrying only
    /// the members that differ from it — `title`, `description` (content and metadata slot
    /// together), `stateId`, `priority` — and, when the named edges differ from the ones the
    /// issue holds, its relations replaced. The read is what the description is merged from:
    /// Linear has no conditional update, so writing the metadata slot without it would
    /// overwrite whatever a person wrote there since. Nothing is sent for a field already
    /// holding the requested value, and nothing at all when nothing differs. `delivers` lands
    /// in the metadata slot under its reserved key, as a copy writes it.
    ///
    /// Either way the task answered is the one the mutation's own selection reports, so its
    /// status is Linear's, and nothing is read after the write.
    async fn targeted_update(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        update.consistent()?;
        if let Some(delivers) = &update.delivers {
            TaskRef::listed(
                TaskRef::DELIVERS_KEY,
                id,
                Some(&self.name),
                delivers.clone(),
            )
            .map_err(|message| SourceError::Refused { message })?;
        }
        // Before any request: a category the mapping gives a task no name for is not one
        // Linear could answer differently for another issue.
        if let Some(status) = &update.status {
            self.statuses
                .name_for(status.category, ItemKind::Task)
                .map_err(|why| why.refusal(&self.name, status.category, ItemKind::Task))?;
        }
        if let Some(status) = update.status.as_ref().filter(|_| {
            TaskUpdate {
                status: None,
                ..update.clone()
            }
            .is_empty()
        }) {
            let Some(task) = self.status_written(id, status.category).await? else {
                return Ok(None);
            };
            return Ok(Some(TaskUpdateOutcome {
                delivers_before: task.delivers.clone(),
                written: std::iter::once(UpdatedField::Status).collect(),
                task,
            }));
        }
        let Some((before, description)) = self.issue_held(id).await? else {
            return Ok(None);
        };
        let (visible, held) = metadata_description(description)?;
        let mut slot = held.clone();
        for (key, value) in &update.metadata_set {
            slot.insert(key.as_str().to_owned(), value.clone());
        }
        for key in &update.metadata_remove {
            slot.remove(key.as_str());
        }
        if let Some(delivers) = &update.delivers {
            set_task_list(&mut slot, TaskRef::DELIVERS_KEY, delivers);
        }
        let mut relations = None;
        if let Some(wanted) = &update.depends_on {
            let current = self.forward_edges(&before.id).await?;
            let ends = |edges: &[DependencyEdge]| {
                let mut ends: Vec<(String, String)> = edges
                    .iter()
                    .map(|edge| {
                        (
                            edge.to.id().to_owned(),
                            format!("{:?}{:?}", edge.to.kind, edge.kind),
                        )
                    })
                    .collect();
                ends.sort();
                ends
            };
            if ends(&current) != ends(wanted) {
                let prepared = self.prepare_edges(wanted, WriteKind::Task).await?;
                let recorded = self.recorded_ends(&prepared, WriteKind::Task);
                if recorded.is_empty() {
                    slot.remove(DependencyEdge::RECORDED_KEY);
                } else {
                    slot.insert(DependencyEdge::RECORDED_KEY.into(), Value::Array(recorded));
                }
                relations = Some(prepared);
            }
        }
        let mut input = serde_json::Map::new();
        if let Some(title) = update
            .title
            .as_ref()
            .filter(|title| **title != before.title)
        {
            input.insert("title".into(), json!(title));
        }
        let content = update.content.as_deref().or(visible.as_deref());
        if content != visible.as_deref() || slot != held {
            let written = Self::described(content, &slot)?;
            // Checked before anything is sent: content ending in what this source reads as
            // its own slot would read back as metadata rather than as the content it was.
            let (reads, read) = metadata_description(written.clone())?;
            if reads.as_deref().unwrap_or_default() != content.unwrap_or_default() || read != slot {
                return Err(SourceError::Refused {
                    message: format!(
                        "this content would read back from source {} as something other than \
                         itself, or ends in what it reads as its own metadata slot; next: change \
                         how the content ends",
                        self.name
                    ),
                });
            }
            input.insert("description".into(), json!(written));
        }
        if let Some(status) = update
            .status
            .as_ref()
            .filter(|status| status.category != before.status.category)
        {
            let (state, _) = self.status_id(status.category, ItemKind::Task).await?;
            input.insert("stateId".into(), json!(state.0));
        }
        if let Some(priority) = update
            .priority
            .filter(|priority| *priority != before.priority)
        {
            input.insert("priority".into(), json!(linear_priority(priority)));
        }
        let carries_state = input.contains_key("stateId");
        let task = if input.is_empty() {
            before.clone()
        } else {
            let answered = self
                .send(
                    graphql::ISSUE_UPDATE_READ,
                    json!({"id":before.id.0,"input":Value::Object(input)}),
                )
                .await
                .and_then(|data| self.updated_issue(&data, &before.id));
            match answered {
                Ok(Some(task)) => task,
                Ok(None) => {
                    return Err(SourceError::Malformed {
                        message: format!("task {id} was updated and then answered as no issue"),
                    });
                }
                Err(error) => {
                    if carries_state {
                        self.forget_vocabulary();
                    }
                    return Err(error);
                }
            }
        };
        if let Some(prepared) = &relations {
            self.write_relations(&before.id, prepared, WriteKind::Task, HeldRelations::Unread)
                .await?;
        }
        let mut written = update.changed(&before, &task);
        if relations.is_some() {
            written.insert(UpdatedField::DependsOn);
        }
        Ok(Some(TaskUpdateOutcome {
            task,
            written,
            delivers_before: before.delivers,
        }))
    }

    /// Set one issue's state to the one `category`'s task name resolves to, in one
    /// `issueUpdate` whose selection answers the task as Linear now holds it — or `None` when
    /// Linear holds no such issue, which it says by refusing the mutation as naming nothing.
    ///
    /// No read before it, so this is the whole of a status write's cost once the resolution is
    /// held: one request — for a source scoped to one project as well. A status write goes to
    /// the issue it names wherever that issue is filed, because the item is named outright and
    /// the scope governs what this source reads, lists and creates, not where a status it is
    /// asked to set may land.
    async fn status_written(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Task>, SourceError> {
        let (state, _) = self.status_id(category, ItemKind::Task).await?;
        let target = id.clone();
        // `stateId` alone, so nothing else about the issue can move: Linear's
        // `IssueUpdateInput` makes every member optional and leaves an absent one as it was.
        let answered = self
            .send_to_held(
                graphql::ISSUE_UPDATE_READ,
                json!({"id":target.0,"input":{"stateId":state.0}}),
            )
            .await;
        match answered {
            Ok(Some(data)) => self.updated_issue(&data, &target),
            Ok(None) => Ok(None),
            Err(error) => {
                self.forget_vocabulary();
                Err(error)
            }
        }
    }

    /// The task an `issueUpdate` sent as [`graphql::ISSUE_UPDATE_READ`] answered with — `None`
    /// for an issue in the trash, on the terms a read by id answers — refusing an answer about
    /// another issue than the one asked for.
    ///
    /// Not narrowed to this source's scope: the issue was named outright and has been written,
    /// so it is answered wherever it is filed.
    ///
    /// `asked` may be the backend id or the identifier (`ENG-1`), because Linear takes either.
    fn updated_issue(&self, data: &Value, asked: &NativeId) -> Result<Option<Task>, SourceError> {
        let issue = mutation_payload(data, MutationRoot::IssueUpdate)?
            .get("issue")
            .filter(|issue| !issue.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing issueUpdate.issue".into(),
            })?;
        if optional_str(issue, "identifier")? != Some(asked.0.as_str()) {
            written_is(issue, asked)?;
        }
        optional(&json!({ "issue": issue }), "issue", |v| {
            map_task(v, &self.name, &self.statuses)
        })
    }

    /// The one long-form field a Linear item has, with this source's own slot at the end.
    ///
    /// Shared by every kind this source writes rather than reimplemented per kind: a
    /// document keeps caller metadata in exactly the slot an issue and a project do, which
    /// is what lets the same read side take it back out.
    fn long_form(
        content: Option<&str>,
        metadata: &std::collections::BTreeMap<String, Value>,
        repositories: &[Repository],
        recorded: Vec<Value>,
    ) -> Result<Option<String>, SourceError> {
        let mut metadata = metadata.clone();
        if repositories.is_empty() {
            metadata.remove(Repository::METADATA_KEY);
        } else {
            metadata.insert(Repository::METADATA_KEY.into(), json!(repositories));
        }
        if recorded.is_empty() {
            metadata.remove(DependencyEdge::RECORDED_KEY);
        } else {
            metadata.insert(DependencyEdge::RECORDED_KEY.into(), Value::Array(recorded));
        }
        Self::described(content, &metadata)
    }

    /// The long-form field holding `content` and a slot of exactly `metadata`, in the one
    /// encoding [`long_form`](Self::long_form) writes: the content alone when there is no
    /// metadata, and otherwise the slot after one blank line.
    fn described(
        content: Option<&str>,
        metadata: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Option<String>, SourceError> {
        let visible = content.unwrap_or_default();
        if metadata.is_empty() {
            return Ok((!visible.is_empty()).then(|| visible.to_owned()));
        }
        let slot = slot_text(metadata)?;
        Ok(Some(if visible.is_empty() {
            slot
        } else {
            format!("{visible}\n\n{slot}")
        }))
    }
    /// What this source says when asked for a project edge carrying no ordering.
    ///
    /// Linear's project relations have exactly one type and it is an ordering. Asked on
    /// 2026-09-04 to create one typed `related` — and separately `blocks` and `dependsOn`
    /// — the real API refused each with `Argument Validation Error` and
    /// `constraints: {"isEnum": "type must be one of the following values: dependency"}`.
    /// That is Linear's own enumeration of the field, from the validator behind GraphQL
    /// where introspection cannot reach it, and it has one member. An issue relation is a
    /// different relation with a different set, which does include `related`, so this
    /// reaches projects alone.
    fn unordered_project_relation(&self, near: &NativeId, far: &str) -> SourceError {
        SourceError::Refused {
            message: format!(
                "source {} cannot carry an unordered dependency between projects, because \
                 Linear types every project relation `dependency` and that is an ordering; \
                 record {near} to {far} as a dependency, or between tasks",
                self.name,
                near = near.0,
            ),
        }
    }
    /// The one edge [`Self::unordered_project_relation`] refuses, if there is one here.
    fn unordered_project_edge(edges: &[DependencyEdge]) -> Option<&DependencyEdge> {
        edges
            .iter()
            .find(|edge| edge.to.kind == ItemKind::Project && edge.kind == DependencyKind::Related)
    }
    /// Replace every relation `near` holds of `kind` with exactly `edges`: the ones it holds
    /// are deleted, then each edge is created.
    ///
    /// `held` is what is already known of the ones it holds: none, for an item the write this
    /// follows created; the first page of them, for one a rewrite's own answer reported; and
    /// otherwise nothing, so they are read. Only a page that says there are more is followed.
    async fn write_relations(
        &self,
        near: &NativeId,
        edges: &[DependencyEdge],
        kind: WriteKind,
        held: HeldRelations<'_>,
    ) -> Result<(), SourceError> {
        let mut cursor: Option<Cursor> = None;
        let mut answered = match held {
            HeldRelations::None => None,
            HeldRelations::Page(page) => Some(page.clone()),
            HeldRelations::Unread => Some(Value::Null),
        };
        while let Some(page) = answered.take() {
            let relations = if page.is_null() {
                let data = self
                    .send(
                        if matches!(kind, WriteKind::Project) {
                            PROJECT_RELATIONS
                        } else {
                            ISSUE_RELATIONS
                        },
                        json!({"id":near.0,"first":MAX_PAGE_SIZE,"after":cursor.as_ref().map(|cursor|&cursor.0)}),
                    )
                    .await?;
                let root = data
                    .get(if matches!(kind, WriteKind::Project) {
                        "project"
                    } else {
                        "issue"
                    })
                    .ok_or_else(|| SourceError::Malformed {
                        message: "missing relation item".into(),
                    })?;
                root.get("relations")
                    .cloned()
                    .ok_or_else(|| SourceError::Malformed {
                        message: "missing relations".into(),
                    })?
            } else {
                page
            };
            for relation in relations
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relations.nodes".into(),
                })?
            {
                let id = backend_id(relation, "id")?;
                let (query, mutation) = if matches!(kind, WriteKind::Project) {
                    (
                        graphql::PROJECT_RELATION_DELETE,
                        MutationRoot::ProjectRelationDelete,
                    )
                } else {
                    (
                        graphql::ISSUE_RELATION_DELETE,
                        MutationRoot::IssueRelationDelete,
                    )
                };
                let deleted = self.send(query, json!({"id":id})).await?;
                mutation_payload(&deleted, mutation)?;
            }
            if let Some(next) = page_next(&relations)? {
                cursor = Some(next);
                answered = Some(Value::Null);
            }
        }
        // Linear requires an anchor at each end of a project relation and validates both
        // against an enum GraphQL cannot see: `ProjectRelationCreateInput` declares them
        // `String!` and enumerates nothing, and the field descriptions read as a choice
        // between the project and a milestone, which is not what they are. Linear's own
        // refusal enumerates them — sent `project` in both, it answered `anchorType must
        // be one of the following values: start, end, milestone` — and `milestone` needs
        // an id this source never sends, so the two whole-project anchors are the whole of
        // what it can send.
        //
        // **Which of them goes where carries the direction, and the two id slots do not.**
        // Linear stores whatever pair it is given and reads a backwards dependency as
        // readily as the right one, so acceptance settles nothing; what does is Linear's
        // own reading of a stored relation, published as the computed `ProjectFilter`
        // members `hasBlockingRelations` ("projects which are blocking") and
        // `hasBlockedByRelations` ("projects which are blocked"). Three relations between
        // two scratch projects, read back through them on 2026-09-04:
        //
        // | `projectId` | `anchorType` | `relatedProjectId` | `relatedAnchorType` | blocked | blocking |
        // | ----------- | ------------ | ------------------ | ------------------- | ------- | -------- |
        // | A           | `start`      | B                  | `end`               | A       | B        |
        // | A           | `end`        | B                  | `start`             | B       | A        |
        // | B           | `end`        | A                  | `start`             | A       | B        |
        //
        // Rows one and three exchange the ids and the anchors together and read alike;
        // rows one and two exchange only the anchors and the reading flips. So the project
        // anchored `start` is the one that waits, whichever slot it sits in, and row one is
        // what this source sends — `near`, the item that depends, in `projectId`. Linear's
        // own callers put the blocker there instead, so copying their `end`/`start` pair
        // across by position would state every dependency backwards in the workspace, and
        // nothing would refuse it.
        const NEAR_ANCHOR: &str = "start";
        const FAR_ANCHOR: &str = "end";
        for edge in edges {
            if edge.to.kind
                != match kind {
                    WriteKind::Task => ItemKind::Task,
                    WriteKind::Project => ItemKind::Project,
                }
            {
                continue;
            }
            let far = match edge.to.id().split_once(':') {
                Some((source, native)) if source == self.name.as_str() => native,
                Some(_) => continue,
                None => edge.to.id(),
            };
            // A project relation is not spelled the way an issue relation is, and this is
            // the whole of what a project's `type` may say.
            //
            // `blocks` there is what the live journey's project write was refused for
            // once the two anchors above stopped being missing: Linear answered HTTP 200
            // with `Argument Validation Error`, the message class its input validator
            // raises for a value outside an accepted set, having already accepted every
            // field of the same input by name — which is what tells that refusal apart
            // from the missing-field one before it, and what says the anchors were not the
            // cause.
            //
            // Which field, and what it takes, was measured against the real API on
            // 2026-09-04 rather than inferred. Each of `blocks`, `dependsOn`, `related`
            // and `DEPENDENCY` was refused with `property: "type"` and
            // `constraints: {"isEnum": "type must be one of the following values:
            // dependency"}`; `dependency` was accepted. That enumeration, like the
            // anchors' above, reaches this source through the validator's `extensions`;
            // see `GqlError::said`.
            //
            // A `Related` project edge is refused at the top of this function by that same
            // enumeration: it has one member and it is an ordering. An issue relation is a
            // different relation with a different set, which does include `related`.
            let relation_type = match (kind, edge.kind) {
                (WriteKind::Project, DependencyKind::Blocks) => "dependency",
                (WriteKind::Task, DependencyKind::Blocks) => "blocks",
                (WriteKind::Task, DependencyKind::Related) => "related",
                // Unreachable past `write_project`'s guard, and an error rather than a
                // skip so it stays that way: an edge dropped here would be a copy
                // reporting success for a dependency the destination does not hold.
                (WriteKind::Project, DependencyKind::Related) => {
                    return Err(self.unordered_project_relation(near, edge.to.id()));
                }
            };
            let (query, input) = if matches!(kind, WriteKind::Project) {
                (
                    graphql::PROJECT_RELATION_CREATE,
                    json!({"projectId":near.0,"relatedProjectId":far,"type":relation_type,"anchorType":NEAR_ANCHOR,"relatedAnchorType":FAR_ANCHOR}),
                )
            } else {
                (
                    graphql::ISSUE_RELATION_CREATE,
                    json!({"issueId":near.0,"relatedIssueId":far,"type":relation_type}),
                )
            };
            let data = self.send(query, json!({"input":input})).await?;
            let mutation = if matches!(kind, WriteKind::Project) {
                MutationRoot::ProjectRelationCreate
            } else {
                MutationRoot::IssueRelationCreate
            };
            let payload = mutation_payload(&data, mutation)?;
            let relation = payload
                .get(if matches!(kind, WriteKind::Project) {
                    "projectRelation"
                } else {
                    "issueRelation"
                })
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {} relation", mutation.as_str()),
                })?;
            backend_id(relation, "id")?;
        }
        Ok(())
    }

    async fn prepare_edges(
        &self,
        edges: &[DependencyEdge],
        kind: WriteKind,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        let mut prepared = Vec::with_capacity(edges.len());
        for edge in edges {
            let mut edge = edge.clone();
            if edge.to.kind
                == match kind {
                    WriteKind::Task => ItemKind::Task,
                    WriteKind::Project => ItemKind::Project,
                }
                && edge
                    .to
                    .id()
                    .split_once(':')
                    .is_some_and(|(source, _)| source != self.name.as_str())
            {
                // Narrowed to what this source holds — its team, and its project when it is
                // scoped to one — and, for an issue, to the ones whose description carries
                // the far end as its origin: a copy of the far end is an item of this source,
                // and an item of another team or project carrying the same origin is not one
                // this source could relate. The origin is confirmed over the parsed slot below.
                let filter = match kind {
                    WriteKind::Project => Self::narrowed(self.project_scope()),
                    WriteKind::Task => {
                        let mut parts = self.issue_scope();
                        parts.extend(slot_phrase(edge.to.id()));
                        Self::narrowed(parts)
                    }
                };
                let mut cursor: Option<Cursor> = None;
                loop {
                    let data = self.send(if matches!(kind, WriteKind::Project) { PROJECTS } else { ISSUES }, json!({"first":MAX_PAGE_SIZE,"after":cursor.as_ref().map(|cursor|&cursor.0),"filter":filter})).await?;
                    let (items, next) = if matches!(kind, WriteKind::Project) {
                        let page =
                            connection(&data, "projects", |v| map_project(v, &self.statuses))?;
                        (
                            page.items
                                .into_iter()
                                .map(|item| (item.id, item.metadata))
                                .collect::<Vec<_>>(),
                            page.next,
                        )
                    } else {
                        let page = connection(&data, "issues", |v| {
                            map_task(v, &self.name, &self.statuses)
                        })?;
                        (
                            page.items
                                .into_iter()
                                .map(|item| (item.id, item.metadata))
                                .collect::<Vec<_>>(),
                            page.next,
                        )
                    };
                    if let Some((id, _)) = items.into_iter().find(|(_, metadata)| {
                        metadata.get("onetaskgraph.origin").and_then(Value::as_str)
                            == Some(edge.to.id())
                    }) {
                        edge.to = DependencyEndpoint::from_native(id, edge.to.kind);
                        break;
                    }
                    let Some(next) = next else { break };
                    cursor = Some(next);
                }
            }
            prepared.push(edge);
        }
        Ok(prepared)
    }
}

#[async_trait::async_trait]
impl TaskSource for LinearSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Native,
            comments: Support::Native,
            priority: Support::Native,
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
            max_page_size: MAX_PAGE_SIZE,
        }
    }
    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }
    async fn health(&self) -> Result<Health, SourceError> {
        let data = self.send(VIEWER, json!({})).await?;
        str_at(
            data.get("viewer").ok_or_else(|| SourceError::Malformed {
                message: "missing viewer".into(),
            })?,
            "id",
        )?;
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self.issue_held(id).await?.map(|(task, _)| task))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self.project_held(id).await?.map(|(project, _)| project))
    }
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let d=self.send(ISSUES,json!({"first":page.limit.min(MAX_PAGE_SIZE),"after":page.cursor.as_ref().map(|c|&c.0),"filter":self.issue_filter(query)?})).await?;
        let page = connection(&d, "issues", |v| map_task(v, &self.name, &self.statuses))?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .filter(|task| Self::confirms(query, task))
                .collect(),
            next: page.next,
        })
    }
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        // llmlint: ignore[changed_behavior_has_e2e] The shared CLI journey `every_complete_dataset_source_filters_projects_by_label_status_and_text` asserts that Linear status filtering returns only P-2 and reports native pushdown; `item_reads_and_transport_error_boundaries_are_exercised` separately asserts the serialized `status:{name:{eqIgnoreCase:…}}` predicate the mapping names.
        let d=self.send(PROJECTS,json!({"first":page.limit.min(MAX_PAGE_SIZE),"after":page.cursor.as_ref().map(|c|&c.0),"filter":self.project_filter(&query.labels,&query.statuses)})).await?;
        let page = connection(&d, "projects", |v| map_project(v, &self.statuses))?;
        // A project's text is applied here, over the page Linear answered: `search_title` and
        // `search_content` are declared for every level, and `ProjectFilter` is not asked for
        // a description match — so the rule decides, over every row of the page.
        Ok(Page {
            items: page
                .items
                .into_iter()
                .filter(|project| {
                    (query.statuses.is_empty() || query.statuses.contains(&project.status.category))
                        && query.text.as_ref().is_none_or(|text| {
                            text_holds(&project.title, project.content.as_deref(), text)
                        })
                })
                .collect(),
            next: page.next,
        })
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        let d = self
            .send(
                LABELS,
                json!({"first":page.limit.min(MAX_PAGE_SIZE),"after":page.cursor.as_ref().map(|c|&c.0)}),
            )
            .await?;
        connection(&d, "issueLabels", map_label)
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(ISSUE_RELATIONS, DependencyRoot::Issue, id, direction, page)
            .await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(
            PROJECT_RELATIONS,
            DependencyRoot::Project,
            id,
            direction,
            page,
        )
        .await
    }
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        // Before anything is read or written, because nothing Linear could answer changes
        // either: neither list may name the task itself or name one task twice, a category
        // this source has disabled is refused in the words a status write is refused with,
        // and a project other than the one this source is scoped to is refused naming both.
        let near = write.target.as_ref().unwrap_or(&write.item.id);
        for (key, entries) in [
            (TaskRef::DELIVERS_KEY, &write.item.delivers),
            (TaskRef::DELIVERED_BY_KEY, &write.item.delivered_by),
        ] {
            TaskRef::listed(key, near, Some(&self.name), entries.clone())
                .map_err(|message| SourceError::Refused { message })?;
        }
        let project = self.filed_in(write.item.project.as_ref(), "task")?;
        // The status is resolved before anything else is sent: a category the mapping gives a
        // task no name for is refused before any request, and a name the team lacks before any
        // mutation. The mapping decides alone; the status's own name plays no part.
        let (state, _) = self
            .status_id(write.item.status.category, ItemKind::Task)
            .await?;
        let edges = self
            .prepare_edges(&write.depends_on, WriteKind::Task)
            .await?;
        let team = self.team_id().await?;
        let labels = self.label_ids(&write.item.labels, WriteKind::Task).await?;
        // The typed lists are what land, whatever the caller's own metadata held under their
        // keys: a key of either name travelling beside the field would be a second answer to
        // the same question, and the field is the one the contract names.
        let mut metadata = write.item.metadata.clone();
        set_task_list(&mut metadata, TaskRef::DELIVERS_KEY, &write.item.delivers);
        set_task_list(
            &mut metadata,
            TaskRef::DELIVERED_BY_KEY,
            &write.item.delivered_by,
        );
        let description = self.write_description(
            write.item.content.as_deref(),
            &metadata,
            &write.item.repositories,
            &edges,
            WriteKind::Task,
        )?;
        // `priority` on a create and on an update alike, `0` included: an update that left
        // it out would keep whatever the destination held, so a copy moving an issue back to
        // no priority would report success for a priority the destination still carries.
        let input = json!({"title":write.item.title,"description":description,"stateId":state,"priority":linear_priority(write.item.priority),"labelIds":labels,"projectId":project});
        let (query, variables, root) = match &write.target {
            // Its relations read back in the same request, so a rewrite needs no read of them.
            Some(id) => (
                graphql::ISSUE_REWRITE,
                json!({"id":id.0,"input":input,"first":MAX_PAGE_SIZE}),
                MutationRoot::IssueUpdate,
            ),
            None => (
                graphql::ISSUE_CREATE,
                {
                    let mut input = input;
                    input["teamId"] = Value::String(team.0);
                    json!({"input":input})
                },
                MutationRoot::IssueCreate,
            ),
        };
        let data = self.send(query, variables).await.inspect_err(|_| {
            self.forget_vocabulary();
        })?;
        let issue =
            mutation_payload(&data, root)?
                .get("issue")
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {}.issue", root.as_str()),
                })?;
        let id = NativeId(backend_id(issue, "id")?.into());
        let held = held_relations(write.target.as_ref(), issue)?;
        self.write_relations(&id, &edges, WriteKind::Task, held)
            .await?;
        Ok(id)
    }
    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        // Before anything is read or written, and before the item's own description
        // records these edges: an edge Linear will never accept has to refuse the whole
        // write, or a copy would create the project and then fail relating it, leaving the
        // undo to clean up a write that could have been refused without a call at all.
        if let Some(edge) = Self::unordered_project_edge(&write.depends_on) {
            return Err(self.unordered_project_relation(&write.item.id, edge.to.id()));
        }
        if let Some(key) = delivery_key_in(&write.item.metadata) {
            return Err(self.undeliverable(key, "project"));
        }
        if let Some(refused) = self.project_out_of_scope(write.target.as_ref()) {
            return Err(refused);
        }
        // Through the project side of the mapping, before anything else is sent: the item's
        // own status name plays no part, because a project's statuses are a vocabulary of
        // their own that the name a status was read under elsewhere says nothing about.
        let (status, _) = self
            .status_id(write.item.status.category, ItemKind::Project)
            .await?;
        let edges = self
            .prepare_edges(&write.depends_on, WriteKind::Project)
            .await?;
        let team = self.team_id().await?;
        let labels = self
            .label_ids(&write.item.labels, WriteKind::Project)
            .await?;
        let description = self.write_description(
            write.item.content.as_deref(),
            &write.item.metadata,
            &write.item.repositories,
            &edges,
            WriteKind::Project,
        )?;
        let input = json!({"name":write.item.title,"description":description,"statusId":status,"labelIds":labels});
        let (query, variables, root) = match &write.target {
            // Its relations read back in the same request, as an issue's are.
            Some(id) => (
                graphql::PROJECT_REWRITE,
                json!({"id":id.0,"input":input,"first":MAX_PAGE_SIZE}),
                MutationRoot::ProjectUpdate,
            ),
            None => (
                graphql::PROJECT_CREATE,
                {
                    let mut input = input;
                    input["teamIds"] = json!([team]);
                    json!({"input":input})
                },
                MutationRoot::ProjectCreate,
            ),
        };
        let data = self.send(query, variables).await.inspect_err(|_| {
            self.forget_vocabulary();
        })?;
        let project = mutation_payload(&data, root)?
            .get("project")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {}.project", root.as_str()),
            })?;
        let id = NativeId(backend_id(project, "id")?.into());
        let held = held_relations(write.target.as_ref(), project)?;
        self.write_relations(&id, &edges, WriteKind::Project, held)
            .await?;
        Ok(id)
    }
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, not an error — Linear reports
        // an unknown issue as an errored response rather than an unsuccessful payload, and
        // `get_task` answering `None` is what says the item is already gone.
        if self.get_task(id).await?.is_none() {
            return Ok(());
        }
        let data = self.send(graphql::ISSUE_DELETE, json!({"id":id.0})).await?;
        mutation_payload(&data, MutationRoot::IssueDelete)?;
        Ok(())
    }
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, on exactly the terms
        // `delete_task` reads it on.
        if self.get_project(id).await?.is_none() {
            return Ok(());
        }
        let data = self
            .send(graphql::PROJECT_DELETE, json!({"id":id.0}))
            .await?;
        mutation_payload(&data, MutationRoot::ProjectDelete)?;
        Ok(())
    }
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        // Read as an optional although the pinned `document(id:)` returns `Document!`, for
        // the reason `delete_task` records: Linear answers an id naming nothing with an
        // errored response rather than a null, and reading the null defensively is what
        // keeps a responder that does answer one from being a malformed-response failure.
        Ok(self.document_held(id).await?.map(|(document, _)| document))
    }
    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        // A document's text is applied over each fetched page with the labels and the
        // orphans, by the contract's own rule: both searches are declared for every level.
        let want = page.limit.min(MAX_PAGE_SIZE) as usize;
        let mut filter = serde_json::Map::new();
        if let ProjectFilter::Is(id) = &query.project {
            if !self.in_scope(Some(id)) {
                return Ok(Page::last(Vec::new()));
            }
            filter.insert("project".into(), json!({"id": {"eq": id.0}}));
        } else if let Some(scope) = &self.project {
            filter.insert("project".into(), json!({"id": {"eq": scope.0}}));
        }
        let filter = Value::Object(filter);
        let mut items = Vec::new();
        let mut cursor = page.cursor.clone();
        loop {
            // Only what is still owed, so the predicates applied here can never make this
            // return more than the caller asked for, and never drop what it fetched.
            let first = want.saturating_sub(items.len()).max(1);
            let d = self
                .send(
                    DOCUMENTS,
                    json!({"first":first,"after":cursor.as_ref().map(|cursor|&cursor.0),"filter":filter}),
                )
                .await?;
            let fetched = connection(&d, "documents", map_document)?;
            items.extend(fetched.items.into_iter().filter(|document| {
                document_matches(document, &query.project, &query.labels)
                    && self.in_scope(document.project.as_ref())
                    && query.text.as_ref().is_none_or(|text| {
                        text_holds(&document.title, document.content.as_deref(), text)
                    })
            }));
            cursor = fetched.next;
            if cursor.is_none() || items.len() >= want {
                return Ok(Page {
                    items,
                    next: cursor,
                });
            }
        }
    }
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        // Two refusals by name rather than two silent drops. Linear's own document type
        // has no labels and a document is not work, so neither a label nor a dependency
        // has anywhere here to land — and a copy that dropped one would report success for
        // an item the destination does not hold.
        if !write.item.labels.is_empty() {
            let named = write
                .item
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot carry a document's labels, because Linear's own \
                     document type has none: {named}",
                    self.name
                ),
            });
        }
        if !write.depends_on.is_empty()
            || write
                .item
                .metadata
                .contains_key(DependencyEdge::RECORDED_KEY)
        {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot carry {} on a document, because a document is not \
                     work and nothing may depend on one",
                    self.name,
                    DependencyEdge::RECORDED_KEY
                ),
            });
        }
        if let Some(key) = delivery_key_in(&write.item.metadata) {
            return Err(self.undeliverable(key, "document"));
        }
        let content = Self::long_form(
            write.item.content.as_deref(),
            &write.item.metadata,
            &write.item.repositories,
            Vec::new(),
        )?;
        let project = self.filed_in(write.item.project.as_ref(), "document")?;
        let (query, variables, root) = match &write.target {
            Some(id) => {
                // A target this workspace does not hold is refused rather than created:
                // the engine established that id before asking, so an absent one is a race
                // this destination must not paper over by writing a second document.
                if self.get_document(id).await?.is_none() {
                    return Err(SourceError::Refused {
                        message: format!("source {} holds no document {}", self.name, id.0),
                    });
                }
                (
                    graphql::DOCUMENT_UPDATE,
                    json!({"id":id.0,"input":{"title":write.item.title,"content":content,"projectId":project}}),
                    MutationRoot::DocumentUpdate,
                )
            }
            None => {
                let mut input = json!({"title":write.item.title,"content":content});
                // A Linear document lives in a project, an initiative, an issue or a team.
                // One filed under no project needs the configured team to be its home, and
                // one filed under a project already has one — so the team is asked for
                // only where it is the answer, rather than made a condition of every write.
                //
                // **`projectId` is left out rather than sent as null, and that is Linear's
                // rule rather than tidiness.** `documentCreate` refuses an input that names
                // more than one home — `Exactly one of initiativeId, teamId, issueId,
                // releaseId, cycleId or projectId must be defined.` — and it counts a
                // *present* key, observed on 2026-09-04: `{projectId: null, teamId: …}` is
                // refused where `{teamId: …}` is accepted. So a document filed under no
                // project must carry no `projectId` at all. `documentUpdate` is the
                // opposite and keeps its explicit null, because there the null is the
                // instruction — it is how a document is moved out of a project, and
                // omitting the key would leave it where it was.
                match &project {
                    Some(project) => input["projectId"] = Value::String(project.clone()),
                    None => input["teamId"] = Value::String(self.team_id().await?.0),
                }
                (
                    graphql::DOCUMENT_CREATE,
                    json!({ "input": input }),
                    MutationRoot::DocumentCreate,
                )
            }
        };
        let data = self.send(query, variables).await?;
        let document = mutation_payload(&data, root)?
            .get("document")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {}.document", root.as_str()),
            })?;
        Ok(NativeId(backend_id(document, "id")?.into()))
    }
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, on exactly the terms
        // `delete_task` reads it on.
        if self.get_document(id).await?.is_none() {
            return Ok(());
        }
        let data = self
            .send(graphql::DOCUMENT_DELETE, json!({"id":id.0}))
            .await?;
        mutation_payload(&data, MutationRoot::DocumentDelete)?;
        Ok(())
    }
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        // A page of no rows is not a page: refused here rather than sent as `last: 0`, which
        // would answer an empty page that reads as a task with no comments.
        if page.limit == 0 {
            return Err(SourceError::Config {
                message: "a page limit of 0 is not a page; ask for at least 1 comment".to_owned(),
            });
        }
        // One request rather than a task lookup and then a read: the issue the comments
        // hang off answers "no such task" by itself, on exactly the terms `get_task` reads
        // it — null, or trashed.
        let d = self
            .send(
                graphql::ISSUE_COMMENTS,
                json!({"id":task.0,"last":page.limit.min(MAX_PAGE_SIZE),"before":page.cursor.as_ref().map(|c|&c.0)}),
            )
            .await?;
        optional(&d, "issue", comment_page)
    }
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        // Before anything is sent, because nothing Linear could answer changes it: see the
        // ruling on the author in this crate's module documentation.
        if let Some(author) = &comment.author {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot post a comment as {author:?}, because Linear records the \
                     user whose API key makes the request as the author of every comment; \
                     leave --author out to post as that user",
                    self.name
                ),
            });
        }
        let Some(issue) = self.commented_issue(task).await? else {
            return Ok(None);
        };
        let data = self
            .send(
                graphql::COMMENT_CREATE,
                json!({"input":{"issueId":issue.0,"body":comment.body.as_str()}}),
            )
            .await?;
        written_comment(&data, MutationRoot::CommentCreate).map(Some)
    }
    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        if !self.comment_is_on(task, comment).await? {
            return Ok(None);
        }
        // `body` alone: the id, the author and the time it was written are the comment's
        // own, so nothing else is sent that Linear could move.
        let data = self
            .send(
                graphql::COMMENT_UPDATE,
                json!({"id":comment.0,"input":{"body":body.as_str()}}),
            )
            .await?;
        written_comment(&data, MutationRoot::CommentUpdate).map(Some)
    }
    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        if !self.comment_is_on(task, comment).await? {
            return Ok(None);
        }
        let data = self
            .send(graphql::COMMENT_DELETE, json!({"id":comment.0}))
            .await?;
        mutation_payload(&data, MutationRoot::CommentDelete)?;
        Ok(Some(comment.clone()))
    }
    /// Resolved exactly as the write resolves it, through the instance's held vocabulary, so
    /// the write that follows sends no read this did not — and a project write the scope
    /// refuses is refused here first, before the resolution is read for it.
    async fn check_status_write(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        target: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        if kind == ItemKind::Project
            && let Some(refused) = self.project_out_of_scope(target)
        {
            return Err(refused);
        }
        self.status_id(category, kind).await.map(|_| ())
    }
    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        Ok(self
            .status_written(id, category)
            .await?
            .map(|task| task.status))
    }
    /// One `issueUpdate` and nothing before it — see `status_written` — answering the task
    /// its own selection reports.
    async fn set_task_status_reading(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Task>, SourceError> {
        self.status_written(id, category).await
    }
    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        // Read first: a trashed issue is not one this source holds, so it is never written
        // to, and the read is what tells "no such task" from a refusal here — unlike a status
        // write, whose budget the read is not worth.
        let Some(task) = self.get_task(id).await? else {
            return Ok(None);
        };
        // `priority` alone: every member of `IssueUpdateInput` is optional and Linear leaves
        // an absent one as the issue holds it, so nothing else about the issue can move.
        let data = self
            .send(
                graphql::ISSUE_PRIORITY_UPDATE,
                json!({"id":task.id.0,"input":{"priority":linear_priority(priority)}}),
            )
            .await?;
        let issue = mutation_payload(&data, MutationRoot::IssueUpdate)?
            .get("issue")
            .filter(|issue| !issue.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing issueUpdate.issue".into(),
            })?;
        written_is(issue, &task.id)?;
        issue_priority(issue).map(Some)
    }
    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        // The raw description, because the metadata slot lives in that same field and has to
        // go back byte for byte: re-encoding it would be a metadata write nobody asked for.
        // The whole issue is read as `get_task` reads it first, so an issue this source could
        // not read is refused before anything is written rather than overwritten blind.
        let Some((task, description)) = self.issue_held(id).await? else {
            return Ok(None);
        };
        let issue = task.id;
        let slot = match description.as_deref() {
            Some(description) => metadata_slot(description)?.map(str::to_owned),
            None => None,
        };
        let description = match &slot {
            None => content.to_owned(),
            Some(slot) if content.is_empty() => slot.clone(),
            // The separator `long_form` writes, so a content write and a copy leave one shape.
            Some(slot) => format!("{content}\n\n{slot}"),
        };
        // Checked before anything is sent: content ending in what this source reads as its own
        // metadata slot would read back as metadata rather than as the content it was.
        if metadata_slot(&description)? != slot.as_deref() {
            return Err(SourceError::Refused {
                message: format!(
                    "this content ends in what source {} reads as its own metadata slot, so part \
                     of it would read back as metadata rather than as content; next: remove that \
                     trailing block from the content",
                    self.name
                ),
            });
        }
        // And what a read will report is exactly what was asked for, or nothing is sent.
        let (reads, _) = metadata_description(Some(description.clone()))?;
        if reads.as_deref().unwrap_or_default() != content {
            return Err(SourceError::Refused {
                message: format!(
                    "this content would read back from source {} as {:?} rather than as itself; \
                     next: change how the content ends",
                    self.name,
                    reads.as_deref().unwrap_or_default()
                ),
            });
        }
        // `description` alone, for the reason `set_task_priority` sends `priority` alone.
        let data = self
            .send(
                graphql::ISSUE_UPDATE,
                json!({"id":issue.0,"input":{"description":description}}),
            )
            .await?;
        let written = mutation_payload(&data, MutationRoot::IssueUpdate)?
            .get("issue")
            .filter(|issue| !issue.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing issueUpdate.issue".into(),
            })?;
        written_is(written, &issue)?;
        Ok(Some(()))
    }
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        let entries = TaskRef::listed(
            TaskRef::DELIVERED_BY_KEY,
            id,
            Some(&self.name),
            delivered_by.to_vec(),
        )
        .map_err(|message| SourceError::Refused { message })?;
        let Some((task, description)) = self.issue_held(id).await? else {
            return Ok(None);
        };
        let (_, held) = metadata_description(description.clone())?;
        let mut slot = held.clone();
        set_task_list(&mut slot, TaskRef::DELIVERED_BY_KEY, &entries);
        // Compared as JSON rather than as the field's bytes, so a slot already holding the
        // list is not rewritten for its spelling alone.
        if slot != held {
            let rewritten = reslotted(description.as_deref(), &slot)?;
            self.write_description_alone(&task.id, rewritten.as_deref())
                .await?;
        }
        Ok(Some(()))
    }
    /// One read of the issue and, unless the key already holds the value, one `issueUpdate`
    /// carrying the description alone, whose metadata slot is the only part that moved.
    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        let Some((task, description)) = self.issue_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(description.clone())?;
        if slot.get(key.as_str()) == Some(value) {
            return Ok(Some(task));
        }
        slot.insert(key.as_str().to_owned(), value.clone());
        let rewritten = reslotted(description.as_deref(), &slot)?;
        self.write_description_alone(&task.id, rewritten.as_deref())
            .await?;
        self.get_task(&task.id)
            .await?
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "task {} was written and then could not be read back",
                    task.id
                ),
            })
    }
    /// One metadata key of a project, on the terms of `set_task_metadata`: one read, and one
    /// `projectUpdate` carrying the description alone.
    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        let Some((project, description)) = self.project_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(description.clone())?;
        if slot.get(key.as_str()) == Some(value) {
            return Ok(Some(project));
        }
        slot.insert(key.as_str().to_owned(), value.clone());
        let rewritten = reslotted(description.as_deref(), &slot)?;
        self.write_project_description(&project.id, rewritten.as_deref())
            .await?;
        self.get_project(&project.id)
            .await?
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "project {} was written and then could not be read back",
                    project.id
                ),
            })
    }
    /// One metadata key of a document, on the terms of `set_task_metadata`: one read, and one
    /// `documentUpdate` carrying the content alone.
    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        let Some((document, content)) = self.document_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(content.clone())?;
        if slot.get(key.as_str()) == Some(value) {
            return Ok(Some(document));
        }
        slot.insert(key.as_str().to_owned(), value.clone());
        let rewritten = reslotted(content.as_deref(), &slot)?;
        self.write_document_content(&document.id, rewritten.as_deref())
            .await?;
        self.get_document(&document.id)
            .await?
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "document {} was written and then could not be read back",
                    document.id
                ),
            })
    }
    /// One read of the issue and one `issueUpdate` carrying the description alone: the new
    /// content, and a slot whose `onetaskgraph.template` entry is `provenance` and whose every
    /// other entry is as it was. This source keeps no template answers — an issue has no room
    /// beside its description that is not the description, and answers written there would
    /// repeat what the content already says — so `answers` reaches nothing here.
    async fn set_task_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        _answers: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let Some((task, description)) = self.issue_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(description.clone())?;
        slot.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance.clone());
        // No read-back check, unlike a content write: this slot is never empty, it is written
        // after the content, and a read takes the last slot off first, so the content reads
        // back as itself whatever it ends in.
        let rewritten = Self::described(Some(content), &slot)?;
        if rewritten != description {
            self.write_description_alone(&task.id, rewritten.as_deref())
                .await?;
        }
        Ok(Some(()))
    }
    /// One document's rendering, on the terms of `set_task_rendering`, through one
    /// `documentUpdate` carrying the content alone.
    async fn set_document_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        _answers: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let Some((document, held)) = self.document_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(held.clone())?;
        slot.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance.clone());
        let rewritten = Self::described(Some(content), &slot)?;
        if rewritten != held {
            self.write_document_content(&document.id, rewritten.as_deref())
                .await?;
        }
        Ok(Some(()))
    }
    /// One project's rendering, on the terms of `set_task_rendering`, through one
    /// `projectUpdate` carrying the description alone.
    async fn set_project_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        _answers: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let Some((project, description)) = self.project_held(id).await? else {
            return Ok(None);
        };
        let (_, mut slot) = metadata_description(description.clone())?;
        slot.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance.clone());
        let rewritten = Self::described(Some(content), &slot)?;
        if rewritten != description {
            self.write_project_description(&project.id, rewritten.as_deref())
                .await?;
        }
        Ok(Some(()))
    }
    /// One read of the issue and one `issueUpdate` carrying only what differs; see
    /// `targeted_update`.
    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.targeted_update(id, update).await
    }

    /// Nothing to drop: what this source holds across a command is its [`Vocabulary`] — the
    /// team's id, its workflow states and the workspace's project statuses — which stays valid
    /// in normal use and is read afresh when a name misses. No issue, project, document or
    /// search answer outlives the call that read it, so the next command already reads each
    /// item as a person left it.
    async fn end_command(&self) -> Result<(), SourceError> {
        Ok(())
    }
}

/// What a whole write's own answer says of the relations its item holds: none for an item it
/// created, and for one it rewrote the first page its selection read back.
fn held_relations<'a>(
    target: Option<&NativeId>,
    written: &'a Value,
) -> Result<HeldRelations<'a>, SourceError> {
    if target.is_none() {
        return Ok(HeldRelations::None);
    }
    written
        .get("relations")
        .filter(|relations| !relations.is_null())
        .map(HeldRelations::Page)
        .ok_or_else(|| SourceError::Malformed {
            message: "a rewrite answered without the relations it was asked for".into(),
        })
}

/// Refuse a narrow project or document write whose payload names another item than the one it
/// was sent for, on the terms [`written_is`] refuses an issue's.
fn acknowledged(item: &Value, asked: &NativeId, root: MutationRoot) -> Result<(), SourceError> {
    let written = backend_id(item, "id")?;
    if written == asked.0 {
        return Ok(());
    }
    Err(SourceError::Malformed {
        message: format!(
            "{} for {asked} answered with the item {written}",
            root.as_str()
        ),
    })
}

/// Refuse a narrow write's payload naming an issue other than the one it was sent for.
///
/// An `issueUpdate` answering with another issue is not this write landing, so it is reported
/// as the malformed answer it is rather than as the task having been written.
fn written_is(issue: &Value, asked: &NativeId) -> Result<(), SourceError> {
    let written = backend_id(issue, "id")?;
    if written == asked.0 {
        return Ok(());
    }
    Err(SourceError::Malformed {
        message: format!("issueUpdate for {asked} answered with the issue {written}"),
    })
}

/// Why a project and a document carry neither [`Task::delivers`] nor [`Task::delivered_by`].
///
/// Only a task delivers or is delivered, so the two reserved keys name nothing a project or a
/// document has. A task keeps both in its description's metadata slot; anything else naming
/// one is refused by name rather than written.
const NO_DELIVERY: &str = "only a task delivers or is delivered, so neither list has a place \
                           on anything else";

/// The reserved delivery key `metadata` carries, if it carries one.
fn delivery_key_in(metadata: &std::collections::BTreeMap<String, Value>) -> Option<&'static str> {
    [TaskRef::DELIVERS_KEY, TaskRef::DELIVERED_BY_KEY]
        .into_iter()
        .find(|key| metadata.contains_key(*key))
}

/// A category as the wire spells it — `in-progress`, `queued` — for a message.
fn category_word(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{category:?}"))
}

impl LinearSource {
    /// The refusal a write naming `named` — a field or a reserved key — on a `what` gets.
    fn undeliverable(&self, named: &str, what: &str) -> SourceError {
        SourceError::Refused {
            message: format!(
                "source {} cannot carry {named} on a {what}: {NO_DELIVERY}; write the {what} \
                 without it",
                self.name
            ),
        }
    }

    /// The backend id of the issue `task` names, or `None` when this source holds no such
    /// task — resolved by `get_task` itself, so a comment call and a task read cannot
    /// disagree about whether a task is there.
    ///
    /// The id Linear answers with rather than the one asked for, because `issue(id:)` also
    /// takes an identifier such as `ENG-1`, and the comment's own `issue{id}` is compared
    /// against — and a comment is created on — the backend id.
    async fn commented_issue(&self, task: &NativeId) -> Result<Option<NativeId>, SourceError> {
        Ok(self.get_task(task).await?.map(|task| task.id))
    }

    /// Whether `comment` is a comment on the issue `task` names.
    ///
    /// Asked before any edit or removal, so an id belonging to another issue — or to no
    /// issue, or to nothing — is answered as no such comment without a mutation reaching
    /// Linear. `commentUpdate` and `commentDelete` address a comment by its id alone, so
    /// without this a task named in error would edit or remove somebody else's comment.
    async fn comment_is_on(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<bool, SourceError> {
        let Some(issue) = self.commented_issue(task).await? else {
            return Ok(false);
        };
        let data = self.send(graphql::COMMENT, json!({"id":comment.0})).await?;
        Ok(optional(&data, "comment", comment_issue)?.flatten() == Some(issue))
    }
}

impl LinearSource {
    /// One issue as this source reads it, beside its raw `description` — or `None` for an
    /// issue Linear does not hold, has trashed, or that is outside the project this source is
    /// scoped to.
    ///
    /// Every read of one issue goes through here, so a status, a content, a metadata and a
    /// rendering write all answer "no such task" on exactly the terms `get_task` does.
    async fn issue_held(
        &self,
        id: &NativeId,
    ) -> Result<Option<(Task, Option<String>)>, SourceError> {
        let data = self.send(ISSUE, json!({"id":id.0})).await?;
        Ok(optional(&data, "issue", |v| {
            Ok((
                map_task(v, &self.name, &self.statuses)?,
                optional_string(v, "description")?,
            ))
        })?
        .filter(|(task, _)| self.in_scope(task.project.as_ref())))
    }

    /// One project and its raw `description`, on the terms of [`Self::issue_held`]: a source
    /// scoped to one project holds that one alone.
    async fn project_held(
        &self,
        id: &NativeId,
    ) -> Result<Option<(Project, Option<String>)>, SourceError> {
        let data = self.send(PROJECT, json!({"id":id.0})).await?;
        Ok(optional(&data, "project", |v| {
            Ok((
                map_project(v, &self.statuses)?,
                optional_string(v, "description")?,
            ))
        })?
        .filter(|(project, _)| self.in_scope(Some(&project.id))))
    }

    /// One document and its raw `content`, on the terms of [`Self::issue_held`].
    async fn document_held(
        &self,
        id: &NativeId,
    ) -> Result<Option<(Document, Option<String>)>, SourceError> {
        // Read as an optional although the pinned `document(id:)` returns `Document!`, for
        // the reason `delete_task` records: Linear answers an id naming nothing with an
        // errored response rather than a null, and reading the null defensively is what
        // keeps a responder that does answer one from being a malformed-response failure.
        let data = self.send(DOCUMENT, json!({"id":id.0})).await?;
        Ok(optional(&data, "document", |v| {
            Ok((map_document(v)?, optional_string(v, "content")?))
        })?
        .filter(|(document, _)| self.in_scope(document.project.as_ref())))
    }

    /// Send one issue's new `description` alone, and nothing else about it.
    async fn write_description_alone(
        &self,
        id: &NativeId,
        description: Option<&str>,
    ) -> Result<(), SourceError> {
        let data = self
            .send(
                graphql::ISSUE_UPDATE,
                json!({"id":id.0,"input":{"description":description}}),
            )
            .await?;
        let issue = mutation_payload(&data, MutationRoot::IssueUpdate)?
            .get("issue")
            .filter(|issue| !issue.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing issueUpdate.issue".into(),
            })?;
        written_is(issue, id)
    }

    /// Send one project's new `description` alone.
    async fn write_project_description(
        &self,
        id: &NativeId,
        description: Option<&str>,
    ) -> Result<(), SourceError> {
        let data = self
            .send(
                graphql::PROJECT_UPDATE,
                json!({"id":id.0,"input":{"description":description}}),
            )
            .await?;
        let project = mutation_payload(&data, MutationRoot::ProjectUpdate)?
            .get("project")
            .filter(|project| !project.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing projectUpdate.project".into(),
            })?;
        acknowledged(project, id, MutationRoot::ProjectUpdate)
    }

    /// Send one document's new `content` alone.
    async fn write_document_content(
        &self,
        id: &NativeId,
        content: Option<&str>,
    ) -> Result<(), SourceError> {
        let data = self
            .send(
                graphql::DOCUMENT_UPDATE,
                json!({"id":id.0,"input":{"content":content}}),
            )
            .await?;
        let document = mutation_payload(&data, MutationRoot::DocumentUpdate)?
            .get("document")
            .filter(|document| !document.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "missing documentUpdate.document".into(),
            })?;
        acknowledged(document, id, MutationRoot::DocumentUpdate)
    }

    /// What `sources fields` reports — and, with `apply`, does first: every name the mapping
    /// gives each kind, checked against that kind's vocabulary, each missing one created when
    /// `apply` asks.
    ///
    /// Created one at a time, tasks' then projects', each in category order, so a create Linear
    /// refuses stops the run with every name before it created and reported so. A name present
    /// under another type is reported with its type and left exactly as it is: nothing here
    /// renames, retypes or deletes.
    async fn status_names(&self, apply: bool) -> Result<StatusNamesReport, SourceError> {
        let team = self.team.clone().ok_or_else(|| SourceError::Refused {
            message: format!(
                "source {} needs config.team to report its status names",
                self.name
            ),
        })?;
        let (mut vocabulary, _) = self.vocabulary(false).await?;
        let mut names = Vec::new();
        let mut refused = None;
        for kind in [ItemKind::Task, ItemKind::Project] {
            for (category, name) in self.statuses.names(kind) {
                let found = vocabulary
                    .find(kind, name.as_str(), &self.name)?
                    .map(|held| held.kind.clone());
                let mut mapped = MappedStatusName {
                    kind,
                    category,
                    name: name.clone(),
                    found: found.map_or(Found::Missing, Found::Present),
                };
                if apply && refused.is_none() && mapped.found == Found::Missing {
                    match self
                        .create_status_name(kind, category, name.as_str(), &vocabulary)
                        .await
                    {
                        Ok(created) => {
                            mapped.found = Found::Created(created.held().kind.clone());
                            self.remember(&created);
                            // Here too, so the next project status goes after this one even
                            // when nothing is held.
                            std::sync::Arc::make_mut(&mut vocabulary).add(created);
                        }
                        Err(error) => {
                            refused = Some(RefusedCreate {
                                kind,
                                name: name.clone(),
                                message: error.to_string(),
                            });
                        }
                    }
                }
                names.push(mapped);
            }
        }
        Ok(StatusNamesReport {
            source: self.name.clone(),
            team,
            names,
            refused,
        })
    }

    /// Create one name of `kind` — a workflow state on the vocabulary's team, or a project
    /// status of the workspace placed after its last — of the type its category derives, in
    /// the fixed colour every created name takes.
    async fn create_status_name(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        name: &str,
        vocabulary: &Vocabulary,
    ) -> Result<Created, SourceError> {
        let kind_of = created_type(category, kind);
        let (query, input, root, payload) = match kind {
            ItemKind::Task => (
                graphql::WORKFLOW_STATE_CREATE,
                json!({"teamId": vocabulary.team.0, "name": name, "type": kind_of,
                       "color": CREATED_COLOR}),
                MutationRoot::WorkflowStateCreate,
                "workflowState",
            ),
            ItemKind::Project => (
                graphql::PROJECT_STATUS_CREATE,
                json!({"name": name, "type": kind_of, "color": CREATED_COLOR,
                       "position": vocabulary.last_position + 1.0}),
                MutationRoot::ProjectStatusCreate,
                "status",
            ),
        };
        let data = self.send(query, json!({ "input": input })).await?;
        let created = mutation_payload(&data, root)?
            .get(payload)
            .filter(|created| !created.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {}.{payload}", root.as_str()),
            })?;
        let held = Held::read(created)?;
        // Held only as what was asked for: an answer naming another name or type would be
        // remembered as this mapping's name, and reported created, when it is not.
        if held.name.as_str() != name || held.kind != kind_of {
            return Err(SourceError::Malformed {
                message: format!(
                    "Linear answered the create of {} {name:?} of type {kind_of} with {:?} of \
                     type {}",
                    vocabulary_word(kind),
                    held.name.as_str(),
                    held.kind
                ),
            });
        }
        Ok(match kind {
            ItemKind::Task => Created::State(held),
            // Placed where Linear says it put it, so the next one goes after.
            ItemKind::Project => Created::Status {
                position: position_of(created)?,
                held,
            },
        })
    }
}

/// `held` with its trailing metadata slot replaced by one holding exactly `slot`, and every
/// byte above the slot exactly as it was — or with a slot appended after one blank line where
/// it had none, and the slot taken off, with the one blank line that set it off, where `slot`
/// is empty.
///
/// The one place a narrow metadata write composes a long-form field, so a metadata set, a
/// `delivered_by` write and a copy-link record move nothing a person wrote.
fn reslotted(
    held: Option<&str>,
    slot: &std::collections::BTreeMap<String, Value>,
) -> Result<Option<String>, SourceError> {
    let held = held.unwrap_or_default();
    let Some((start, _, _)) = slot_bounds(held)? else {
        return LinearSource::described((!held.is_empty()).then_some(held), slot);
    };
    let above = &held[..start];
    if slot.is_empty() {
        let visible = above
            .strip_suffix("\n\n")
            .or_else(|| above.strip_suffix('\n'))
            .unwrap_or(above);
        return Ok((!visible.is_empty()).then(|| visible.to_owned()));
    }
    Ok(Some(format!("{above}{}", slot_text(slot)?)))
}

/// Hold `entries` under `key` in one slot's metadata, or no such key when there are none.
fn set_task_list(
    metadata: &mut std::collections::BTreeMap<String, Value>,
    key: &str,
    entries: &[TaskRef],
) {
    if entries.is_empty() {
        metadata.remove(key);
    } else {
        metadata.insert(
            key.to_owned(),
            Value::Array(
                entries
                    .iter()
                    .map(|entry| Value::String(entry.as_str().to_owned()))
                    .collect(),
            ),
        );
    }
}

/// One page of an issue's comments, oldest first.
///
/// Linear answered newest first, walking backwards from `before`, so the page is reversed
/// and the next cursor is the one *behind* it; see the ruling on comments in this crate's
/// module documentation for why the walk runs that way.
fn comment_page(v: &Value) -> Result<Page<Comment>, SourceError> {
    let c = v.get("comments").ok_or_else(|| SourceError::Malformed {
        message: "missing comments connection".into(),
    })?;
    let mut items = c
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing comment nodes".into(),
        })?
        .iter()
        .map(map_comment)
        .collect::<Result<Vec<_>, _>>()?;
    items.reverse();
    let info = c.get("pageInfo").ok_or_else(|| SourceError::Malformed {
        message: "missing pageInfo".into(),
    })?;
    let older = info
        .get("hasPreviousPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing boolean pageInfo.hasPreviousPage".into(),
        })?;
    let next = if older {
        Some(Cursor(str_at(info, "startCursor")?.into()))
    } else {
        None
    };
    Ok(Page { items, next })
}

fn map_comment(v: &Value) -> Result<Comment, SourceError> {
    let author = match v.get("user") {
        None => {
            return Err(SourceError::Malformed {
                message: "missing comment user field".into(),
            });
        }
        // An integration or a bot: Linear names no user, and this source invents none.
        Some(Value::Null) => None,
        Some(user) => Some(str_at(user, "displayName")?.to_owned()),
    };
    Ok(Comment {
        id: NativeId(backend_id(v, "id")?.into()),
        author,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        body: str_at(v, "body")?.into(),
        url: optional_string(v, "url")?,
    })
}

/// The comment a `commentCreate` or `commentUpdate` answered with, as Linear now holds it.
fn written_comment(data: &Value, root: MutationRoot) -> Result<Comment, SourceError> {
    let comment = mutation_payload(data, root)?
        .get("comment")
        .ok_or_else(|| SourceError::Malformed {
            message: format!("missing {}.comment", root.as_str()),
        })?;
    map_comment(comment)
}

/// The issue a comment is on, or `None` for a comment on something else — a project, a
/// document, an update — which is a comment no task of this source has.
fn comment_issue(v: &Value) -> Result<Option<NativeId>, SourceError> {
    match v.get("issue") {
        None => Err(SourceError::Malformed {
            message: "missing comment issue field".into(),
        }),
        Some(Value::Null) => Ok(None),
        Some(issue) => Ok(Some(NativeId(backend_id(issue, "id")?.into()))),
    }
}

/// Linear relates one Linear item to another and nothing else, so an edge whose far end
/// is in a different source is the one edge no `relations` entry can hold. Those edges
/// are read from the near item's own [`DependencyEdge::RECORDED_KEY`] metadata, and they
/// are served *after* the native relations are spent: a page under this cursor is the
/// recorded tail of the same walk, which keeps the native pages exactly what they were.
const RECORDED_CURSOR: &str = "onetaskgraph.depends_on:";

impl LinearSource {
    async fn dependencies(
        &self,
        query: &str,
        root: DependencyRoot,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let limit = page.limit.min(MAX_PAGE_SIZE);
        let cursor = page.cursor.as_ref().map(|c| c.0.as_str());
        if let Some(offset) = cursor.and_then(|c| c.strip_prefix(RECORDED_CURSOR)) {
            // This cursor resumes the *forward* tail and only a forward walk ever issues
            // one, so a reverse read carrying it is resuming a walk it did not come from.
            // Serving it would answer a reverse read with forward edges, which is the one
            // thing a recorded edge must never do — its reverse is derived from the far
            // end and is never written down here.
            if direction != Direction::DependsOn {
                return Err(SourceError::Malformed {
                    message: format!(
                        "{RECORDED_CURSOR}{offset} resumes recorded forward edges, which a                          reverse dependency read never issues; resume it in the direction                          that reported it"
                    ),
                });
            }
            let offset: usize = offset.parse().map_err(|_| SourceError::Malformed {
                message: format!("{RECORDED_CURSOR}{offset} is not a recorded-edge cursor"),
            })?;
            let d = self
                .send(query, json!({"id":id.0,"first":1,"after":null}))
                .await?;
            return Ok(recorded_page(
                recorded(&d, root, id, &self.name)?,
                offset,
                limit as usize,
            ));
        }
        let d = self
            .send(query, json!({"id":id.0,"first":limit,"after":cursor}))
            .await?;
        let mut answered = relation_page(&d, root, id, direction)?;
        // Only forwards: the reverse of a recorded edge is derived from the far end, never
        // written down on the near item.
        if answered.next.is_none()
            && direction == Direction::DependsOn
            && !recorded(&d, root, id, &self.name)?.is_empty()
        {
            answered.next = Some(Cursor(format!("{RECORDED_CURSOR}0")));
        }
        Ok(answered)
    }
}

fn recorded(
    d: &Value,
    root: DependencyRoot,
    id: &NativeId,
    name: &SourceName,
) -> Result<Vec<DependencyEdge>, SourceError> {
    let item = d.get(root.as_str()).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {}", root.as_str()),
    })?;
    let (_, metadata) = metadata_description(optional_string(item, "description")?)?;
    // `relations` on an issue holds issues and on a project holds projects, both of this
    // workspace — so a same-kind far end in this same source is one Linear itself was
    // supposed to hold, and the key is refused rather than quietly read, whether the entry
    // left the source out or spelled this one.
    DependencyEdge::recorded(
        &metadata,
        id,
        root.item_kind(),
        name,
        Some(root.item_kind()),
    )
    .map_err(|message| SourceError::Malformed { message })
}

fn recorded_page(edges: Vec<DependencyEdge>, offset: usize, limit: usize) -> Page<DependencyEdge> {
    let total = edges.len();
    let items: Vec<DependencyEdge> = edges.into_iter().skip(offset).take(limit.max(1)).collect();
    let end = offset.saturating_add(items.len());
    Page {
        items,
        next: (end < total).then(|| Cursor(format!("{RECORDED_CURSOR}{end}"))),
    }
}

/// The status an issue's workflow state or a project's project status reads as, through this
/// instance's `status_mapping` for that kind.
///
/// A name the kind's mapping names reads as that category, under the name Linear holds — which
/// is what lets two states of one type, `Todo` and `Queued`, read as two categories. Every
/// other name reads as `unknown` under its own name, whatever its type: Linear has no built-in
/// names, so a type is not a category, and reading one as if it were would report a category
/// no write of this source could have put it at.
fn mapped_status(
    v: &Value,
    statuses: &StatusMapping,
    kind: ItemKind,
) -> Result<Status, SourceError> {
    let name = str_at(v, "name")?;
    Ok(Status {
        category: statuses
            .category_of(kind, name)
            .unwrap_or(StatusCategory::Unknown),
        name: name.to_owned(),
    })
}
fn str_at<'a>(v: &'a Value, k: &str) -> Result<&'a str, SourceError> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("missing string field {k}"),
        })
}
fn map_label(v: &Value) -> Result<Label, SourceError> {
    Ok(Label {
        id: NativeId(str_at(v, "id")?.into()),
        name: str_at(v, "name")?.into(),
        color: optional_string(v, "color")?,
    })
}
fn labels_of(v: &Value) -> Result<Vec<Label>, SourceError> {
    v.get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing label nodes".into(),
        })?
        .iter()
        .map(map_label)
        .collect()
}
fn time(v: &Value, k: &str) -> Result<Option<DateTime<Utc>>, SourceError> {
    optional_str(v, k)?
        .map(|s| {
            s.parse().map_err(|e| SourceError::Malformed {
                message: format!("invalid {k}: {e}"),
            })
        })
        .transpose()
}
/// One issue as a task, `source` being this source's configured name.
///
/// The name is what lets [`TaskRef::listed`] tell `work:I-1` on the issue `I-1` of the
/// source `work` apart as that issue itself, rather than recognising only the bare spelling.
fn map_task(v: &Value, source: &SourceName, statuses: &StatusMapping) -> Result<Task, SourceError> {
    let (content, mut metadata) = metadata_description(optional_string(v, "description")?)?;
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    let id = NativeId(str_at(v, "id")?.into());
    // Taken out of the caller's metadata as they are read: a reserved key is this product's,
    // and reporting it there as well would hand a consumer two spellings of one list.
    let delivers = delivery_list(&mut metadata, TaskRef::DELIVERS_KEY, &id, source)?;
    let delivered_by = delivery_list(&mut metadata, TaskRef::DELIVERED_BY_KEY, &id, source)?;
    Ok(Task {
        id,
        // `Issue.identifier` is `String!` and every read of an issue selects it, so a
        // response without one is a response this source cannot read rather than an issue
        // with no handle — Linear gives every issue one.
        key: Some(str_at(v, "identifier")?.into()),
        title: str_at(v, "title")?.into(),
        content,
        status: mapped_status(
            v.get("state").ok_or_else(|| SourceError::Malformed {
                message: "missing state".into(),
            })?,
            statuses,
            ItemKind::Task,
        )?,
        priority: issue_priority(v)?,
        labels: labels_of(v.get("labels").ok_or_else(|| SourceError::Malformed {
            message: "missing labels".into(),
        })?)?,
        project: filed_under(v)?,
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
        delivers,
        delivered_by,
    })
}
/// A priority as Linear's `Issue.priority` and its two input members spell it.
///
/// Linear's own scale, as its published schema describes the field: `0` is no priority,
/// `1` urgent, `2` high, `3` normal and `4` low. Normal is this contract's `medium`.
const fn linear_priority(priority: Priority) -> u8 {
    match priority {
        Priority::None => 0,
        Priority::Urgent => 1,
        Priority::High => 2,
        Priority::Medium => 3,
        Priority::Low => 4,
    }
}

/// The priority an issue carries, read from `Issue.priority`.
///
/// Linear declares that field `Float!` while its inputs take an `Int`, so `2` and `2.0` are
/// the same answer. Anything else — absent, null, fractional, or outside `0` to `4` — is a
/// response this source cannot read, never a guess at the nearest level: a priority reported
/// that a filter for it could not find is capability rule 1 broken.
fn issue_priority(v: &Value) -> Result<Priority, SourceError> {
    let raw = v.get("priority").ok_or_else(|| SourceError::Malformed {
        message: "missing number field priority".into(),
    })?;
    let level = raw.as_f64().filter(|level| level.fract() == 0.0);
    Priority::ALL
        .into_iter()
        .find(|priority| level == Some(f64::from(linear_priority(*priority))))
        .ok_or_else(|| SourceError::Malformed {
            message: format!(
                "field priority is {raw}, which is none of Linear's priorities 0 (none), \
                 1 (urgent), 2 (high), 3 (normal) and 4 (low)"
            ),
        })
}

/// One delivery list read out of an issue's metadata slot, and removed from it.
///
/// An entry that is not a task id, that names the issue itself, or that repeats is a
/// malformed response naming the task and the entry, never a list quietly shortened.
fn delivery_list(
    metadata: &mut std::collections::BTreeMap<String, Value>,
    key: &str,
    task: &NativeId,
    source: &SourceName,
) -> Result<Vec<TaskRef>, SourceError> {
    let held = metadata.remove(key);
    TaskRef::from_value(key, task, Some(source), held.as_ref())
        .map_err(|message| SourceError::Malformed { message })
}
/// Remove the two delivery keys from a project's or a document's metadata.
///
/// Neither is work that delivers anything, so a key there names nothing this contract has,
/// and it is not the caller's free metadata either: it is this product's reserved spelling.
fn strip_delivery_keys(metadata: &mut std::collections::BTreeMap<String, Value>) {
    metadata.remove(TaskRef::DELIVERS_KEY);
    metadata.remove(TaskRef::DELIVERED_BY_KEY);
}
fn map_project(v: &Value, statuses: &StatusMapping) -> Result<Project, SourceError> {
    let (content, mut metadata) = metadata_description(optional_string(v, "description")?)?;
    strip_delivery_keys(&mut metadata);
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    Ok(Project {
        id: NativeId(str_at(v, "id")?.into()),
        title: str_at(v, "name")?.into(),
        content,
        status: mapped_status(
            v.get("status").ok_or_else(|| SourceError::Malformed {
                message: "missing status".into(),
            })?,
            statuses,
            ItemKind::Project,
        )?,
        labels: labels_of(v.get("labels").ok_or_else(|| SourceError::Malformed {
            message: "missing project labels".into(),
        })?)?,
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
}

/// Where a Linear entity is: the web address Linear itself reports for it, as a link.
///
/// Every issue, project and document of a Linear workspace has a page a person can open,
/// so this source says so for all three — the counterpart of a folder of Markdown
/// reporting the path of the file behind an item. A source that reported nothing here is
/// what leaves a reader holding an opaque id, and `None` is reserved for the case Linear
/// really did not say, which is not the same as saying the entity is nowhere.
fn web_address(url: Option<&str>) -> Option<Location> {
    url.map(|url| Location::Url(url.to_owned()))
}

/// The project a Linear item is filed under, or `None` for one filed under nothing.
///
/// One reader for issues and documents alike, because the field is the same field: an
/// absent `project` key is a malformed response, a null one is an orphan.
fn filed_under(v: &Value) -> Result<Option<NativeId>, SourceError> {
    match v.get("project") {
        None => Err(SourceError::Malformed {
            message: "missing project field".into(),
        }),
        Some(Value::Null) => Ok(None),
        Some(project) => Ok(Some(NativeId(str_at(project, "id")?.into()))),
    }
}

fn map_document(v: &Value) -> Result<Document, SourceError> {
    let (content, mut metadata) = metadata_description(optional_string(v, "content")?)?;
    strip_delivery_keys(&mut metadata);
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    Ok(Document {
        id: NativeId(str_at(v, "id")?.into()),
        title: str_at(v, "title")?.into(),
        content,
        project: filed_under(v)?,
        // Linear's `Document` carries no labels, and that is the published schema rather
        // than a gap here: the types of it that carry `labels` are `Issue`, `Project`,
        // `Team`, `Initiative` and `Organization`. Reporting none is what a source with no
        // native slot owes; standing one up beside a first-class type is what this source
        // exists not to do, and `write_document` refuses a label by name for the same
        // reason rather than dropping it.
        labels: Vec::new(),
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
}

/// Whether this document satisfies the predicates this source applies to a fetched page.
///
/// Two of them reach a page rather than the `documents(filter:)` variables, and each for a
/// reason of Linear's own. `DocumentFilter.project` is a `ProjectFilter` where
/// `IssueFilter.project` is a `NullableProjectFilter`, so only the issue side can be asked
/// for the items belonging to no project. And a Linear document carries no label at all,
/// so a query demanding one keeps nothing and a query excluding one keeps everything —
/// which is this source *applying* the predicate it declares native, over the labels the
/// document really has, rather than ignoring it.
fn document_matches(document: &Document, project: &ProjectFilter, labels: &LabelFilter) -> bool {
    let carries = |name: &String| {
        document
            .labels
            .iter()
            .any(|label| label.name.eq_ignore_ascii_case(name))
    };
    let filed = match project {
        ProjectFilter::Any => true,
        ProjectFilter::Orphans => document.project.is_none(),
        ProjectFilter::Is(id) => document.project.as_ref() == Some(id),
    };
    filed
        && (labels.any_of.is_empty() || labels.any_of.iter().any(&carries))
        && labels.all_of.iter().all(&carries)
        && !labels.none_of.iter().any(&carries)
}

fn optional<T>(
    d: &Value,
    k: &str,
    f: impl Fn(&Value) -> Result<T, SourceError>,
) -> Result<Option<T>, SourceError> {
    match d.get(k) {
        None => Err(SourceError::Malformed {
            message: format!("missing {k}"),
        }),
        Some(Value::Null) => Ok(None),
        // An item Linear no longer shows is not an item this source holds, and Linear says
        // so with `archivedAt` rather than by answering null.
        //
        // **None of Linear's three `delete` verbs removes anything.** `issueDelete`,
        // `projectDelete` and `documentDelete` move the item to the trash: observed on
        // 2026-09-04, each answered `success: true` and the item still read back by id,
        // carrying `archivedAt` and `trashed: true`. Its separate *archive* verb is a third
        // state — `archivedAt` set, `trashed` null — and Linear excludes both from every
        // connection, so `issues`, `projects` and `documents` had already stopped returning
        // them while a read by id still did.
        //
        // `archivedAt` rather than `trashed` for exactly that reason: it is the marker both
        // states share, so a read by id answers what a listing answers, and a delete means
        // what a copy's undo needs it to mean — the item this run created is gone.
        Some(value) if !matches!(value.get("archivedAt"), None | Some(Value::Null)) => Ok(None),
        Some(value) => f(value).map(Some),
    }
}
fn connection<T>(
    d: &Value,
    k: &str,
    f: impl Fn(&Value) -> Result<T, SourceError>,
) -> Result<Page<T>, SourceError> {
    let c = d.get(k).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {k} connection"),
    })?;
    let items = c
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing nodes".into(),
        })?
        .iter()
        .map(f)
        .collect::<Result<_, _>>()?;
    let next = page_next(c)?;
    Ok(Page { items, next })
}
#[derive(Clone, Copy)]
enum DependencyRoot {
    Issue,
    Project,
}
impl DependencyRoot {
    const fn item_kind(self) -> ItemKind {
        match self {
            Self::Issue => ItemKind::Task,
            Self::Project => ItemKind::Project,
        }
    }
    const fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Project => "project",
        }
    }
}
fn relation_page(
    d: &Value,
    root: DependencyRoot,
    id: &NativeId,
    direction: Direction,
) -> Result<Page<DependencyEdge>, SourceError> {
    let key = if direction == Direction::DependsOn {
        "relations"
    } else {
        "inverseRelations"
    };
    let c = d
        .get(root.as_str())
        .and_then(|v| v.get(key))
        .ok_or_else(|| SourceError::Malformed {
            message: format!("missing {key}"),
        })?;
    let nodes = c
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing relation nodes".into(),
        })?;
    let mut items = Vec::new();
    for n in nodes {
        let other = n
            .get(if direction == Direction::DependsOn {
                "relatedIssue"
            } else {
                "issue"
            })
            .or_else(|| {
                n.get(if direction == Direction::DependsOn {
                    "relatedProject"
                } else {
                    "project"
                })
            })
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| SourceError::Malformed {
                message: "missing related id".into(),
            })?;
        let (from, to) = if direction == Direction::DependsOn {
            (id.clone(), NativeId(other.into()))
        } else {
            (NativeId(other.into()), id.clone())
        };
        // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Linear publishes relation type as a string in the accepted 2026-08-24 schema; this boundary deliberately rejects every undocumented value, and real-HTTP tests prove both accepted values and rejection.
        let relation_type =
            n.get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relation type".into(),
                })?;
        // An issue relation and a project relation do not share a vocabulary. Linear
        // spells a project dependency `dependency`, where an issue's is `blocks`; the
        // write side sends exactly that pair and says why. So each root reads only its
        // own, and a value the other root would have accepted is refused here rather than
        // read as an edge this source could not have written.
        //
        // `related` is one of those values, and only an issue relation has it. Linear's
        // validator enumerates a project relation's `type` as `dependency` alone — see
        // the write side, which had `related` refused by the real API on 2026-09-04 — so
        // a project relation typed `related` is not a relation this workspace can hold.
        let kind = match (root, relation_type) {
            (DependencyRoot::Issue, "blocks") | (DependencyRoot::Project, "dependency") => {
                DependencyKind::Blocks
            }
            (DependencyRoot::Issue, "related") => DependencyKind::Related,
            _ => {
                return Err(SourceError::Malformed {
                    message: format!(
                        "invalid relation type: {relation_type} on a {} relation",
                        root.as_str()
                    ),
                });
            }
        };
        // llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
        let item_kind = root.item_kind();
        items.push(DependencyEdge {
            from: DependencyEndpoint::from_native(from, item_kind),
            to: DependencyEndpoint::from_native(to, item_kind),
            kind,
        });
    }
    let next = page_next(c)?;
    Ok(Page { items, next })
}

fn optional_str<'a>(v: &'a Value, k: &str) -> Result<Option<&'a str>, SourceError> {
    match v.get(k) {
        None => Err(SourceError::Malformed {
            message: format!("missing field {k}"),
        }),
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("field {k} is not a string"),
            }),
    }
}

/// Linear has no caller-defined fields. The source owns an unobtrusive Markdown comment at the
/// end of the long-form field, and writes it on one line with the canonical JSON inside a code
/// span: ``<!-- onetaskgraph.metadata `{…}` -->``. The code span is the one spelling Linear keeps
/// byte for byte in a description or a document — see the module documentation's ruling on what
/// Linear does to an HTML comment — and [`slot_json`] escapes the three characters that could
/// end it early.
const METADATA_PREFIX: &str = "<!-- onetaskgraph.metadata";
/// The opening of the slot this source writes.
const METADATA_OPEN_SPAN: &str = "<!-- onetaskgraph.metadata `";
/// The close of the slot this source writes.
const METADATA_CLOSE_SPAN: &str = "` -->";
/// The opening of the multi-line slot this source wrote before the code span, still read: an
/// item written then keeps its metadata. Linear normalized its JSON, so a key or a value
/// Linear rewrote reads back as Linear left it — or, for an array Linear escaped, as a
/// malformed slot naming itself — and the next write of that item writes the code span.
const METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const METADATA_CLOSE: &str = "\n-->";
/// The same close as Linear hands that multi-line slot back: it escapes a line opening
/// `-->`, so the slot reads back with a backslash before its close (observed from the real API
/// on 2026-09-14 for a document and on 2026-10-02 for an issue's description too).
const METADATA_CLOSE_ESCAPED: &str = "\n\\-->";

/// One JSON value in the encoding the slot holds it in: compact canonical JSON with `<`, `>`
/// and `` ` `` escaped as `\u003c`, `\u003e` and `\u0060`.
///
/// All three only ever occur inside a JSON string, where the escape means the same character,
/// so the value parses back exactly; and with them escaped no value can close the code span or
/// the HTML comment around it. A search phrase is not built with this: `slot_phrase` sends only
/// a value none of whose characters any encoder escapes, which this leaves as written.
fn slot_json(value: &impl serde::Serialize) -> Result<String, SourceError> {
    let encoded = serde_json::to_string(value).map_err(|error| SourceError::Malformed {
        message: error.to_string(),
    })?;
    Ok(encoded
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('`', "\\u0060"))
}

/// The slot holding exactly `metadata`, as this source writes it.
fn slot_text(metadata: &std::collections::BTreeMap<String, Value>) -> Result<String, SourceError> {
    Ok(format!(
        "{METADATA_OPEN_SPAN}{}{METADATA_CLOSE_SPAN}",
        slot_json(metadata)?
    ))
}

/// Where the trailing metadata slot of `description` is: the byte its opening marker starts
/// at, and the span of the encoded JSON inside it — or `None` when it ends in no slot.
///
/// The one place the slot is recognised, in either spelling, so what [`metadata_description`]
/// reads out and what [`metadata_slot`] keeps for a content write are the same bytes.
fn slot_bounds(description: &str) -> Result<Option<(usize, usize, usize)>, SourceError> {
    let Some(start) = description.rfind(METADATA_PREFIX) else {
        return Ok(None);
    };
    let rest = &description[start..];
    let (encoded_start, close) = if rest.starts_with(METADATA_OPEN_SPAN) {
        let encoded_start = start + METADATA_OPEN_SPAN.len();
        (
            encoded_start,
            description[encoded_start..]
                .rfind(METADATA_CLOSE_SPAN)
                .map(|at| (at, METADATA_CLOSE_SPAN.len())),
        )
    } else if rest.starts_with(METADATA_OPEN) {
        let encoded_start = start + METADATA_OPEN.len();
        (
            encoded_start,
            [METADATA_CLOSE, METADATA_CLOSE_ESCAPED]
                .into_iter()
                .filter_map(|close| {
                    description[encoded_start..]
                        .find(close)
                        .map(|at| (at, close.len()))
                })
                .min(),
        )
    } else {
        return Ok(None);
    };
    let Some((relative_end, close_len)) = close else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in Linear description".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    if !description[encoded_end + close_len..].trim().is_empty() {
        return Ok(None);
    }
    Ok(Some((start, encoded_start, encoded_end)))
}

/// The metadata slot `description` ends in, exactly as it is stored, or `None`.
fn metadata_slot(description: &str) -> Result<Option<&str>, SourceError> {
    Ok(slot_bounds(description)?.map(|(start, _, _)| &description[start..]))
}

fn metadata_description(
    description: Option<String>,
) -> Result<(Option<String>, std::collections::BTreeMap<String, Value>), SourceError> {
    let Some(description) = description else {
        return Ok((None, Default::default()));
    };
    let Some((start, encoded_start, encoded_end)) = slot_bounds(&description)? else {
        return Ok((Some(description), Default::default()));
    };
    let metadata =
        serde_json::from_str(&description[encoded_start..encoded_end]).map_err(|error| {
            SourceError::Malformed {
                message: format!(
                    "invalid canonical JSON in Linear onetaskgraph metadata slot: {error}"
                ),
            }
        })?;
    // Exactly the text above the slot less the one blank line `long_form` sets it off by, so
    // content whose own end is whitespace reads back as itself. A description edited in Linear
    // down to a single line break before the slot loses just that one.
    let above = &description[..start];
    let visible = above
        .strip_suffix("\n\n")
        .or_else(|| above.strip_suffix('\n'))
        .unwrap_or(above);
    Ok(((!visible.is_empty()).then(|| visible.to_owned()), metadata))
}

/// The narrowing that asks Linear for the issues whose description holds `"<value>"` — a
/// string `value` as any JSON encoder writes it, quotes included — or `None` when `value` holds
/// a character an encoder may write another way, and only the confirmation decides.
///
/// A candidate set rather than the answer: the phrase can sit in the visible prose, or under
/// another key, and both are kept out by the confirmation over the parsed slot that follows
/// every read. What it cannot do is miss an issue whose slot holds the value — in the code span
/// this source writes, in the multi-line slot it wrote before, or in one a person spaced or
/// re-encoded by hand — which is what makes sending it sound. So it names the value alone, never
/// the key beside it, whose spacing a slot is free to vary; and only a value of printable ASCII
/// none of whose characters any encoder escapes — not `"`, `\`, `/`, `<`, `>`, `&`, `'` or a
/// backtick — because one that is escaped would be spelled in a stored slot otherwise than here.
fn slot_phrase(value: &str) -> Option<Value> {
    let verbatim = value.chars().all(|character| {
        (character.is_ascii_graphic() && !"\"\\/<>&'`".contains(character)) || character == ' '
    });
    verbatim.then(|| json!({"description": {"contains": format!("\"{value}\"")}}))
}

/// Whether `title`/`content` satisfies `query`, case-insensitively — the contract's own rule,
/// which the engine applies for a source that does not search, so the two cannot answer one
/// workspace differently.
fn text_holds(title: &str, content: Option<&str>, query: &TextQuery) -> bool {
    let terms = query.terms.to_lowercase();
    let in_title = title.to_lowercase().contains(&terms);
    let in_content = content.is_some_and(|body| body.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}

fn optional_string(v: &Value, k: &str) -> Result<Option<String>, SourceError> {
    Ok(optional_str(v, k)?.map(Into::into))
}
fn backend_id<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    let id = str_at(value, field)?;
    (!id.is_empty())
        .then_some(id)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("field {field} is an empty backend id"),
        })
}
fn mutation_payload(data: &Value, root: MutationRoot) -> Result<&Value, SourceError> {
    let root = root.as_str();
    let payload = data.get(root).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {root}"),
    })?;
    match payload.get("success").and_then(Value::as_bool) {
        Some(true) => Ok(payload),
        Some(false) => Err(SourceError::Refused {
            message: format!("Linear reported {root} was unsuccessful"),
        }),
        None => Err(SourceError::Malformed {
            message: format!("missing boolean {root}.success"),
        }),
    }
}
fn page_next(c: &Value) -> Result<Option<Cursor>, SourceError> {
    let info = c.get("pageInfo").ok_or_else(|| SourceError::Malformed {
        message: "missing pageInfo".into(),
    })?;
    let more = info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing boolean pageInfo.hasNextPage".into(),
        })?;
    if !more {
        return Ok(None);
    }
    let cursor = str_at(info, "endCursor")?;
    Ok(Some(Cursor(cursor.into())))
}
