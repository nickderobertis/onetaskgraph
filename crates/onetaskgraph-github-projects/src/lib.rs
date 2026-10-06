//! A stateless onetaskgraph source over one GitHub Projects v2 board.
//!
//! **A board is a container of projects, not a project.** Its own `title`,
//! `shortDescription` and `readme` are never read as an item's fields and are never
//! written: nothing in this source can rename the board a user configured.
//!
//! **A project is an issue and its tasks are that issue's sub-issues.** GitHub's schema
//! decides that: `Issue` exposes `parent`, `subIssues` and `subIssuesSummary`, and
//! `DraftIssue` exposes none of them. Creating an issue needs a `repositoryId`, and a
//! board has none, so a write without [`GitHubProjectsConfig::repository`] is refused
//! naming the field — but that repository is the *fallback*, not the home of every item.
//!
//! <!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] The rule's one
//! executable source is `GitHubProjectsSource::creation_target`; this is where a reader of
//! the module meets it, and `tests/plugin.rs` drives every arm below against the loopback
//! board and asserts on `createIssue`'s own `repositoryId`, so the prose cannot outlive a
//! change to the rule. -->
//! **Which repository an issue is created in is decided by the item's own `repositories`
//! field, under one rule.** Exactly one entry names the repository the issue is created in:
//! a task issue is where a person finds the work from the repository it changes, and one
//! filed in a board's nominated repository is invisible from every other. Zero entries, or
//! two or more, name none, so a task's or a document's issue is created in the repository
//! its parent project's issue lives in — read from the board, or from this process's own
//! record of a project it created earlier in the same command — and a project's issue, or
//! a task or document written with no parent, is created in the configured `repository:`.
//! What that rule refuses, it refuses before `createIssue`, so no issue is half-created. An
//! existing issue is never moved: the update path leaves the issue where it is and records
//! the list in the metadata slot when it differs, so the read side's derivation and the
//! creation rule agree by construction.
//! <!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->
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
//! **A task's comments are its issue's comments.** They are read off `Issue.comments` and
//! written with `addComment`, `updateIssueComment` and `deleteIssueComment`, and a comment's
//! id is GitHub's own node id for the `IssueComment`. Two things GitHub decides are refused
//! rather than papered over: a board **draft** is not an issue and has no comments at all, so
//! a comment call on one is refused rather than answered with an empty page; and GitHub signs
//! every comment as the account the token belongs to, so a comment handed an author of its
//! own is refused rather than posted under another name. GitHub's comment mutations take the
//! comment's id and nothing else, so an edit or a delete first reads which issue that comment
//! is on, and a comment on some other issue is one this task does not have.
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
//! why neither can hold a caller's own prose. Setting one caller key on its own — on a task,
//! a project or a document alike — is one update of the issue body that changes that slot
//! and not one byte outside it, and it is not sent at all when the key already holds the
//! value. The link a copy records on an item it copied, `onetaskgraph.copies`, is small and
//! is kept in that same slot, written by that same update.
//!
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This public module documentation is a required user-facing description; the loopback plugin tests and shared live journey drive StatusMapping resolution, both mutations, and observed read-back together.
//! **Status.** `status_mapping` is per-instance configuration, in the shared grammar
//! [`onetaskgraph_plugin_api::StatusMapping`] documents, from a status category to an
//! option of the board's one `Status` field for a task and for a project: a bare name is
//! the option for both kinds, `null` disables the category for both, and `{task, project}`
//! names it per kind. A category the mapping does not mention keeps its shipped default for
//! both kinds; one it mentions is exactly what it configures, so a per-kind object no longer
//! gets the shipped default for the kind it leaves out. Two categories one kind would read
//! back from one option are refused as the configuration is read, ignoring case, while one
//! option may stand for different categories of the two kinds. Writes go by the kind of the
//! item written: a status that kind has no option for, or whose option the board lacks, is
//! refused before any mutation, naming the source, the kind, the category and the key
//! `status_mapping.<category>.<kind>` — there is no fallback. `done` selects its mapped
//! option and closes the issue as `COMPLETED`; `cancelled` selects its mapped option and
//! closes it as `NOT_PLANNED`, for either kind. Every open category reopens a closed issue
//! before selecting its option. Reads give a closed issue's reason precedence over its
//! option, while an open issue's option decides its category through its own kind's
//! mapping, and an option that mapping does not name reads as `unknown` under its own name.
//! The guarded [`GitHubProjectsSource::status_options`] and
//! [`GitHubProjectsSource::fields`] operations are the one path here that calls
//! `updateProjectV2Field`: GitHub replaces the whole option list, so they preserve every
//! existing option id and verify the field and item assignments immediately afterwards.
//! They ask for both kinds' options, counting a terminal category's mapped option as
//! configured because a terminal write refuses without it. No ordinary source read or
//! write calls that mutation, whose
//! `singleSelectOptions` *overwrites* a field's option set, so no addition is additive
//! and a mistake destroys every item's status. A status this board cannot represent is a
//! refusal naming the status and the instance instead.
//!
//! `unknown` has no shipped option because this source cannot preserve an open-ended
//! status word: it writes an existing board option and never
//! creates an option. An operator may map `unknown` to one existing option, in which case
//! every unknown word lands on that option and reads back as `unknown` under the option's
//! name. This differs from `local-md`, which writes and reads the original word itself.
//!
//! The shipped terminal options are exactly `done: Done` and `cancelled: Cancelled`.
//! `done` also closes the issue because GitHub derives `subIssuesSummary.completed`
//! and the board's own `Sub-issues progress` field from closed sub-issues: a plan whose
//! finished tasks were only moved to a "Done" column would read 0% complete forever.
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
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
//! | `comments` | **Supported and proven,** over the task issue's own comment connection, oldest first and paged by GitHub's own cursor; added, edited and removed through GitHub's comment mutations, paced as every other mutation is. A draft item has no comments on GitHub and is refused, and so is an author, because GitHub records the signed-in account as every comment's author. |
//! | `priority` | **Supported and proven** by an instance configured with `priority_mapping`, and declared unsupported by one without it, which reports every task's priority as `none` and sends exactly the requests it sent before priorities existed. The priority is the board's single-select `Priority` field: no value is `none`, a mapped option is its level, matched case-insensitively, and an option the mapping does not name fails the read of that task, naming the option. A write selects the mapped option, or clears the value for `none`; a board without the field or the option is refused, pointing at `sources fields`, which is the one thing that creates either. |
//! | `filter_by_priority` | **Supported and proven,** over the priority each task reads as — `none` for every task of an instance without `priority_mapping`. |
//! | `filter_by_comment_activity` | **Supported, and exact** for comments created and for comments edited at or after `commented_since`, in every repository — of any owner — the board's items live in. Applied by asking a narrower question rather than by reading the board: GitHub's issue search scoped by `project:<owner>/<number>` alone, with an `updated:>=` qualifier, names the candidates, and each candidate's own comments confirm it, so neither `ProjectV2.items` nor any issue the search did not name is read. That rests on GitHub moving an issue's `updatedAt` when a comment on it is added **or edited**, which the credentialed journey `an_edited_comment_moves_its_issue_and_is_selected_since` re-takes on every run of this lane. The search is an index that lags a write by a second or two, so a caller asking again from its last instant should overlap the two by more than that. |
//! | `orphan_tasks` | **Supported and proven.** A task issue with no `parent` is in no project. |
//! | `filter_by_label` | **Supported and proven,** over the issue's own labels. |
//! | `filter_by_status` | **Supported and proven,** over the board's `Status` option and the issue's open or closed state, through this instance's own `status_mapping` for the item's kind — a task query by the task half, a project query by the project half, `unknown` included. |
//! | `filter_by_metadata` | **Supported, and asked of GitHub.** A query naming metadata values is one board-scoped issue search with each value a quoted phrase `in:body` — GitHub's index covers the metadata comment at the end of the body, which is where caller metadata lives — and every candidate is confirmed against its own parsed metadata comment, so only an item holding that string at that key and path is returned. **A value with no letter or digit is refused** — the empty string, whitespace or punctuation alone — before any request, as a `SourceError::Refused` (wire kind `refused`) naming the value: GitHub's index holds words, so no bounded query can find such a value, and this source neither reads the whole board for it nor answers it as empty. |
//! | `filter_by_origin` | **Supported, and asked of GitHub without enumerating the board.** The union of three reads, each confirmed by an exact match against the item's own origin field: the board's field filter over the `onetaskgraph.origin` text field, the issue search for the id as a phrase in the body where a write of this release mirrors it, and this process's own writes. See *Where a read-after-write guarantee comes from* for the window the three leave. |
//! | `search_title` | **Supported, and asked of GitHub for a task,** over `Issue.title`: a task query's text is one board-scoped issue search for it as a phrase `in:title`, every candidate confirmed by the case-insensitive substring rule. GitHub matches whole words, so a task holding the text only inside a longer word is not returned — a narrowing this source declares rather than hides. **A text with no letter or digit that is not blank is refused** — `--` for one — before any request, as the same `refused` error naming the text, for the reason a metadata value like it is; a blank text is not refused, and keeps the board read it always had, confirmed by the same substring rule. A project query's text, and a document query's text when the query is scoped to no project, is that same board-scoped search for the same phrase in the same fields, refused on the same terms, every candidate confirmed by its kind and by the same substring rule, so it narrows exactly as a task's does; a document query scoped to one project sends no search, reads that project's sub-issues and confirms its text over them by the substring rule alone, so it is neither narrowed to whole words nor refused for a text with no letter or digit. A board draft is not an issue, so no text search lists one, a draft titled as a document included. |
//! | `search_content` | **Supported,** on the same terms, `in:body`, over the visible body — the trailing metadata comment is not part of what the substring rule confirms. |
//! | `task_dependencies` | **Supported and proven,** in both directions: `blockedBy` and `blocking`. |
//! | `project_dependencies` | **Supported and proven,** in both directions, over the same two connections, because a project here is an issue. |
//! | `max_page_size` | **Supported and proven.** [`MAX_PAGE_SIZE`], GitHub's own connection maximum. |
//!
//! Nothing here is unsupported. `documents` and `comments` are not predicates — they say this
//! source has documents and that its tasks have comments, both of which hold — and the three
//! facts behind the uniform `Native` on the
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
//! cost, because whichever of the reads below answers a query has already read every
//! candidate that query will return before it filters anything. Filtering those items is
//! in-process work over data already in hand.
//!
//! Third, six task predicates are asked of GitHub as a narrower question and the rest are
//! applied in process over what that question returned. A project filter has a relationship — a
//! project's tasks are that issue's sub-issues, and asking the issue for them is both cheaper
//! and exact. Comment activity is the issue search's `updated:` qualifier. A text search, and
//! a search for metadata values, is the board-scoped issue search carrying the text and each
//! value as quoted phrases; an origin is the board's own field filter over its origin field
//! beside the same search for the id. **The text search narrows, and that is this source's
//! declared semantics:** GitHub matches whole words where the substring rule this source and
//! the local Markdown source confirm with would match inside one, so an item holding the text
//! only inside a longer word is never a candidate. Every item returned does contain the text.
//! A project query's text, and a document query's scoped to no project, is that same search
//! and narrows on the same terms, its candidates confirmed by their kind as well.
//! GitHub's issue search offers no qualifier for a label set, a status column or a priority,
//! so those three are applied in process over the candidates, and a query carrying none of
//! the six narrowing predicates reads the board. Declaring one `Unsupported` would make the
//! engine compensate for work this source has already done, and declaring `projects` native
//! while ignoring the filter (which this source once did) silently returns another project's
//! tasks, because the engine trusts the declaration and applies nothing locally.
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
//! | one item, by its own id | [`graphql::ISSUE`] — `node(id:)`, carrying the field definitions of the boards it sits on and the far ends of its `blockedBy`, which is what a write of it needs — and, when that node is a board draft, [`graphql::DRAFT`] — the draft and the one board item it is | the item |
//! | one task with its first page of comments, for `task show` and a comment listing | [`graphql::ISSUE_DETAIL`] — the same `node(id:)` read with the issue's `comments` | the item and a page of its comments |
//! | several tasks with their comments, for `task show-many` | [`graphql::ISSUE_DETAILS`] — [`DETAIL_BATCH`] aliased `node(id:)` fields per request | each item and a page of its comments |
//! | the board's own id and field definitions, for a write whose item does not carry them | [`graphql::BOARD_FIELDS`] — the board's `id` and `fields`, and no `items` — or, for a create that needs the repository's id too, [`graphql::CREATION_CONTEXT`], both in one request | the board's fields |
//! | one project's tasks or documents | [`graphql::SUB_ISSUES`] — that issue's own `subIssues` | that project |
//! | which projects this board holds | [`graphql::SEARCH_ISSUES`] — an issue search scoped to the board | the board's issues, without their board items |
//! | which projects hold a text, or which documents do when no project narrows the question | [`graphql::SEARCH_ISSUES`] — the board-scoped search with the text as one quoted phrase, `in:title`, `in:body` or both, as a task's text is sent — walked to its end in pages of twenty | the issues that match |
//! | which tasks were commented on since an instant | [`graphql::SEARCH_ISSUES`] — the same board-scoped search with an `updated:>=` qualifier — then [`graphql::ISSUE_COMMENTS`] for each candidate it names | the issues updated since, and their comments |
//! | which tasks hold a text, or a metadata value | [`graphql::SEARCH_ISSUES`] — the board-scoped search with the text and each value as quoted phrases, `in:title`, `in:body` or both, and an `updated:>=` qualifier too when comment activity is asked for — in pages of twenty, only as many as the caller's rows need | the issues that match |
//! | which tasks were copied from one origin | [`graphql::ORIGIN_LOOKUP`] — the board's own `items` under its field filter on the origin field, and the same board-scoped search for the id `in:body`, in one request, each paged at three | the carriers of that origin, which is one item |
//! | every task, every document, every label, when nothing above narrows the question | [`graphql::BOARD`] — the board's own `items` — **and** [`graphql::SEARCH_ISSUES`], because neither enumeration of a board is complete alone; see [`GitHubProjectsSource::board`] | the board, twice over |
//! | which board item one issue is, past the page that came with it | [`graphql::ISSUE_BOARD_ITEMS`] — that issue's own `projectItems` | one issue's memberships |
//!
//! The following standalone-ticket requests are pinned by the real CLI fixture journeys
//! `follow_up_writes_resolve_each_item_once_and_batch_the_copy_fields` and
//! `a_batched_detail_read_costs_one_request_and_one_point_per_detail_batch`, as request count
//! equal to declared points equal to the row. They include the origin lookup and the
//! field/repository discovery a create needs. A bound re-copy changes status, priority,
//! content and metadata; comment recount means a subsequent detail read. Each request here
//! costs one declared point. A membership beyond the embedded page can additionally require
//! the one-point membership recovery described above. A bound re-copy of a task filed under a
//! project adds one read, the engine confirming that project's link by its own id once per
//! command; and the same-source far ends a write newly names — those that do not already block
//! the item, whose own read answered for them — are read together by their own ids,
//! [`DETAIL_BATCH`] to one [`graphql::ISSUE_DETAILS`] request, each new edge then one
//! [`graphql::ADD_BLOCKED_BY`]. Both additions are rows of the table below, pinned by
//! `a_bound_recopy_adds_one_project_read_and_batches_the_dependencies_it_newly_names`.
//!
//! **[`DETAIL_BATCH`] is 24**: the largest batch of [`graphql::ISSUE_DETAILS`] the node-count
//! model prices at one point. Each aliased item is six of GitHub's aggregate, so 24 are 144,
//! which rounds to one point, and 25 are 150, which rounds to two; `tests/point_cost.rs`
//! holds both halves.
//!
//! **An existing item is written body last.** A bound re-copy and a `task update` send its
//! board fields first — the `Status` option and the `Priority` together, in one request — then
//! its parent and its `blockedBy`, and its title, body and state in one `updateIssue` last.
//! GitHub runs no two requests as one, and runs a document's mutation fields in order without
//! undoing an earlier field when a later one fails, so that order is what makes a write
//! refused part-way leave the item's body, and every metadata key in it, exactly as it stood;
//! the one piece of metadata written before the body, an origin a copy re-points, is put back
//! when a later write is refused — and when putting it back is refused too, the write's own
//! refusal names that key, what it now holds and what it held. `crates/onetaskgraph/tests/e2e/write_order.rs` refuses each
//! of those writes in turn, whole and as one aliased field failing after the one before it.
//!
//! **Two facts about GitHub the write rows rest on, each read off GitHub's published schema
//! artifact <https://docs.github.com/public/fpt/schema.docs.graphql> on 2026-10-01 and pinned
//! in `tests/fixtures/schema.graphql`, and the first then put to GitHub itself:**
//!
//! - **A board is accepted at creation but its item is not answered, so a create still files
//!   the issue itself: a new copy is 5 requests, and 4 with `--create`.**
//!   `CreateIssueInput.projectV2Ids: [ID!]` is declared there — "An array of Node IDs for
//!   Projects V2 associated with this issue", `@possibleTypes(concreteTypes: ["ProjectV2"])`.
//!   The credentialed journey `real_projects_v2_contract_writes_and_leaves_no_residue` was run
//!   against a real board on 2026-10-01 with a create sending the board there and reading the
//!   item off the payload's `Issue.projectItems`: every one of its four creates answered with
//!   no item on the board, so each went on to [`graphql::ADD_TO_BOARD`], and the fourth was
//!   refused "Content already exists in this project" — GitHub had filed the issue after
//!   answering, and refuses a second filing rather than answering with the item it holds. A
//!   create therefore sends no `projectV2Ids` and files the issue with
//!   `addProjectV2ItemById`, the one call whose answer names the board item. The saving that is
//!   real is the read before it: the board's fields and the repository's id together, in
//!   [`graphql::CREATION_CONTEXT`], at the point the repository is known.
//! - **A comment still reads its target first, so a comment is 2 requests.**
//!   `AddCommentInput.subjectId: ID!` is declared there with
//!   `@possibleTypes(concreteTypes: ["Issue", "PullRequest"], abstractType:
//!   "IssueOrPullRequest")`. A board draft is no such subject and would be refused, but a
//!   project's issue, a document's issue, an issue on no board of this source and a pull
//!   request all are: GitHub writes the comment, so there is no refusal to map into "that is
//!   not a task of this board". [`graphql::ISSUE`] before [`graphql::ADD_COMMENT`] is what
//!   refuses those by name.
//!
//! | Verb | Requests / points | Documents |
//! | --- | --- | --- |
//! | new copy | 5 | ORIGIN_LOOKUP, CREATION_CONTEXT (the board's fields and the repository's id together), CREATE_ISSUE, ADD_TO_BOARD, UPDATE_FIELDS |
//! | copy --create | 4 | CREATION_CONTEXT, CREATE_ISSUE, ADD_TO_BOARD, UPDATE_FIELDS: the new copy without its ORIGIN_LOOKUP |
//! | bound copy | 3 | ISSUE (with the board's fields and the issue's `blockedBy`, so no BOARD_FIELDS or ISSUE_DEPENDENCIES), UPDATE_FIELDS, then UPDATE_ISSUE last |
//! | bound copy, filed under a project | 4 | the bound copy's three, and one ISSUE of the destination project its link names, read once per command |
//! | bound copy, newly naming n dependencies | + ceil(n / DETAIL_BATCH) + n | ISSUE_DETAILS for the far ends that do not already block the item, DETAIL_BATCH (24) to a request (one alone is ISSUE), then one ADD_BLOCKED_BY each; a far end already blocking it is answered by its own read and costs nothing |
//! | comment | 2 | ISSUE, ADD_COMMENT: the target is read first, because GitHub accepts a comment on any issue or pull request (see below) |
//! | detail | 1 | ISSUE_DETAIL: the item and its first page of comments, for `task show` and `task comment list`; `--no-comments` is ISSUE alone |
//! | batched detail | ceil(n / DETAIL_BATCH) | ISSUE_DETAILS: `task show-many` of `n` items, DETAIL_BATCH (24) at a time, comments included or not |
//! | recount | 1 | ISSUE_DETAIL |
//! | status | 2 | ISSUE, UPDATE_FIELD; a terminal status additionally updates issue state |
//! | priority | 2 | ISSUE, UPDATE_FIELD or CLEAR_FIELD, with stored priority in the mutation response |
//! | content | 2 | ISSUE, UPDATE_ISSUE |
//! | metadata | 2 | ISSUE, UPDATE_ISSUE |
//! | update | 3 | `task update` naming any of title, body, metadata, status and priority — all five included: ISSUE, UPDATE_FIELDS (the status option and the priority together), UPDATE_ISSUE (title, body with its metadata slot, and state) last |
//! | record only | 1 | ISSUE |
//!
//! <!-- github-search-paging:start -->
//! Board-scoped text, metadata, project-name and comment-activity searches send every
//! page at `first = 20` (SEARCH_PAGE_SIZE), the SEARCH_ISSUES document's one-point
//! ceiling. A later page is sent only when `hasNextPage` is true and the caller still
//! needs rows. A page is never resized to the rows still needed: GitHub orders one
//! search differently at different page sizes, so one fixed size makes a paged walk
//! send exactly the requests one whole read sends, and the answer's order is the order
//! those pages arrive in. A page below twenty would cost the same one point, and GitHub
//! prices this document by rows, so twenty-row pages cost per row what 100-row pages do.
//! Project-name lookup continues until an exact match or exhaustion. A task limit bounds
//! returned and fetched pages: a limit is sliced from the pages it needs, and local
//! confirmation can require more candidates than matching rows. Walking all pages
//! returns the whole answer. The opaque version-4 source cursor carries GitHub's page
//! cursor and how far into that page the last answer stopped, and resumes in the same
//! process or a new one, without duplicates or gaps. It carries no rows: one process
//! sends each page's search once, and a new process re-reads only the page it resumes
//! in, then sends a further page once, never as a re-read, only when its limit still
//! needs rows. Every request either walk sends is the one a whole read sends for that page. Own writes replace stale index
//! copies and complete missing rows at exhaustion. Cache entries are whole GitHub pages,
//! so a small answer cannot truncate a wider question. Origin pages remain three; whole-board sizing is unchanged.
//! Read-after-write is a per-process guarantee. A cursor resumed in a new process is
//! not required to include the original process's writes still omitted by the index.
//! <!-- github-search-paging:end -->
//!
//! The board half of an issue — its board item's id, its `Status` option and this
//! source's origin text field — rides along on `Issue.projectItems` in the first three, so
//! an item reached any of those ways resolves through the same
//! [`GitHubProjectsSource::resolve`] the board walk uses and reports the same title, the
//! same status, the same labels and the same qualified id. That connection comes back a
//! *page* at a time, at `BOARD_ITEMS_PAGE_SIZE`, so the entry for this board is looked for
//! on the page in hand and — only if that page reports more of the connection — in the
//! last row's read of that one issue's memberships, resumed from the page's own cursor and
//! walked to exhaustion. An issue with no entry for *this* board is not this source's to
//! report, which is what keeps an id naming another repository's issue from being answered
//! as an item of this board; and because the page is where the search starts rather than
//! where it ends, that answer is one about a connection read to exhaustion and never about
//! an unread page. Nothing costs the extra read but an issue on more boards than a page
//! holds: an issue this board really does not hold reports no next page, so its
//! memberships are already exhausted where they arrived.
//!
//! **No document here selects the board's own `Labels` field, and nothing is lost by
//! that.** An item's labels are read from its content alone, wherever that content is
//! reached: the three documents above select `Issue.labels` on the fragment, and
//! [`graphql::BOARD`] selects the same connection on the `... on Issue` arm of its
//! `content`. A board's `Labels` field is not one anybody fills in: it is a built-in
//! `ProjectV2FieldType`, it is absent from `ProjectV2CustomFieldType` so no project can
//! create one, and `ProjectV2FieldValue` — the whole of what
//! `updateProjectV2ItemFieldValue` accepts — offers no way to write one. So GitHub derives
//! it from the content, for every content type it exists on, and there is nothing it can
//! hold that the content does not already say: for an `Issue` it *is* that issue's own
//! labels, so selecting it beside them unions a set with itself.
//!
//! **A draft loses nothing by that either**, which is the reasoning this paragraph once had
//! backwards. `DraftIssue` exposes no `labels` field, and by the three schema facts above
//! it cannot carry a board `Labels` value to be derived from one — so a draft has nothing
//! to select *and nothing to lose*, and reports no labels at all. A `PullRequest` item is
//! discarded by [`GitHubProjectsSource::resolve`] before labels are read. Both halves are
//! held to that by tests in `tests/plugin.rs`: the four ways an item is reached report one
//! label set, and that set is the fixture issue's own, by
//! `an_item_reports_the_same_labels_title_status_and_id_however_it_is_reached`; and a board
//! item whose content is a draft reports an empty set, by
//! `a_board_item_whose_content_is_a_draft_reports_no_labels_at_all`. The absence of the
//! selection is held over [`graphql::DOCUMENTS`] by
//! `no_document_selects_the_boards_own_labels_field`.
//!
//! The whole-board row is still the board's own item connection, and deliberately: a
//! **draft** board item is not an issue, so no search can list one, and the reads that have
//! to answer for the whole board are the ones whose cost is the board's size anyway.
//!
//! **A question about one item this source already names by id never lists the board.**
//! Whether that item is on this board, and what its board fields are, is answered by reading
//! that item — its own `Issue.projectItems`, walked to exhaustion by
//! [`GitHubProjectsSource::resolve_issue`], or a draft's own board item — and never by
//! looking for it in [`graphql::BOARD`]'s `items` or in a listing this command already
//! holds. That covers a write's destination, the project a new item is filed under, a
//! same-source far end a dependency names, a status write, the dependency slot a draft keeps,
//! and the delete that takes back an item a copy made. What such a write needs of the board
//! and the item does not carry — the board's id, the `Status` and origin field definitions —
//! comes from [`graphql::BOARD_FIELDS`], which reads no item at all. The reason is evidence,
//! not economy alone: `ProjectV2.items` is a projection that lags the membership GitHub
//! itself reports — an issue added with `addProjectV2ItemById` can be missing from it for
//! minutes. Scanning this host's 842-item board has refused a document copy and an update
//! even though the items' own reads named that board. A scan there gives the wrong answer
//! as well as paying for every page. So a `board.items` lookup does not belong on any of
//! those paths.
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
//! `BOARD_ITEMS_PAGE_SIZE` on the page of an issue's board memberships a read carries.
//! `$nestedFirst` is spent twice down one path of a board read, so that constant is
//! effectively squared there, which is why it is the one the limit is most sensitive to.
//! `BOARD_ITEMS_PAGE_SIZE` is small for a reason of its own, recorded beside it: what a
//! page of memberships misses is recovered by one further read rather than refused, so it
//! buys a bound every read pays for at the price of a request only a multi-board issue
//! pays.
//!
//! **`nodeCount` and `cost` are two numbers against two limits, and both are computed
//! offline here — per document, one document at a time.** `nodeCount` is the one above: the
//! most nodes one query may return, checked per query and bounded by [`NODE_COUNT_LIMIT`].
//! `cost` is rate-limit points, metered per hour across everything one credential does; it
//! is what the two limiters [`Limiter`] tells apart meter, and a document under
//! [`NODE_COUNT_LIMIT`] still says nothing about its price. [`worst_case_point_cost`] is
//! that second number, and `tests/point_cost.rs` pins every document in
//! [`graphql::DOCUMENTS`] at what it costs — there being no per-call point ceiling to hold
//! one under, the pin itself is the check. The credentialed lane reconciles both figures
//! against GitHub's own, off a probe it already sends.
//!
//! **What is pinned that way is a per-document price and never a session's.** The record in
//! `session-cost.md` measures the two quantities a whole session can be counted in offline —
//! **requests** and **worst-case nodes** — and neither is points. What one whole session
//! consumes of the hourly point allowance is observable only from a credentialed run's own
//! `x-ratelimit-*` headers, which is what [`accounting`] fills its per-budget figures from
//! and what `tests/live.rs` prints at the end of every run.
//!
//! [node-limits]: https://docs.github.com/en/graphql/overview/rate-limits-and-node-limits-for-the-graphql-api
//!
//! **Where a read-after-write guarantee comes from, since neither of GitHub's two
//! enumerations of a board can supply one alone.** Resolving a node id is strongly
//! consistent, so a read by id and a project's own sub-issues are already current. The
//! other two are not, and they are behind by different amounts and in different directions:
//!
//! - GitHub's **issue search** is an index and answers a write made moments ago with the
//!   value from before it — usually for a second or two.
//! - **`ProjectV2.items`** is a projection GitHub rebuilds behind the write, and an item put
//!   on a board with `addProjectV2ItemById` can be **absent** from it — not present with its
//!   content withheld, absent, with the connection walked to its own `hasNextPage: false` —
//!   for *minutes*, while `Issue.projectItems` names the same membership at once.
//!
//! That second one is a measurement rather than a caution. This repository's own
//! credentialed journey writes a project and waits for the board to report it, then writes a
//! task and waits for the same thing seconds later on the same board: the project wait is
//! answered through the search and converged in two or three attempts in each of three runs,
//! and the task wait is answered through `ProjectV2.items` and converged in none of them
//! inside thirty. Separately, an item added to a second and larger board was read back by
//! `Issue.projectItems` on that board's own id while every one of that connection's nine
//! pages, walked to exhaustion nine minutes after the add, did not name it. Reading a board
//! through the lagging one alone is what had a board read deny an issue that had certainly
//! landed on it.
//!
//! So [`GitHubProjectsSource::board`] is the **union** of both — each search result still
//! admitted only on this board's own strongly-consistent `Issue.projectItems`, and neither
//! enumeration dropped, because only `ProjectV2.items` lists a board draft and only the
//! search reports what the projection is behind on. What closes the last
//! gap, the one where both are behind, is [`GitHubProjectsSource::created`]: every read this
//! source answers is completed with what this process itself wrote, so an item created
//! seconds ago is reported whether or not GitHub has caught up. Nothing else is remembered,
//! nothing is written down, and the record dies with the process. **A wait that has to
//! observe GitHub's own data cannot be answered from that record** — which is why the
//! credentialed journey asks through a source built afresh, and why the union above rather
//! than a longer wait is what makes such a wait converge.
//!
//! **A narrowed read is the same bargain, stated for each of the three predicates it
//! answers.** A read carrying a text, metadata or origin predicate asks GitHub's index rather
//! than walking the board, and every such answer is completed with what this process wrote —
//! its [`created`](GitHubProjectsSource::created) record and every existing item it wrote,
//! each filtered by the same predicates as the rest — so an item this command wrote a moment
//! ago is returned by a query that matches it whether or not the index has caught up. An item
//! a caller holds the id of is read by that id, with `node(id:)`, which is strongly
//! consistent. What is left is stated rather than papered over:
//!
//! | Read | Finds | Behind by |
//! | --- | --- | --- |
//! | text, metadata | the issue search for the phrases | what another process wrote in the last second or two, until GitHub indexes it |
//! | origin, first read | the board's field filter over the origin field — every carrier, whichever release wrote it | what `ProjectV2.items` is behind on, which the measurements above put in minutes |
//! | origin, second read | the issue search for the id in the body, where a write of this release mirrors it | a second or two, as any search |
//! | origin, third read | this process's own writes | nothing |
//!
//! So an origin carrier another process added within the last second or two, before either
//! index has it, can be missing from an origin query, and one written by the release before
//! this one — its origin in the field alone — can be missing for as long as the board's own
//! item connection is behind on it. A copy that must not duplicate its own earlier write
//! relies on the link it records, not on either index. **A board draft is not an issue**, so
//! a draft is never returned by a text, metadata or origin query, whatever it holds: no search
//! lists one, the origin lookup drops any the board's own field filter names, and one this
//! process wrote is not added back either.
//!
//! **The origin lives in the board field, and the body holds a mirror of it.** A write that
//! carries an origin writes it to the `onetaskgraph.origin` text field and also into the
//! body's metadata slot, so the issue search can find it in seconds. The field is
//! authoritative: this source reads an item's origin from the field alone, so a slot that
//! disagrees with it, or holds one where the field holds none, is never read as a second
//! origin — and the release before this one reads the slot, drops that key's copy for the
//! field's, and sees the same one origin.
//!
//! Filtering happens before paging, so a page of a filtered result is a page of the
//! survivors rather than the survivors of a page. Label matching and the substring rule a
//! text candidate is confirmed by answer the same question the same way the local Markdown
//! source's do; which candidates a text search has to confirm is GitHub's word match, which
//! is the one place the two sources can answer the same text differently.
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
//! **A live session refuses to start unless the account can afford it.** Before it does any
//! of the work it exists to do, the journey makes one request — `GET /rate_limit`, which
//! GitHub documents as not counting against the REST rate limit and which answers both of
//! its budgets at once — and starts only if, for each of them, what remains minus this
//! session's estimated cost is still at least
//! `onetaskgraph_live::RETAINED_BUFFER` — twenty per cent — of that budget's whole
//! allowance. A session that cannot **declines**: it did not run, so it is
//! neither a pass nor a failing assertion, and it says which budget was short, that budget's
//! limit, what remained, the estimate, the buffer and when it resets — then stops, without
//! waiting for the budget to come back. The estimate is derived offline from
//! `tests/fixtures/session-cost.txt` and a cost model stated in `tests/journey/budget.rs`,
//! which is also where the published rule that model rests on is cited; the accounting
//! above records the gate's own read like any other request, and
//! [`accounting::Session::report`] prints the estimate beside what the session really spent.
//!
//! **GitHub is the authority on both of its own numbers, and the credentialed lane goes and
//! asks it.** Everything above computes `nodeCount` and `cost` offline from a document's own
//! text, which is what lets it run on every platform and on a pull request from a fork with
//! no credential — and that is what actually stops a regression merging. But an offline
//! arithmetic can only ever agree with itself: if GitHub changes its rules, this workspace
//! goes on computing the old answer and nothing notices. So `tests/live.rs` reconciles them.
//! GitHub's schema exposes `rateLimit(dryRun: true)`, whose `nodeCount` is *"the maximum
//! number of nodes this query may return"* and whose `cost` is what that document would
//! spend, both for a document **without executing it**, and the lane asks it for every query
//! document this source sends, under the largest bindings this source sends, and fails when
//! GitHub's figure and [`worst_case_node_count`] or [`worst_case_point_cost`] disagree. A
//! mutation is skipped, because `rateLimit` is a field of `Query` and cannot be asked about
//! one; the offline pins still cover it. It records what those calls reported about the
//! account's own allowance, because whether asking is free is a thing to observe rather than
//! to assume. Two quantities, not one: [`NODE_COUNT_LIMIT`] bounds `nodeCount` per query,
//! and `cost` is metered against an hourly allowance the accounting above reads off a
//! credentialed run's own response headers.
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
    Capabilities, Comment, CommentBody, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind,
    DependencySupport, Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label,
    LabelFilter, Location, MetadataKey, Metering, NativeId, NewComment, Page, PageRequest,
    Priority, Project, ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, StatusMapping, Support, Task, TaskDetailRead,
    TaskQuery, TaskRef, TaskSource, TaskUpdate, TaskUpdateOutcome, TextFields, TextQuery,
    UnmappedStatus, UpdatedField, WriteSupport,
};
use reqwest::{Client, StatusCode, Url};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod accounting;

use accounting::Accounting;

/// The registry name for this plugin.
pub const KIND: &str = "github-projects";
/// GitHub's maximum connection page size.
pub const MAX_PAGE_SIZE: u32 = 100;
/// Every page of a board-scoped narrowing search: 20 rows, one point of SEARCH_ISSUES, the
/// most one point buys. GitHub prices that document by rows, so pages of 20 cost what pages
/// of 100 cost per row, and a page of fewer than 20 costs the same one point.
pub const SEARCH_PAGE_SIZE: u32 = 20;
/// How many items one [`graphql::ISSUE_DETAILS`] request reads, each with the first page of
/// its comments: the largest batch the node-count model prices at one point.
///
/// Each aliased item is resolved once, and what GitHub charges for it is the connections
/// under it — its labels, its page of board memberships, the field values of each of those
/// three memberships, and its comments: six requests' worth of the aggregate GitHub divides
/// by a hundred and rounds. Twenty-four items come to 144, which rounds to one point;
/// twenty-five come to 150, which rounds to two. `tests/point_cost.rs` prices the document at
/// one point and fails if one item more would still be priced at one.
pub const DETAIL_BATCH: usize = 24;

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
/// everything the credential does — two numbers against two limits, and this constant
/// bounds only the first. The second is computed offline too, per document:
/// [`worst_case_point_cost`], pinned for every document in [`graphql::DOCUMENTS`] by
/// `tests/point_cost.rs`, and reconciled against GitHub's own `cost` by the credentialed
/// lane. There is no constant like this one to hold a price under, because points are an
/// hourly allowance rather than a per-call bound.
///
/// Neither is a session's price. What `session-cost.md` records of a whole session is its
/// **requests** and its **worst-case nodes**; what a whole session spends in points is
/// reported only by a credentialed run's own `x-ratelimit-*` headers, through
/// [`accounting`]. The module section on the three ways this source reaches an item says how
/// the count is arrived at, and which of the page sizes below decide it.
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
/// `Issue.projectItems`, and only the entry for *this* board is read. This connection sits
/// under a page of issues, so every point of it multiplies through the whole document and
/// is paid for whether or not any issue is on a second board — which is why it is
/// deliberately far smaller than [`NESTED_PAGE_SIZE`].
///
/// **Three, because what a page misses is now recovered rather than refused**, and the
/// recovery is what the value is chosen against. An issue whose entry for this board sits
/// past this page costs one further request — [`graphql::ISSUE_BOARD_ITEMS`], resumed from
/// that page's own cursor — so the value trades a bound every read pays for a request only
/// a multi-board issue pays. At one, a deployment whose issues commonly sit on two or more
/// boards would pay that request *per issue*, which is order N against the one page per
/// hundred issues a read costs today. At three it is only reached by an issue on four or
/// more boards at once, which keeps the recovery path exceptional rather than routine for
/// a plausible deployment.
const BOARD_ITEMS_PAGE_SIZE: u32 = 3;
/// How many carriers of one copy origin one page of [`graphql::ORIGIN_LOOKUP`] asks each of
/// its two connections for.
///
/// An origin names one item, so the answer an origin lookup expects is one carrier, and a
/// second is a duplicate a copy already takes the first of. Both connections are walked to
/// exhaustion whatever this is, so it decides how many requests an unusual answer costs and
/// never what the answer is. It is small because every point of it is paid on every lookup,
/// and a copy makes one lookup per item it has no link for: at three, ten lookups cost fewer
/// worst-case nodes than the one whole-board read they replaced.
const ORIGIN_PAGE_SIZE: u32 = 3;

pub use github_graphql_node_count::{NodeCountError, Variables};

/// The largest value this source can bind to each page-size variable its documents name.
///
/// Every `first:` in [`graphql`] reads one of these four, and each is capped at the
/// constant above it wherever a caller's own limit could reach it — `$first` at
/// [`MAX_PAGE_SIZE`], `$nestedFirst` at `NESTED_PAGE_SIZE`, `$boardItems` at
/// `BOARD_ITEMS_PAGE_SIZE`, `$originFirst` at `ORIGIN_PAGE_SIZE`. So this is the worst case a caller can drive this source to,
/// not one configuration of it, which is what makes a bound computed under it a bound on
/// every read.
pub fn largest_page_sizes() -> Variables {
    Variables::from([
        ("first".to_owned(), MAX_PAGE_SIZE),
        ("nestedFirst".to_owned(), NESTED_PAGE_SIZE),
        ("boardItems".to_owned(), BOARD_ITEMS_PAGE_SIZE),
        ("originFirst".to_owned(), ORIGIN_PAGE_SIZE),
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

/// The most rate-limit points one call of `document` could spend, by GitHub's published
/// rules.
///
/// Computed offline from the document's own text under [`largest_page_sizes`] — no
/// network, no credential and no schema — by
/// [`github_graphql_node_count::point_cost`], which is where the rules themselves live.
/// This is `cost`, metered **per hour** against the allowance one credential shares across
/// everything it does; it is not `nodeCount`, which is [`worst_case_node_count`] and is
/// bounded per query by [`NODE_COUNT_LIMIT`]. There is no per-call ceiling to hold this
/// under, so what `tests/point_cost.rs` does with it is pin every document in
/// [`graphql::DOCUMENTS`] at what it costs, and the credentialed lane reconciles those
/// figures against GitHub's own reported `cost`.
///
/// # Errors
///
/// Returns the calculation's own [`NodeCountError`] when `document` does not parse, holds
/// no single operation, or binds a page size this source does not name — each of which is
/// a defect in the document rather than a number.
pub fn worst_case_point_cost(document: &str) -> Result<u64, NodeCountError> {
    github_graphql_node_count::point_cost(document, &largest_page_sizes())
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
/// independently. [`STATUS_OPTIONS_UPDATE`] is the sole document that may rewrite a board
/// field, and its guarded caller always supplies the complete existing option set with ids.
pub mod graphql {
    /// The board half of one item: the field values every document here reads it from.
    ///
    /// A macro for the same reason [`board_issue!`] below is one, a level further in. This
    /// selection is needed by that fragment, by [`BOARD`] under the board's own `items`,
    /// and by [`ISSUE_BOARD_ITEMS`] under a membership walk — and all three have to produce
    /// *the same value*, because
    /// [`GitHubProjectsSource::resolve`](super::GitHubProjectsSource) reads them through
    /// one path. Three spellings of it is what would drift, so there is one.
    ///
    /// The `Status` option and this source's own origin text field are the whole of it. It
    /// selects no `ProjectV2ItemFieldLabelValue`: GitHub derives that field from the item's
    /// content, so it holds nothing the content's own `labels` do not already say, and it
    /// would sit a label connection two page sizes deep.
    macro_rules! board_item_values {
        () => {
            r#"fieldValues(first:$nestedFirst){nodes{
          ... on ProjectV2ItemFieldSingleSelectValue{name field{
            ... on ProjectV2SingleSelectField{id name options{id name}}
          }}
          ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{id name}}}
        }pageInfo{hasNextPage}}"#
        };
    }

    /// Everything this source reads about one issue, wherever it reaches that issue.
    ///
    /// A macro rather than a constant so the three documents below can `concat!` it: one
    /// spelling of these fields is what makes an issue read through the board-scoped
    /// search, through its own node id, and through its project's sub-issue relationship
    /// resolve to *the same* item, which is the whole of what
    /// [`GitHubProjectsSource::resolve_issue`](super::GitHubProjectsSource) relies on.
    ///
    /// `projectItems` is what carries the board half of an issue: the board item's own id
    /// and the [`board_item_values!`] above — the `Status` option and this source's origin
    /// text field — that a `ProjectV2.items` read used to carry. It is asked for on the
    /// issue rather than on the board, which is what makes the cost of a read proportional
    /// to what was asked for instead of to the board's size.
    ///
    /// It carries a *page* of that connection, at `BOARD_ITEMS_PAGE_SIZE`, and its
    /// `endCursor` is what [`ISSUE_BOARD_ITEMS`] resumes from when this board's entry is
    /// not on that page: a page here is where the search for the entry starts rather than
    /// where it ends.
    ///
    /// It does **not** select the board's `Labels` field value, and that is the whole of
    /// what keeps the three documents below under [`NODE_COUNT_LIMIT`](super::NODE_COUNT_LIMIT):
    /// a label connection there sits under `fieldValues` under `projectItems` under a page
    /// of issues, spending `$nestedFirst` twice down one path, and took
    /// [`SEARCH_ISSUES`] and [`SUB_ISSUES`] to 2,556,100 nodes against a limit of 500,000.
    /// No label is lost — this is a fragment `on Issue`, whose own `labels` are selected
    /// above, and that connection is where every label this source reports comes from. No
    /// document in this module selects the board field any longer, [`BOARD`] included; the
    /// module documentation records why nothing it could have held is lost.
    macro_rules! board_issue {
        () => {
            concat!(
                r#" fragment BoardIssue on Issue{__typename id number title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total}
      labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}
      projectItems(first:$boardItems){nodes{id project{id number}
        "#,
                board_item_values!(),
                r#"}pageInfo{hasNextPage endCursor}}}"#
            )
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

    /// What a dependency read selects of each far end: enough to say which kind of item it
    /// is, its body included for the kind marker.
    macro_rules! related_issue {
        () => {
            " fragment Related on Issue{id title body parent{id} subIssuesSummary{total}}"
        };
    }

    /// One issue by its own node id, which is what a qualified id names here — with what a
    /// write of it needs and the issue does not carry in `board_issue!`: the field
    /// definitions of the boards it sits on, and the far ends of its `blockedBy`.
    ///
    /// Strongly consistent, unlike the search above: GitHub's issue search is an index and
    /// answers a write made moments ago with the value from before it, and resolving a node
    /// id does not.
    ///
    /// **Why those two ride here and not on the fragment.** A copy or an update of an item
    /// reads it by its own id, and with them that one read answers everything the write
    /// needs: which option ids the board's `Status` and `Priority` fields hold — so no
    /// [`BOARD_FIELDS`] — and which issues block it, with each one's kind — so no
    /// [`ISSUE_DEPENDENCIES`]. On `board_issue!` they would sit under the hundred-issue
    /// pages of [`SEARCH_ISSUES`] and [`SUB_ISSUES`], multiplying both documents' price. Here
    /// they sit under one item, and this read is still one point.
    pub const ISSUE: &str = concat!(
        r#"query($id:ID!,$first:Int!,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      node(id:$id){__typename ...BoardIssue ... on Issue{
        boards:projectItems(first:$boardItems){nodes{project{id number fields(first:$nestedFirst){nodes{
          ... on ProjectV2SingleSelectField{__typename id name options{id name}}
          ... on ProjectV2Field{__typename id name}
        }pageInfo{hasNextPage}}}}}
        blockedBy(first:$first){nodes{...Related}pageInfo{hasNextPage endCursor}}
      }}
    }"#,
        board_issue!(),
        related_issue!()
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

    /// What a read of the board's own `items` selects of each item's content.
    ///
    /// A macro for the reason [`board_item_values!`] is one: [`BOARD`] and [`ORIGIN_LOOKUP`]
    /// both walk `ProjectV2.items` and hand each item to one resolver, so they select its
    /// content by one spelling.
    macro_rules! board_item_content {
        () => {
            r#" content{
        ... on Issue{__typename id number title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total} labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        ... on PullRequest{__typename id}
        ... on DraftIssue{__typename id title body createdAt updatedAt}
      }"#
        };
    }

    /// Reads the board's fields and one page of its items.
    pub const BOARD: &str = concat!(
        r#"query($owner:String!,$number:Int!,$first:Int!,$after:String,$nestedFirst:Int!,$duplicates:Boolean!){
      owner:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){...Board}}
      }
    } fragment Board on ProjectV2 { id title
      fields(first:$nestedFirst){nodes{
        ... on ProjectV2SingleSelectField{__typename id name options{id name}}
        ... on ProjectV2Field{__typename id name}
      }pageInfo{hasNextPage}}
      items(first:$first,after:$after){nodes{id "#,
        board_item_values!(),
        board_item_content!(),
        r#"} pageInfo{hasNextPage endCursor}}
    }"#
    );

    /// Every carrier of one copy origin, by two reads in one request, and nothing else of
    /// the board.
    ///
    /// **`originItems`** is the board's own items narrowed by its own field filter —
    /// `ProjectV2.items(query:)`, which GitHub's schema declares as "Search query for
    /// filtering items" — to those whose `onetaskgraph.origin` text field holds the
    /// qualified id, quoted. It reads the field every carrier already holds, whichever release
    /// wrote it, and matches it exactly: measured on 2026-09-29 against a 394-item board,
    /// the quoted, the unquoted and the bare-value spellings each returned exactly the one
    /// carrier and a prefix of the value returned none. It is `ProjectV2.items`, so it lags a
    /// fresh `addProjectV2ItemById` the way that connection does.
    ///
    /// **`search`** is the board-scoped issue search for the same id as a quoted phrase in
    /// the body, which is where this source mirrors the origin into its metadata slot. GitHub
    /// indexes that comment, and the index catches up with a write in a second or two rather
    /// than in minutes, so it finds a carrier another process wrote that the first read is
    /// still behind on.
    ///
    /// Each connection pages at `$originFirst`, its own small size — see `ORIGIN_PAGE_SIZE`
    /// — and resumes from its own cursor; a connection already walked to its end is resumed
    /// from its last cursor, which answers an empty page. Every candidate either read returns
    /// is confirmed against its own origin field before it is reported, so a token match of
    /// the search or anything else the filter admits never is.
    ///
    /// The root is aliased `originItems` rather than `owner`, so nothing counting the board's
    /// own whole reads counts this one among them.
    pub const ORIGIN_LOOKUP: &str = concat!(
        r#"query($owner:String!,$number:Int!,$filter:String!,$search:String!,$type:SearchType!,$originFirst:Int!,$itemsAfter:String,$searchAfter:String,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      originItems:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){
          items(first:$originFirst,after:$itemsAfter,query:$filter){nodes{id "#,
        board_item_values!(),
        board_item_content!(),
        r#"} pageInfo{hasNextPage endCursor}}
        }}
      }
      search(query:$search,type:$type,first:$originFirst,after:$searchAfter){
        pageInfo{hasNextPage endCursor}
        nodes{__typename ...BoardIssue}
      }
    }"#,
        board_issue!()
    );

    /// The board's own id and field definitions, and not one of its items.
    ///
    /// What a write needs of the board when the item it writes does not say: the id a field
    /// write and `addProjectV2ItemById` address, and the definitions of the `Status` and
    /// origin fields. It selects no `items`, so what it costs is the board's field list
    /// however many items the board holds — and it decides nothing about which items those
    /// are, which is the question a read of one item by its own id answers instead.
    ///
    /// The root is aliased `boardFields` rather than `owner`, so nothing counting the
    /// board's item reads by their root counts this one among them.
    pub const BOARD_FIELDS: &str = r#"query($owner:String!,$number:Int!,$nestedFirst:Int!){
      boardFields:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){id
          fields(first:$nestedFirst){nodes{
            ... on ProjectV2SingleSelectField{__typename id name options{id name}}
            ... on ProjectV2Field{__typename id name}
          }pageInfo{hasNextPage}}
        }}
      }
    }"#;

    /// One board draft by its own node id, with the board item it sits in.
    ///
    /// A draft is not an issue, so [`ISSUE`] reaches it and reads nothing of it; this is the
    /// second read that answers it. `DraftIssue.projectV2Items` names the board item a draft
    /// is — GitHub links a draft to one item — with the same [`board_item_values!`] the
    /// issue fragment reads, so a draft reached by id resolves through the same resolver a
    /// board listing hands it to, and nothing has to list the board to find one.
    pub const DRAFT: &str = concat!(
        r#"query($id:ID!,$nestedFirst:Int!,$boardItems:Int!){
      node(id:$id){__typename ... on DraftIssue{id title body createdAt updatedAt
        projectV2Items(first:$boardItems){nodes{id project{id number}
        "#,
        board_item_values!(),
        r#"}pageInfo{hasNextPage endCursor}}}}
    }"#
    );

    /// One issue's board memberships alone, walked past the page a read of it carried.
    ///
    /// The recovery read behind [`GitHubProjectsSource::resolve_issue`](super::GitHubProjectsSource):
    /// every document above carries a *page* of `Issue.projectItems`, and an issue on more
    /// boards than that page holds may have this board's entry past its end. This asks that
    /// one issue for its memberships and nothing else — the caller already holds the issue —
    /// so an answer of "this board does not hold it" is only ever given about a connection
    /// read to exhaustion.
    ///
    /// It selects the board item's id, its project number and the same
    /// [`board_item_values!`] the fragment does, because what it produces is handed to the
    /// very same resolver: an issue recovered this way reports the same title, the same
    /// status, the same labels and the same qualified id as one whose entry was on the
    /// page.
    ///
    /// `$first` rather than `$boardItems`: this document reads one issue, so nothing
    /// multiplies through it and the membership connection can be walked at
    /// [`MAX_PAGE_SIZE`](super::MAX_PAGE_SIZE) — which is what keeps the recovery to one
    /// further request for any issue a person really keeps.
    pub const ISSUE_BOARD_ITEMS: &str = concat!(
        r#"query($id:ID!,$first:Int!,$after:String,$nestedFirst:Int!){
      node(id:$id){
        ... on Issue{projectItems(first:$first,after:$after){
          nodes{id project{id number}
        "#,
        board_item_values!(),
        r#"}
          pageInfo{hasNextPage endCursor}}}
      }
    }"#
    );
    /// Resolves the configured repository's node id, which creating an issue requires.
    pub const REPOSITORY: &str = r#"query($owner:String!,$name:String!){repository(owner:$owner,name:$name){id nameWithOwner}}"#;
    /// What creating an issue needs and has not read yet: the board's own id and field
    /// definitions, as [`BOARD_FIELDS`] reads them, and the node id of the repository the
    /// issue is created in, as [`REPOSITORY`] reads it — in one request.
    ///
    /// Sent at the point a create knows which repository it is for, when neither half is
    /// already known to this process; a create needing only one of them sends that one's own
    /// document. Neither half is kept past the process: a field's option ids are re-minted by
    /// `sources fields --apply`, so a copy of them held between runs would write the wrong
    /// status.
    pub const CREATION_CONTEXT: &str = r#"query($owner:String!,$number:Int!,$nestedFirst:Int!,$repositoryOwner:String!,$repositoryName:String!){
      boardFields:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){id
          fields(first:$nestedFirst){nodes{
            ... on ProjectV2SingleSelectField{__typename id name options{id name}}
            ... on ProjectV2Field{__typename id name}
          }pageInfo{hasNextPage}}
        }}
      }
      repository(owner:$repositoryOwner,name:$repositoryName){id nameWithOwner}
    }"#;
    /// Reads both dependency directions for one issue, with each far end's own kind — and
    /// the issue's own body, which is where an edge to another source is recorded, so that
    /// half of a dependency read needs no second read of the issue or of the board.
    pub const ISSUE_DEPENDENCIES: &str = concat!(
        r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){__typename
      ... on Issue{body
        blockedBy(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
        blocking(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
      }}}"#,
        related_issue!()
    );
    /// Creates one issue in the configured repository, on no board: [`ADD_TO_BOARD`] files
    /// it. `CreateIssueInput.projectV2Ids` is not sent — see the crate's notes on what GitHub
    /// answered when it was.
    pub const CREATE_ISSUE: &str =
        r#"mutation($input:CreateIssueInput!){createIssue(input:$input){issue{id number url}}}"#;
    /// Puts an existing issue on the configured board.
    pub const ADD_TO_BOARD: &str = r#"mutation($input:AddProjectV2ItemByIdInput!){addProjectV2ItemById(input:$input){item{id}}}"#;
    /// Updates an issue's visible fields and its open or closed state in one call.
    pub const UPDATE_ISSUE: &str =
        r#"mutation($input:UpdateIssueInput!){updateIssue(input:$input){issue{id}}}"#;
    /// Updates an existing draft's user-visible fields.
    pub const UPDATE_DRAFT: &str = r#"mutation($input:UpdateProjectV2DraftIssueInput!){updateProjectV2DraftIssue(input:$input){draftIssue{id}}}"#;
    /// Updates a text or single-select value on one project item.
    pub const UPDATE_FIELD: &str = r#"mutation($input:UpdateProjectV2ItemFieldValueInput!,$readPriority:Boolean!,$priorityName:String!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id fieldValueByName(name:$priorityName) @include(if:$readPriority){... on ProjectV2ItemFieldSingleSelectValue{name field{... on ProjectV2SingleSelectField{id name options{id name}}}}}}}}"#;
    /// Writes up to three board fields and an optional clear in one ordered mutation.
    pub const UPDATE_FIELDS: &str = r#"mutation($input:UpdateProjectV2ItemFieldValueInput!,$second:UpdateProjectV2ItemFieldValueInput!,$third:UpdateProjectV2ItemFieldValueInput!,$clear:ClearProjectV2ItemFieldValueInput!,$writeSecond:Boolean!,$writeThird:Boolean!,$writeClear:Boolean!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id}} second:updateProjectV2ItemFieldValue(input:$second) @include(if:$writeSecond){projectV2Item{id}} third:updateProjectV2ItemFieldValue(input:$third) @include(if:$writeThird){projectV2Item{id}} cleared:clearProjectV2ItemFieldValue(input:$clear) @include(if:$writeClear){projectV2Item{id}}}"#;
    /// Clears one project item's value of one field, which is what a `none` priority is.
    pub const CLEAR_FIELD: &str = r#"mutation($input:ClearProjectV2ItemFieldValueInput!,$readPriority:Boolean!,$priorityName:String!){clearProjectV2ItemFieldValue(input:$input){projectV2Item{id fieldValueByName(name:$priorityName) @include(if:$readPriority){... on ProjectV2ItemFieldSingleSelectValue{name field{... on ProjectV2SingleSelectField{id name options{id name}}}}}}}}"#;
    /// Creates one single-select field with its options. Only the guarded field setup may use
    /// this document, and only for a field the board lacks.
    pub const CREATE_FIELD: &str = r#"mutation($input:CreateProjectV2FieldInput!){createProjectV2Field(input:$input){projectV2Field{... on ProjectV2SingleSelectField{id name options{id name color description}}}}}"#;
    /// Replaces a single-select field's options. Only the guarded field setup — the
    /// `status-options` and `fields` operations — may use this document, because GitHub
    /// treats the input as the complete option list.
    pub const STATUS_OPTIONS_UPDATE: &str = r#"mutation($input:UpdateProjectV2FieldInput!){updateProjectV2Field(input:$input){projectV2Field{... on ProjectV2SingleSelectField{id options{id name color description}}}}}"#;
    /// A fresh snapshot of the Status field and every board item's assignment.
    pub const STATUS_OPTIONS_SNAPSHOT: &str = r#"query($owner:String!,$number:Int!,$first:Int!,$after:String,$nestedFirst:Int!){owner:repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id fields(first:$nestedFirst){nodes{... on ProjectV2SingleSelectField{id name options{id name color description}}}pageInfo{hasNextPage}} items(first:$first,after:$after){nodes{id fieldValues(first:$nestedFirst){nodes{... on ProjectV2ItemFieldSingleSelectValue{name optionId field{... on ProjectV2SingleSelectField{id name}}}}pageInfo{hasNextPage}}}pageInfo{hasNextPage endCursor}}}}}}"#;
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

    /// Everything this source reads about one issue comment, wherever it reaches one.
    ///
    /// A macro for the reason [`board_issue!`] is one: a comment listed, a comment just added
    /// and a comment just edited are handed to one mapper, so they are selected by one
    /// spelling. `author` is `Actor`, which GitHub answers `null` for an account that no
    /// longer exists, and `login` is the one member every kind of actor carries.
    macro_rules! issue_comment {
        () => {
            "id author{login} createdAt updatedAt body url"
        };
    }

    /// One task's comments: a page of its issue's own `comments` connection.
    ///
    /// **No `orderBy`, and that is what makes the page oldest first.** GitHub's only
    /// `IssueCommentOrder` field is `UPDATED_AT`, which would move a comment to the end of the
    /// list every time somebody edited it; left unordered the connection answers in the order
    /// the comments were written, which is the order GitHub documents for the same collection
    /// over REST — ascending id. Nothing multiplies through it, so `$first` is the whole of its
    /// node count and the caller's own page size is pushed straight down.
    pub const ISSUE_COMMENTS: &str = concat!(
        r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){__typename ... on Issue{comments(first:$first,after:$after){nodes{"#,
        issue_comment!(),
        r#"}pageInfo{hasNextPage endCursor}}}}}"#
    );
    /// One issue by its own node id, with a page of its comments: what `task show` and a
    /// comment listing read, in one request.
    ///
    /// [`ISSUE`] and [`ISSUE_COMMENTS`] in one document, rather than one then the other. The
    /// comments are selected here and **not** on the shared `board_issue!` fragment, which
    /// [`SEARCH_ISSUES`] and [`SUB_ISSUES`] nest under a page of a hundred issues: a comment
    /// connection there would multiply through both of those documents' price, and neither
    /// needs one.
    pub const ISSUE_DETAIL: &str = concat!(
        r#"query($id:ID!,$first:Int!,$after:String,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      node(id:$id){__typename ...BoardIssue ... on Issue{comments(first:$first,after:$after){nodes{"#,
        issue_comment!(),
        r#"}pageInfo{hasNextPage endCursor}}}}
    }"#,
        board_issue!()
    );

    /// One alias of [`ISSUE_DETAILS`]: the item a batch's `$id<n>` names, with the first
    /// page of its comments when `$comments` asks for them.
    macro_rules! issue_details_alias {
        ($n:literal) => {
            concat!(
                "\n      i",
                stringify!($n),
                ":node(id:$id",
                stringify!($n),
                "){__typename ...BoardIssue ... on Issue{comments(first:$first) @include(if:$comments){nodes{",
                issue_comment!(),
                "}pageInfo{hasNextPage endCursor}}}}"
            )
        };
    }

    /// [`ISSUE_DETAIL`] for [`DETAIL_BATCH`](super::DETAIL_BATCH) items at once, each by its
    /// own node id, as one fixed-size document of aliased `node(id:)` fields.
    ///
    /// **Aliased `node(id:)` rather than `nodes(ids:)`, and that is what keeps its price
    /// honest.** The `github-graphql-node-count` model this workspace prices with treats a
    /// field that supplies neither `first` nor `last` as free, and `nodes(ids:)` supplies
    /// neither — so every connection under it would be priced at nothing and the pin in
    /// `tests/point_cost.rs` would understate what GitHub charges. Each alias here is the
    /// one-item read the model already prices, so the batch costs what its aliases cost.
    ///
    /// **Fixed-size, so there is one document to price.** A batch of fewer items binds the
    /// slots it has no item for to the last item it does, and reads that item again; the
    /// price is the document's, whatever its variables, so a short batch costs what a full
    /// one does and nothing more.
    pub const ISSUE_DETAILS: &str = concat!(
        r#"query($id0:ID!,$id1:ID!,$id2:ID!,$id3:ID!,$id4:ID!,$id5:ID!,$id6:ID!,$id7:ID!,$id8:ID!,$id9:ID!,$id10:ID!,$id11:ID!,$id12:ID!,$id13:ID!,$id14:ID!,$id15:ID!,$id16:ID!,$id17:ID!,$id18:ID!,$id19:ID!,$id20:ID!,$id21:ID!,$id22:ID!,$id23:ID!,$first:Int!,$comments:Boolean!,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){"#,
        issue_details_alias!(0),
        issue_details_alias!(1),
        issue_details_alias!(2),
        issue_details_alias!(3),
        issue_details_alias!(4),
        issue_details_alias!(5),
        issue_details_alias!(6),
        issue_details_alias!(7),
        issue_details_alias!(8),
        issue_details_alias!(9),
        issue_details_alias!(10),
        issue_details_alias!(11),
        issue_details_alias!(12),
        issue_details_alias!(13),
        issue_details_alias!(14),
        issue_details_alias!(15),
        issue_details_alias!(16),
        issue_details_alias!(17),
        issue_details_alias!(18),
        issue_details_alias!(19),
        issue_details_alias!(20),
        issue_details_alias!(21),
        issue_details_alias!(22),
        issue_details_alias!(23),
        "\n    }",
        board_issue!()
    );

    /// Which issue one comment is on, read before that comment is edited or removed.
    ///
    /// GitHub's comment mutations take the comment's id and nothing else, so without this a
    /// comment id given against the wrong task would change a comment on another issue.
    pub const COMMENT_ISSUE: &str =
        r#"query($id:ID!){node(id:$id){__typename ... on IssueComment{id issue{id}}}}"#;
    /// Adds one comment to an issue, signed as the account the token belongs to.
    pub const ADD_COMMENT: &str = concat!(
        r#"mutation($input:AddCommentInput!){addComment(input:$input){subject{id} commentEdge{node{"#,
        issue_comment!(),
        r#"}}}}"#
    );
    /// Replaces the body of one issue comment.
    pub const UPDATE_COMMENT: &str = concat!(
        r#"mutation($input:UpdateIssueCommentInput!){updateIssueComment(input:$input){issueComment{"#,
        issue_comment!(),
        r#"}}}"#
    );
    /// Removes one issue comment. Its payload carries nothing about the comment it removed.
    pub const DELETE_COMMENT: &str = r#"mutation($input:DeleteIssueCommentInput!){deleteIssueComment(input:$input){clientMutationId}}"#;

    /// Every document above, with what this source is doing when it sends one.
    ///
    /// One list rather than a `match` beside the constants: a rate-limit diagnostic has to
    /// name the call that was refused, and a `match` with a catch-all arm would answer a
    /// document added later with "talking to GitHub" and never say so.
    ///
    /// `documents_are_all_inventoried` reads this file back and fails naming any `pub
    /// const` here that this list omits, so the two cannot part — which is the same guard
    /// `CATEGORIES` carries, in the one shape available to a set of `&str` constants.
    pub const DOCUMENTS: [(&str, &str); 33] = [
        (SEARCH_ISSUES, "searching this board's issues"),
        (ISSUE, "reading one issue"),
        (
            ISSUE_BOARD_ITEMS,
            "reading one issue's board memberships past the page it came with",
        ),
        (SUB_ISSUES, "reading a project's tasks"),
        (BOARD, "reading the board"),
        (ORIGIN_LOOKUP, "looking up the items copied from one origin"),
        (BOARD_FIELDS, "reading the board's fields"),
        (DRAFT, "reading one draft"),
        (REPOSITORY, "reading the destination repository"),
        (
            CREATION_CONTEXT,
            "reading the board's fields and the destination repository",
        ),
        (ISSUE_DEPENDENCIES, "reading an issue's dependencies"),
        (CREATE_ISSUE, "creating an issue"),
        (ADD_TO_BOARD, "adding an issue to the board"),
        (UPDATE_ISSUE, "updating an issue"),
        (UPDATE_DRAFT, "updating a draft item"),
        (UPDATE_FIELD, "writing a board field"),
        (UPDATE_FIELDS, "writing board fields together"),
        (CLEAR_FIELD, "clearing a board field"),
        (
            CREATE_FIELD,
            "creating a board single-select field with its options",
        ),
        (
            STATUS_OPTIONS_SNAPSHOT,
            "snapshotting board Status options and assignments",
        ),
        (
            STATUS_OPTIONS_UPDATE,
            "safely replacing the board Status option list",
        ),
        (ADD_SUB_ISSUE, "filing an issue under its project"),
        (REMOVE_SUB_ISSUE, "taking an issue out of its project"),
        (ADD_BLOCKED_BY, "recording a dependency"),
        (REMOVE_BLOCKED_BY, "removing a dependency"),
        (DELETE_ISSUE, "deleting an issue"),
        (ISSUE_COMMENTS, "reading a task's comments"),
        (ISSUE_DETAIL, "reading one issue with its comments"),
        (
            ISSUE_DETAILS,
            "reading a batch of issues with their comments",
        ),
        (COMMENT_ISSUE, "reading which issue a comment is on"),
        (ADD_COMMENT, "adding a comment"),
        (UPDATE_COMMENT, "editing a comment"),
        (DELETE_COMMENT, "deleting a comment"),
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
/// than looked for in the response text. `api rate limit already exceeded` is what GraphQL
/// answers a request made once the hour's budget is spent — "API rate limit already exceeded
/// for user ID …" in the `errors` of an HTTP 200, with no `type` — and neither of the other
/// two phrases is a substring of it, so without it that answer read as a refusal that will
/// never lift. Pinned and gated exactly as [`SECONDARY_WORDINGS`] is, and public for the same
/// one reason.
pub const PRIMARY_WORDINGS: [&str; 4] = [
    "api rate limit exceeded",
    "api rate limit already exceeded",
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
/// one, a sub-issue link, a dependency, a comment — or edits or removes content of that
/// kind, and no query in [`graphql::DOCUMENTS`] does, so what the secondary limiter counts
/// and what the keyword says are the same set. That is what makes the keyword a sound test
/// rather than a convenient one: pacing an edit or a removal the limiter might not have
/// counted costs a wait, and not pacing one it did count costs the next fifty minutes.
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

/// The name of a `Status` single-select option on the board.
///
/// Validated on the way in rather than checked later, so a blank option name — which
/// nothing on a board can be — is a state this type cannot hold.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(try_from = "String")]
#[schemars(extend("minLength" = 1))]
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
    // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This doc is the field's schema description, which is what a person configuring the source reads, so it has to say when the field decides an issue's repository and when the item's own field does; the rule's one executable source is `GitHubProjectsSource::creation_target`, and `tests/plugin.rs` drives each case named here against the loopback board.
    /// `owner/name` of the repository this source creates an issue in when the item's own
    /// `repositories` field does not decide it.
    ///
    /// An item naming exactly one repository is created there; a task or a document naming
    /// none or several is created in its parent project's repository; and a project, or a
    /// task or document with no parent, naming none or several is created here. A board
    /// has no repository of its own and `createIssue` requires one, so a write without
    /// this is refused naming the field. Reads never need it.
    pub repository: Option<String>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the `owner/name` grammar before private construction.
    // llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
    /// Environment variable containing a fine-grained token with Projects and Issues
    /// read/write plus Pull requests read-only access for every repository represented on
    /// the board.
    #[serde(default = "default_token_env")]
    pub token_env: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the environment-variable grammar.
    /// GraphQL endpoint. GitHub Enterprise installations may override it.
    #[serde(default = "default_endpoint")]
    pub endpoint: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` converts it to the private validated `Url`.
    /// Per-instance mapping from a status category to the option of the board's one
    /// `Status` field it lands on, for a task and for a project.
    ///
    /// The shared `StatusMapping` grammar: each value is one option name for both kinds,
    /// `null` to disable the category for both, or `{task, project}` naming it per kind,
    /// where a kind left out leaves the category unmapped for that kind. A category this
    /// does not mention keeps its shipped default for both kinds: `backlog` to "Backlog",
    /// `todo` to "Todo", `queued` to "Queued", `in-progress` to "In Progress", `done` to
    /// "Done" plus closed as completed, `cancelled` to "Cancelled" plus closed as not
    /// planned, and `draft` and `unknown` unmapped. A category it does mention gets no
    /// shipped default for a kind it leaves out. `done` and `cancelled` close the issue for
    /// either kind. No two categories may name one option for the same kind, ignoring case.
    /// `unknown` may name one existing option; every unknown word then lands on it and
    /// reads back as `unknown` under its name. Unlike `local-md`, this source cannot keep
    /// each unknown word because it never creates board options.
    #[serde(default)]
    pub status_mapping: StatusMapping,
    /// Per-instance mapping from a task's priority to an option of this board's
    /// single-select field named `Priority`.
    ///
    /// Absent, this source holds no priority: every task reads as `none`, and a write of any
    /// other priority is refused before it reaches this board. Present, each of `urgent`,
    /// `high`, `medium` and `low` it does not mention keeps its shipped default — `Urgent`,
    /// `High`, `Medium` and `Low` — and an item with no value in the `Priority` field reads
    /// as `none`, so writing `none` clears the value. Option names match case-insensitively;
    /// no two levels may name one option. Reads and writes never create the field or an
    /// option: `onetaskgraph sources fields <source> --apply` does, and a write naming one
    /// the board lacks is refused pointing there.
    #[serde(default)]
    pub priority_mapping: Option<PriorityMappingConfig>,
    /// How fast this source writes, and how long it waits out a rate-limit refusal.
    ///
    /// Every field keeps its shipped default when it is absent, and the defaults are
    /// GitHub's own published limits rather than taste. See [`Pacing`].
    #[serde(default)]
    pub pacing: PacingConfig,
}

/// Which option of the board's `Priority` field each priority lands on.
///
/// One member per level rather than a map, so a key that is not a level is refused where
/// the configuration is read, naming the levels there are. `none` is not a member: it is no
/// value in the field, not an option of it.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PriorityMappingConfig {
    /// The option `urgent` lands on; `Urgent` when absent.
    pub urgent: Option<PriorityOptionName>,
    /// The option `high` lands on; `High` when absent.
    pub high: Option<PriorityOptionName>,
    /// The option `medium` lands on; `Medium` when absent.
    pub medium: Option<PriorityOptionName>,
    /// The option `low` lands on; `Low` when absent.
    pub low: Option<PriorityOptionName>,
}

/// The name of an option of the board's `Priority` single-select field.
///
/// Validated on the way in, for the reason [`ColumnName`] is: nothing on a board can have a
/// blank name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(try_from = "String")]
#[schemars(extend("minLength" = 1))]
pub struct PriorityOptionName(String);

impl PriorityOptionName {
    /// The option name, as the board spells it.
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PriorityOptionName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.trim().is_empty() {
            return Err("a priority_mapping option name cannot be blank".to_owned());
        }
        Ok(Self(name))
    }
}

/// The name of the board field a priority is held in.
pub const PRIORITY_FIELD: &str = "Priority";

/// The four priorities a board option can hold, in the order a new `Priority` field lists
/// them. `none` is not among them: it is the field holding no value.
///
/// This list mirrors `Priority`, so it carries its own drift gate, in the shape [`CATEGORIES`]
/// does: [`level_position`] is a wildcard-free match, so a priority added to the shared
/// vocabulary fails to compile until it is placed there, and this crate's suite reconciles
/// this list and [`PriorityMappingConfig`]'s members against that enum's own derived schema.
pub const PRIORITY_LEVELS: [Priority; 4] = [
    Priority::Urgent,
    Priority::High,
    Priority::Medium,
    Priority::Low,
];

/// Where one priority sits in [`PRIORITY_LEVELS`], or `None` for `none`, which is no option;
/// see that list for what this pins.
#[must_use]
pub const fn level_position(priority: Priority) -> Option<usize> {
    match priority {
        Priority::None => None,
        Priority::Urgent => Some(0),
        Priority::High => Some(1),
        Priority::Medium => Some(2),
        Priority::Low => Some(3),
    }
}

/// This instance's complete priority-to-option mapping, read in both directions.
///
/// One option per level, held in [`PRIORITY_LEVELS`] order, once it is established that no
/// two levels name one option.
#[derive(Debug, Clone)]
struct PriorityMapping {
    options: [PriorityOptionName; 4],
}

impl PriorityMapping {
    fn resolve(config: PriorityMappingConfig, instance: &SourceName) -> Result<Self, SourceError> {
        let shipped = |name: &str| PriorityOptionName(name.to_owned());
        let mapping = Self {
            options: [
                config.urgent.unwrap_or_else(|| shipped("Urgent")),
                config.high.unwrap_or_else(|| shipped("High")),
                config.medium.unwrap_or_else(|| shipped("Medium")),
                config.low.unwrap_or_else(|| shipped("Low")),
            ],
        };
        for (index, option) in mapping.options.iter().enumerate() {
            if let Some(earlier) = mapping.options[..index]
                .iter()
                .position(|other| other.as_str().eq_ignore_ascii_case(option.as_str()))
            {
                return Err(SourceError::Config {
                    message: format!(
                        "priority_mapping of source {instance} sends both {} and {} to the board \
                         option {:?}; one option cannot read back as two priorities",
                        PRIORITY_LEVELS[earlier],
                        PRIORITY_LEVELS[index],
                        option.as_str()
                    ),
                });
            }
        }
        Ok(mapping)
    }

    /// The option `priority` lands on, or `None` for `none`, which is no option at all.
    fn option(&self, priority: Priority) -> Option<&str> {
        level_position(priority).map(|index| self.options[index].as_str())
    }

    /// The priority a board option name reports, or `None` when nothing maps to it.
    fn priority_of(&self, option: &str) -> Option<Priority> {
        self.options
            .iter()
            .position(|name| name.as_str().eq_ignore_ascii_case(option))
            .map(|index| PRIORITY_LEVELS[index])
    }

    /// Every mapped option name, in the order a new `Priority` field lists them.
    fn names(&self) -> impl Iterator<Item = &str> {
        self.options.iter().map(PriorityOptionName::as_str)
    }
}

/// What one item's `Priority` field says, read through this instance's mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HeldPriority {
    /// A priority this source reports: an option the mapping names, or no value (`none`).
    Read(Priority),
    /// An option the mapping does not name, which is never read as a level or as `none`.
    Unmapped(String),
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
        let prefix = format!("source {name}: ");
        let source = GitHubProjectsSource::recording_into(name, config, secrets, ledger).map_err(
            |error| match error {
                // The shared `StatusMapping::distinct` names the source itself.
                SourceError::Config { message } if message.starts_with(&prefix) => {
                    SourceError::Config { message }
                }
                SourceError::Config { message } => SourceError::Config {
                    message: format!("{prefix}{message}"),
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
    /// Not usable against this instance for this kind, and why.
    Disabled(UnmappedStatus),
    /// The board's `Status` option of this name.
    Column(ColumnName),
    /// A closed issue, with both its board option and the reason that says which closed it means.
    // llmlint: ignore[invalid_states_unrepresentable] The reason is fixed by the category — `done` closes as completed, `cancelled` as not planned — and this private enum is built in one place, `BoardStatuses::resolve`, which pairs each from the category's own slot. Carrying the reason on the target is what lets every write site that holds only a target derive its `stateInput` from that one resolved model rather than re-deriving it from a category and risking a disagreement with the mapping.
    Terminal(ColumnName, ClosedState),
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
pub const CATEGORIES: [StatusCategory; 8] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::Queued,
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
        StatusCategory::Queued => 3,
        StatusCategory::InProgress => 4,
        StatusCategory::Done => 5,
        StatusCategory::Cancelled => 6,
        StatusCategory::Unknown => 7,
    }
}

/// The spelling a status category is configured and reported under.
fn category_name(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Draft => "draft",
        StatusCategory::Backlog => "backlog",
        StatusCategory::Todo => "todo",
        StatusCategory::Queued => "queued",
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

/// The shipped default for one category this instance's `status_mapping` does not mention,
/// for either kind.
fn shipped_default(category: StatusCategory) -> StatusTarget {
    match category {
        StatusCategory::Backlog => StatusTarget::Column(shipped_column("Backlog")),
        StatusCategory::Todo => StatusTarget::Column(shipped_column("Todo")),
        StatusCategory::Queued => StatusTarget::Column(shipped_column("Queued")),
        StatusCategory::InProgress => StatusTarget::Column(shipped_column("In Progress")),
        StatusCategory::Done => {
            StatusTarget::Terminal(shipped_column("Done"), ClosedState::Completed)
        }
        StatusCategory::Cancelled => {
            StatusTarget::Terminal(shipped_column("Cancelled"), ClosedState::NotPlanned)
        }
        StatusCategory::Draft | StatusCategory::Unknown => {
            StatusTarget::Disabled(UnmappedStatus::Unconfigured)
        }
    }
}

/// The two kinds a status is written and read for, each with its own half of the mapping.
const STATUS_KINDS: [ItemKind; 2] = [ItemKind::Task, ItemKind::Project];

/// This instance's complete category-to-target mapping for each kind, read in both
/// directions.
///
/// One target per category per kind, held at that category's own [`category_position`], so
/// a category missing from the mapping, named twice in it, or filed out of order is a state
/// this type cannot hold rather than one [`Self::target`] has to defend against. Both kinds'
/// targets are options of the board's one `Status` field.
#[derive(Debug, Clone)]
struct BoardStatuses {
    tasks: [StatusTarget; CATEGORIES.len()],
    projects: [StatusTarget; CATEGORIES.len()],
}

impl BoardStatuses {
    /// Resolve `configured` against the shipped defaults, refusing two categories one kind
    /// would read back from one option.
    ///
    /// A category the mapping does not mention keeps its shipped default for both kinds; one
    /// it does mention is exactly what it configures, so a per-kind object leaves the kind it
    /// omits unmapped rather than defaulted.
    fn resolve(configured: &StatusMapping, instance: &SourceName) -> Result<Self, SourceError> {
        let resolve_kind =
            |kind: ItemKind| -> Result<[StatusTarget; CATEGORIES.len()], SourceError> {
                // `CATEGORIES[position] == category` for every category — the crate's suite
                // asserts it — so mapping the list in order fills each category's own slot.
                let mut targets = CATEGORIES.map(shipped_default);
                for (slot, category) in targets.iter_mut().zip(CATEGORIES) {
                    if !configured.mentions(category) {
                        continue;
                    }
                    *slot = match configured.name_for(category, kind) {
                        Err(why) => StatusTarget::Disabled(why),
                        Ok(name) => {
                            let option = ColumnName::try_from(name.as_str().to_owned())
                                .map_err(|message| SourceError::Config { message })?;
                            match category {
                                StatusCategory::Done => {
                                    StatusTarget::Terminal(option, ClosedState::Completed)
                                }
                                StatusCategory::Cancelled => {
                                    StatusTarget::Terminal(option, ClosedState::NotPlanned)
                                }
                                _ => StatusTarget::Column(option),
                            }
                        }
                    };
                }
                StatusMapping::distinct(
                    instance,
                    kind,
                    CATEGORIES
                        .iter()
                        .zip(&targets)
                        .filter_map(|(category, target)| target.option().map(|o| (*category, o))),
                )?;
                Ok(targets)
            };
        Ok(Self {
            tasks: resolve_kind(ItemKind::Task)?,
            projects: resolve_kind(ItemKind::Project)?,
        })
    }

    /// Every category's target for `kind`, in category order.
    const fn targets(&self, kind: ItemKind) -> &[StatusTarget; CATEGORIES.len()] {
        match kind {
            ItemKind::Task => &self.tasks,
            ItemKind::Project => &self.projects,
        }
    }

    fn target(&self, kind: ItemKind, category: StatusCategory) -> &StatusTarget {
        &self.targets(kind)[category_position(category)]
    }

    /// The category a board option name reports for `kind`, or `None` when nothing of that
    /// kind maps to it.
    fn category_of(&self, kind: ItemKind, option: &str) -> Option<StatusCategory> {
        CATEGORIES.into_iter().find(|category| {
            self.target(kind, *category)
                .option()
                .is_some_and(|name| name.eq_ignore_ascii_case(option))
        })
    }

    /// Every option name either kind maps a category to, each once ignoring case, in
    /// category order with a task's name before a project's — what the guarded setup asks
    /// the `Status` field to hold.
    fn wanted(&self) -> Vec<String> {
        let mut wanted: Vec<String> = Vec::new();
        for category in CATEGORIES {
            for kind in STATUS_KINDS {
                if let Some(name) = self.target(kind, category).option()
                    && !wanted.iter().any(|held| held.eq_ignore_ascii_case(name))
                {
                    wanted.push(name.to_owned());
                }
            }
        }
        wanted
    }

    /// The status an item of `kind` reports, from the three things a read of it says: its
    /// board `Status` option, whether its issue is closed, and the reason it was closed with.
    ///
    /// The closed state decides the category and the `Status` option decides the name, so
    /// a closed issue sitting in a "Shipped" column reports `done` named `Shipped`, whatever
    /// its kind. A closed issue whose reason is `DUPLICATE` or `REOPENED` reports `Unknown`:
    /// a duplicate is not finished work, and calling it done is a lie the next copy would
    /// write back. `REOPENED`-while-closed is a state this source can never produce, so
    /// it is read permissively rather than refused — reads are faithful, and refusals
    /// belong on writes. An open item's option reads through its own kind's mapping, and an
    /// option that mapping does not name reads as `Unknown` under its own name.
    ///
    /// One function of those three rather than of a response, so a narrow status write can
    /// answer what a re-read would report by applying it to the state it has just written.
    fn status(
        &self,
        kind: ItemKind,
        option: Option<&str>,
        closed: bool,
        reason: Option<&str>,
    ) -> Status {
        if closed {
            let category = match reason {
                None | Some("COMPLETED") => StatusCategory::Done,
                Some("NOT_PLANNED") => StatusCategory::Cancelled,
                Some(_) => StatusCategory::Unknown,
            };
            let fallback = match category {
                StatusCategory::Done => "Done",
                StatusCategory::Cancelled => "Cancelled",
                _ => "Closed",
            };
            return Status {
                category,
                name: option.unwrap_or(fallback).to_owned(),
            };
        }
        let name = option.unwrap_or("Open").to_owned();
        Status {
            category: self
                .category_of(kind, &name)
                .unwrap_or(StatusCategory::Unknown),
            name,
        }
    }
}

impl BoardStatuses {
    /// For each kind, the option names it maps a category to that `existing` lacks, ignoring
    /// case; a kind lacking none is left out.
    fn missing_by_kind(&self, existing: &[StatusOption]) -> Vec<KindMissing> {
        STATUS_KINDS
            .into_iter()
            .filter_map(|kind| {
                let missing: Vec<String> = self
                    .targets(kind)
                    .iter()
                    .filter_map(StatusTarget::option)
                    .filter(|wanted| {
                        !existing
                            .iter()
                            .any(|option| option.name.as_str().eq_ignore_ascii_case(wanted))
                    })
                    .map(str::to_owned)
                    .collect();
                (!missing.is_empty()).then_some(KindMissing { kind, missing })
            })
            .collect()
    }
}

impl StatusTarget {
    /// The board option this target selects, or `None` for an unmapped one.
    fn option(&self) -> Option<&str> {
        match self {
            Self::Column(name) | Self::Terminal(name, _) => Some(name.as_str()),
            Self::Disabled(_) => None,
        }
    }
}

// llmlint: ignore-block[comments_earn_their_place, contracts_have_one_source_or_a_drift_gate] Every `createIssue` names one of these, and which one is the rule — a reader who reaches the type from `create_and_file_issue` gets the rule in one sentence here without the method's refusals, which stay on `creation_target`, the rule's one executable source; `tests/plugin.rs` drives every arm of it against the loopback board.
/// One repository this source can create an issue in, as `owner/name`.
///
/// Every `createIssue` this source sends names one of these: the item's own single
/// `repositories` entry, else its parent project issue's repository, else the configured
/// [`GitHubProjectsConfig::repository`]. [`GitHubProjectsSource::creation_target`] makes
/// that choice and says what it refuses before `createIssue`.
// llmlint: ignore-end[comments_earn_their_place, contracts_have_one_source_or_a_drift_gate]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

    /// The one host whose repositories this source creates issues in, spelled once: it is
    /// what [`Self::origin`] renders and what [`Self::from_origin`] accepts.
    const HOST: &str = "github.com";

    fn origin(&self) -> String {
        format!("{}/{}/{}", Self::HOST, self.owner, self.name)
    }

    /// The repository a normalized origin names, or why it is none this source can create
    /// an issue in: another host, or more or fewer than `owner/name` under this one.
    fn from_origin(origin: &Repository) -> Result<Self, String> {
        let not_here = || {
            format!(
                "{} is not a {}/owner/name repository",
                origin.as_str(),
                Self::HOST
            )
        };
        let (host, rest) = origin.as_str().split_once('/').ok_or_else(not_here)?;
        if host != Self::HOST {
            return Err(not_here());
        }
        Self::parse(rest).map_err(|_| not_here())
    }

    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
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
    statuses: BoardStatuses,
    /// Where each priority lands on this board, or `None` when this instance holds none.
    priorities: Option<PriorityMapping>,
    client: Client,
    /// Every item this source has created in this command, in the order it created them —
    /// dropped by [`TaskSource::end_command`].
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
    /// Every item that already existed and that this source has written in this command, as
    /// it wrote it — dropped by [`TaskSource::end_command`].
    ///
    /// The other half of [`Self::created`], held on the same terms and for the reason a
    /// narrowed read needs it: an answer from GitHub's search or from the board's own field
    /// filter is an index behind a write this process made moments ago, so a query matching
    /// what this process just wrote onto an existing item would otherwise miss it. Nothing
    /// is remembered that this process did not itself just write.
    updated: Mutex<Vec<Resolved>>,
    /// How fast this source writes, and how long it waits out a refusal.
    pacing: Pacing,
    /// When the last content-creating mutation finished, or the moment the furthest-out
    /// reserved slot releases the next one, whichever is later — so the one after it can be
    /// spaced from that. See [`MIN_MUTATION_INTERVAL_MS`] for the interval and
    /// [`GitHubProjectsSource::finish_mutation`] for why completion rather than release is
    /// what it is measured from.
    last_mutation: Mutex<Option<Instant>>,
    /// The board as this process last read it, for the length of one command — dropped by
    /// [`TaskSource::end_command`].
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
    /// Every issue this board's own search reported, for the length of one command — dropped
    /// by [`TaskSource::end_command`].
    ///
    /// The second half of a board read, and cached for the same reason and on the same
    /// terms as the first: it lives and dies with the process, nothing is written down, and
    /// a write this process makes updates the entry here exactly as it updates the one in
    /// [`Self::board_cache`]. One read answers every question a command asks, so a command
    /// that lists this board's projects and its tasks pays for one search rather than two.
    search_cache: Mutex<Option<Vec<Resolved>>>,
    /// What each narrowed question GitHub was asked answered, keyed by that question, for
    /// the length of one command — dropped by [`TaskSource::end_command`].
    ///
    /// The narrowed counterpart of [`Self::search_cache`], held on the same terms: it lives
    /// and dies with the process, nothing is written down, a write this process makes
    /// updates the entry here as it updates the other two, and every answer is completed
    /// with this process's own writes each time it is given. A command that asks the same
    /// narrowed question twice — a wait polling for its own items, a listing repeated after a
    /// write — pays for it once, which is what the whole-board read it replaced gave it.
    narrowed_cache: Mutex<BTreeMap<String, Vec<Resolved>>>,
    search_next: Mutex<BTreeMap<String, Option<String>>>,
    /// Records already resolved in this command, reused by writes and for comment identity.
    /// Explicit item reads still reach GitHub. Nothing is persisted, and
    /// [`TaskSource::end_command`] drops every record, so a write in the next command reads
    /// its item as a person has since left it.
    resolved_cache: Mutex<BTreeMap<NativeId, Resolved>>,
    /// The board's own id and field definitions as this process last read them on their
    /// own, for the length of one command — dropped by [`TaskSource::end_command`].
    ///
    /// What a write needs of the board and its item does not say, read once per command
    /// rather than once per item written, on the terms [`Self::board_cache`] is held on: it
    /// lives and dies with the process and nothing is written down. It holds no item and so
    /// can answer no question about one — see [`Self::board_fields`].
    fields_cache: Mutex<Option<BoardFields>>,
    /// Each destination repository's node id, resolved once per repository
    /// rather than per issue created.
    ///
    /// A repository's node id does not change, and re-reading it for every issue of a copy
    /// spent one request per item on an answer this source already had. It is a map rather
    /// than one entry because a copy files each item in the repository its own
    /// `repositories` field names, so a plan across five repositories asks GitHub five
    /// times and not once per item.
    repository_cache: Mutex<BTreeMap<RepositoryTarget, String>>,
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

/// GitHub's closed single-select color vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatusOptionColor {
    /// Gray.
    Gray,
    /// Blue.
    Blue,
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Purple.
    Purple,
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Pink.
    Pink,
}

/// Whether a guarded board setup — of the fields, or of the Status options alone — plans or
/// applies its additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    /// Read without mutation.
    Plan,
    /// Apply and verify.
    Apply,
}

/// The name [`SetupMode`] had when Status was the one field set up, kept so a caller written
/// against it goes on compiling.
pub type StatusOptionsMode = SetupMode;

/// The explicit result of the requested operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StatusOptionsOutcome {
    /// A read-only plan.
    Planned,
    /// Apply found nothing missing.
    Unchanged,
    /// Additions were applied and verified.
    Applied,
}

/// A GitHub single-select option's opaque GraphQL node identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct StatusOptionId(#[schemars(length(min = 1))] String);

impl TryFrom<String> for StatusOptionId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        if id.trim().is_empty() {
            return Err("a GitHub Status option id cannot be blank".to_owned());
        }
        Ok(Self(id))
    }
}

/// One existing or proposed option in a guarded Status-field update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct StatusOption {
    /// GitHub's stable id.
    pub id: StatusOptionId,
    /// The visible option name.
    pub name: ColumnName,
    /// GitHub's single-select color token.
    pub color: StatusOptionColor,
    /// The option description, including an empty one.
    pub description: String,
}

/// One board item's Status assignment, retained as recovery data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct StatusAssignment {
    /// The project item id whose assignment this is.
    // llmlint: ignore[invalid_states_unrepresentable] This opaque GraphQL node ID is
    // carried verbatim as operator recovery data; introducing a semantic type would claim
    // validation rules GitHub does not publish and no operation here interprets.
    pub item_id: String,
    /// The selected option, absent when the item has no status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<AssignedStatusOption>,
}

/// The inseparable id and name of an assigned option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AssignedStatusOption {
    /// GitHub's stable id.
    pub id: StatusOptionId,
    /// The visible name.
    pub name: ColumnName,
}

/// The plan and verified outcome of reconciling configured Status options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct StatusOptionsReport {
    /// The configured source name.
    pub source: SourceName,
    /// Configured option names absent before the operation.
    // llmlint: ignore[invalid_states_unrepresentable] Each value originates from a
    // `ColumnName` and has therefore already passed its nonblank validation; retaining the
    // serialized string here preserves the report's intentionally simple public contract.
    pub missing: Vec<String>,
    /// What the requested operation did.
    pub outcome: StatusOptionsOutcome,
    /// The complete option list observed before any mutation.
    pub existing: Vec<StatusOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusSnapshot {
    // llmlint: ignore[invalid_states_unrepresentable] This private opaque GraphQL ID is
    // passed back as the mutation's project identity; a newtype could enforce no stronger
    // invariant because GitHub publishes no grammar for it.
    board_id: String,
    // llmlint: ignore[invalid_states_unrepresentable] This private opaque GraphQL ID is
    // passed back as the mutation's field identity; a newtype could enforce no stronger
    // invariant because GitHub publishes no grammar for it.
    field_id: String,
    options: Vec<StatusOption>,
    assignments: Vec<StatusAssignment>,
}

/// The name of the board field a status is held in.
const STATUS_FIELD: &str = "Status";

/// Every item's value of each field `report` names, as it stood before the setup wrote
/// anything — what a person puts back when the setup is refused part way.
fn recovery(report: &FieldsReport, before: &BoardSnapshot) -> Result<String, SourceError> {
    let assignments: BTreeMap<&str, Vec<StatusAssignment>> = report
        .fields
        .iter()
        .map(|field| (field.field.name(), before.assignments(field.field)))
        .collect();
    serde_json::to_string_pretty(&assignments).map_err(|error| SourceError::Malformed {
        message: format!("cannot render the pre-write field recovery snapshot: {error}"),
    })
}

/// One board field the guarded setup reads and writes — every one it reads, and the only
/// ones it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
pub enum BoardField {
    /// The single-select `Status` field every instance's `status_mapping` resolves into.
    Status,
    /// The single-select `Priority` field an instance's `priority_mapping` resolves into.
    Priority,
}

impl BoardField {
    /// The field's name on the board.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Status => STATUS_FIELD,
            Self::Priority => PRIORITY_FIELD,
        }
    }

    /// The field a board calls `name`, or `None` for one this setup does not own.
    fn named(name: &str) -> Option<Self> {
        [Self::Status, Self::Priority]
            .into_iter()
            .find(|field| field.name() == name)
    }
}

/// What the guarded setup did to one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FieldOutcome {
    /// A read-only plan.
    Planned,
    /// Apply found the field there with every configured option.
    Unchanged,
    /// Missing options were added to the field that was there, and verified.
    Applied,
    /// The field was not there; it was created holding the configured options, and verified.
    Created,
}

/// One field's plan, or its verified outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct FieldReport {
    /// Which field.
    pub field: BoardField,
    /// Whether the board had the field before the operation.
    // llmlint: ignore[invalid_states_unrepresentable] `exists` beside `outcome` is the report's
    // wire shape as its consumer's contract fixes it — `{"field", "exists", "missing",
    // "outcome", "existing"}` — so folding one into the other would change a published JSON
    // shape. The contradictory pairings cannot be built: `GitHubProjectsSource::fields` is the
    // one constructor, and it derives `outcome` from `exists` in one match.
    pub exists: bool,
    /// Configured option names the field lacked before the operation — every one of them,
    /// in the order a new field lists them, when the field was not there at all.
    // llmlint: ignore[invalid_states_unrepresentable] Each value originates from a validated
    // mapping name and has therefore already passed its nonblank validation; the serialized
    // string is the report's intentionally simple public contract, as `StatusOptionsReport`'s is.
    pub missing: Vec<String>,
    /// For the `Status` field, which item kind each missing name is configured for: one
    /// entry per kind `status_mapping` names a missing option for, task before project, each
    /// listing that kind's missing names in category order. A name both kinds use is in
    /// both. Empty — and left out of the JSON — when nothing is missing, and always for
    /// `Priority`, which only a task holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    // Kept in the schema as `"default": []` although the JSON leaves an empty list out, so
    // both SDKs model an absent `kinds` as an empty list rather than as `null`.
    #[schemars(!skip_serializing_if)]
    pub kinds: Vec<KindMissing>,
    /// What the requested operation did.
    pub outcome: FieldOutcome,
    /// The field's complete option list observed before any mutation; empty when the field
    /// was not there.
    pub existing: Vec<StatusOption>,
}

/// The `Status` option names one item kind's `status_mapping` names that the field lacked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct KindMissing {
    /// The kind these names are configured for.
    pub kind: ItemKind,
    /// The names that kind maps a category to and the field lacked, in category order.
    // llmlint: ignore[invalid_states_unrepresentable] Each value originates from a validated
    // mapping name, as `FieldReport::missing`'s do, and the serialized string is the report's
    // intentionally simple public contract.
    pub missing: Vec<String>,
}

/// The plan and verified outcome of setting up every field a source's configuration names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct FieldsReport {
    /// The configured source name.
    pub source: SourceName,
    /// `Status`, always, and `Priority` when the source sets `priority_mapping`.
    // llmlint: ignore[invalid_states_unrepresentable] A list is the report's wire shape as its
    // consumer's contract fixes it — `{"source", "fields": [...]}` — so a struct with one member
    // per field would change a published JSON shape. The states the list could hold and the
    // contract forbids cannot be built: `GitHubProjectsSource::fields` is the one constructor,
    // and it pushes `Status` first and exactly once, then `Priority` exactly when configured.
    pub fields: Vec<FieldReport>,
}

/// Which options one field is configured with, in the order a new field would list them.
struct FieldPlan {
    field: BoardField,
    wanted: Vec<String>,
}

/// One single-select field as the guarded setup snapshots it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotField {
    // llmlint: ignore[invalid_states_unrepresentable] This private opaque GraphQL ID is
    // passed back as the mutation's field identity; a newtype could enforce no stronger
    // invariant because GitHub publishes no grammar for it.
    field_id: String,
    options: Vec<StatusOption>,
}

/// Every single-select field of a board and every item's value of each.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BoardSnapshot {
    // llmlint: ignore[invalid_states_unrepresentable] This private opaque GraphQL ID is
    // passed back as the mutation's project identity; a newtype could enforce no stronger
    // invariant because GitHub publishes no grammar for it.
    board_id: String,
    fields: BTreeMap<BoardField, SnapshotField>,
    /// Each board item's id, and its value of each field this setup owns that it holds one of.
    items: Vec<(String, BTreeMap<BoardField, AssignedStatusOption>)>,
}

impl BoardSnapshot {
    /// Every item's value of `field`, in board order — the recovery data a drift refusal
    /// carries.
    fn assignments(&self, field: BoardField) -> Vec<StatusAssignment> {
        self.items
            .iter()
            .map(|(item_id, values)| StatusAssignment {
                item_id: item_id.clone(),
                option: values.get(&field).cloned(),
            })
            .collect()
    }
}

impl GitHubProjectsSource {
    /// Report missing configured Status options and, when `apply` is true, add them with
    /// a whole-list mutation that preserves every existing id and verifies the result.
    ///
    /// # Errors
    ///
    /// Refuses a board without a single-select `Status` field. A post-write difference in
    /// any pre-existing option id or item assignment is refused with the complete pre-write
    /// assignment snapshot in the diagnostic for recovery.
    // llmlint: ignore[changed_behavior_has_e2e] The CLI journeys cover plan, no-op apply,
    // successful mutation, both drift refusals, source selection, missing Status, casing,
    // and paging. Transport errors remain the shared `graphql` boundary's behavior rather
    // than a new status-options behavior, and the pinned-schema test prevents valid GitHub
    // responses from entering the defensive malformed-response branches below.
    pub async fn status_options(
        &self,
        mode: StatusOptionsMode,
    ) -> Result<StatusOptionsReport, SourceError> {
        let before = self.status_snapshot().await?;
        // A terminal category's option is as configured as an open one's: a terminal
        // write validates it before closing and refuses when the board lacks it. Both
        // kinds' names are options of the one field, so both are asked for.
        let missing = self
            .statuses
            .wanted()
            .into_iter()
            .filter(|wanted| {
                !before
                    .options
                    .iter()
                    .any(|option| option.name.as_str().eq_ignore_ascii_case(wanted))
            })
            .collect::<Vec<_>>();
        let report = StatusOptionsReport {
            source: self.name.clone(),
            missing: missing.clone(),
            outcome: match (mode, missing.is_empty()) {
                (StatusOptionsMode::Plan, _) => StatusOptionsOutcome::Planned,
                (StatusOptionsMode::Apply, true) => StatusOptionsOutcome::Unchanged,
                (StatusOptionsMode::Apply, false) => StatusOptionsOutcome::Applied,
            },
            existing: before.options.clone(),
        };
        if mode == StatusOptionsMode::Plan || missing.is_empty() {
            return Ok(report);
        }
        let mut options = before
            .options
            .iter()
            .map(|option| {
                json!({
                    "id": option.id, "name": option.name, "color": option.color,
                    "description": option.description,
                })
            })
            .collect::<Vec<_>>();
        options.extend(missing.iter().map(|name| {
            json!({
                "name": name, "color": "GRAY", "description": ""
            })
        }));
        self.graphql(
            graphql::STATUS_OPTIONS_UPDATE,
            json!({"input": {
                "projectId": before.board_id, "fieldId": before.field_id,
                "singleSelectOptions": options,
            }}),
        )
        .await?;
        let after = self.status_snapshot().await?;
        let options_preserved = before
            .options
            .iter()
            .all(|old| after.options.iter().any(|new| new == old));
        let additions_present = missing.iter().all(|wanted| {
            after
                .options
                .iter()
                .any(|option| option.name.as_str().eq_ignore_ascii_case(wanted))
        });
        if !options_preserved || !additions_present || after.assignments != before.assignments {
            let recovery = serde_json::to_string_pretty(&before.assignments).map_err(|error| {
                SourceError::Malformed {
                    message: format!("cannot render pre-write Status recovery snapshot: {error}"),
                }
            })?;
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub changed a pre-existing Status option id, name, color or description, or an item assignment after the guarded update; the pre-write item assignment snapshot is:\n{recovery}"
                ),
            });
        }
        Ok(report)
    }

    /// A fresh snapshot of the Status field and every board item's assignment of it.
    ///
    /// # Errors
    ///
    /// Refuses a board without a single-select `Status` field, and one the token cannot see.
    async fn status_snapshot(&self) -> Result<StatusSnapshot, SourceError> {
        // Status alone, as this operation has always read it: a `Priority` field is another
        // operation's, so nothing about it can refuse this one.
        let mut board = self.board_snapshot(&[BoardField::Status]).await?;
        let field = board
            .fields
            .remove(&BoardField::Status)
            .ok_or_else(|| self.no_status_field())?;
        Ok(StatusSnapshot {
            assignments: board.assignments(BoardField::Status),
            board_id: board.board_id,
            field_id: field.field_id,
            options: field.options,
        })
    }

    /// The refusal a board with no `Status` field is answered with by the guarded setup.
    fn no_status_field(&self) -> SourceError {
        SourceError::Refused {
            message: format!("source {} board has no Status field", self.name),
        }
    }

    // llmlint: ignore-block[changed_behavior_has_e2e] Valid snapshot shapes are exercised through
    // the real CLI loopback journey, including pagination. The individual malformed guards
    // are defensive validation of a schema-pinned third-party response, not separate user
    // journeys; drift and missing-field failures cover the operation's recovery behavior.
    /// A fresh snapshot of each of the `owned` fields on the board, with its options, and of
    /// every board item's value of each, walked to the end of the board's items. A field not
    /// in `owned` is read past whatever it holds.
    async fn board_snapshot(&self, owned: &[BoardField]) -> Result<BoardSnapshot, SourceError> {
        let mut after: Option<String> = None;
        let mut snapshot: Option<BoardSnapshot> = None;
        loop {
            let data = self
                .graphql(
                    graphql::STATUS_OPTIONS_SNAPSHOT,
                    json!({
                        "owner": self.owner, "number": self.project_number,
                        "first": MAX_PAGE_SIZE, "after": after, "nestedFirst": MAX_PAGE_SIZE,
                    }),
                )
                .await?;
            let board = data
                .pointer("/owner/projectV2")
                .filter(|board| board.is_object())
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "source {} has no accessible GitHub Projects board",
                        self.name
                    ),
                })?;
            if board
                .pointer("/fields/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
                != Some(false)
            {
                return Err(SourceError::Malformed {
                    message:
                        "GitHub project fields is incomplete or has malformed pageInfo.hasNextPage"
                            .into(),
                });
            }
            let mut fields = BTreeMap::new();
            // Only the fields this setup owns, by name: a node the single-select fragment did not
            // match carries no name, and a person's own single-select field — a `Size`, a
            // `Team` — is none of this setup's business, so nothing about it can refuse one. A
            // `Status` or `Priority` field without its options is malformed, not absent.
            // llmlint: ignore[boundary_inputs_validated] The field page this loop reads is validated as complete immediately above: any `fields.pageInfo.hasNextPage` other than `false` is refused as malformed before a node is read, so an incomplete page is never taken for the board's whole field set.
            for (owned, field) in board
                .pointer("/fields/nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project fields.nodes is not an array".into(),
                })?
                .iter()
                .filter_map(|field| {
                    let named = BoardField::named(field.get("name")?.as_str()?)?;
                    owned.contains(&named).then_some((named, field))
                })
            {
                let options = field
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub single-select field options is not an array".into(),
                    })?
                    .iter()
                    .map(|option| {
                        Ok(StatusOption {
                            id: StatusOptionId::try_from(required_str(option, "id")?.to_owned())
                                .map_err(|message| SourceError::Malformed { message })?,
                            name: ColumnName::try_from(required_str(option, "name")?.to_owned())
                                .map_err(|message| SourceError::Malformed {
                                    message: format!(
                                        "GitHub single-select option name is invalid: {message}"
                                    ),
                                })?,
                            color: serde_json::from_value(
                                option.get("color").cloned().unwrap_or(Value::Null),
                            )
                            .map_err(|error| {
                                SourceError::Malformed {
                                    message: format!(
                                        "GitHub single-select option color is invalid: {error}"
                                    ),
                                }
                            })?,
                            description: optional_str(option, "description")?
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect::<Result<Vec<_>, SourceError>>()?;
                let snapshot = SnapshotField {
                    field_id: required_nonblank_str(field, "id")?.to_owned(),
                    options,
                };
                // A board's field names are unique, so a second one is an answer that cannot
                // say which field the setup would act on — refused rather than one chosen.
                if fields.insert(owned, snapshot).is_some() {
                    return Err(SourceError::Malformed {
                        message: format!(
                            "GitHub answered two {} fields for this board",
                            owned.name()
                        ),
                    });
                }
            }
            let board_id = required_nonblank_str(board, "id")?.to_owned();
            let current = snapshot.get_or_insert_with(|| BoardSnapshot {
                board_id,
                fields,
                items: Vec::new(),
            });
            let items = board
                .pointer("/items/nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items.nodes is not an array".into(),
                })?;
            for item in items {
                let field_values =
                    item.get("fieldValues")
                        .ok_or_else(|| SourceError::Malformed {
                            message: "GitHub project item is missing fieldValues".into(),
                        })?;
                if field_values
                    .pointer("/pageInfo/hasNextPage")
                    .and_then(Value::as_bool)
                    != Some(false)
                {
                    return Err(SourceError::Malformed {
                        message: "GitHub project item fieldValues is incomplete or has malformed pageInfo.hasNextPage".into(),
                    });
                }
                let values = item
                    .pointer("/fieldValues/nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub project item fieldValues.nodes is not an array".into(),
                    })?;
                let item_id = required_nonblank_str(item, "id")?;
                let mut assigned = BTreeMap::new();
                for value in values {
                    let Some(field) = value
                        .pointer("/field/name")
                        .and_then(Value::as_str)
                        .and_then(BoardField::named)
                        .filter(|field| owned.contains(field))
                    else {
                        continue;
                    };
                    let held = assigned.insert(
                        field,
                        AssignedStatusOption {
                            id: StatusOptionId::try_from(
                                required_str(value, "optionId")?.to_owned(),
                            )
                            .map_err(|message| SourceError::Malformed { message })?,
                            name: ColumnName::try_from(required_str(value, "name")?.to_owned())
                                .map_err(|message| SourceError::Malformed {
                                    message: format!(
                                        "GitHub assigned {} name is invalid: {message}",
                                        field.name()
                                    ),
                                })?,
                        },
                    );
                    // An item holds one value of a field, so a second one leaves no way to
                    // tell which it holds — and a verification or recovery built on either
                    // could restore the wrong one.
                    if held.is_some() {
                        return Err(SourceError::Malformed {
                            message: format!(
                                "GitHub answered two {} values for board item {item_id}",
                                field.name()
                            ),
                        });
                    }
                }
                current.items.push((item_id.to_owned(), assigned));
            }
            let page = board.get("items").ok_or_else(|| SourceError::Malformed {
                message: "GitHub project is missing items".into(),
            })?;
            let has_next = page
                .pointer("/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items.pageInfo.hasNextPage is not a boolean".into(),
                })?;
            if !has_next {
                break;
            }
            let next =
                required_nonblank_str(page.get("pageInfo").unwrap_or(&Value::Null), "endCursor")?;
            validate_cursor_progress(after.as_deref(), next)?;
            after = Some(next.to_owned());
        }
        snapshot.ok_or_else(|| SourceError::Malformed {
            message: "GitHub returned no board field snapshot".into(),
        })
    }
    // llmlint: ignore-end[changed_behavior_has_e2e]

    /// Report every board field this source's configuration names and, with
    /// [`SetupMode::Apply`], set each up: add the options a field lacks, and create
    /// the `Priority` field when the board has none.
    ///
    /// The fields are `Status`, always, with the options `status_mapping` resolves to; and
    /// `Priority`, when `priority_mapping` is set, with its four mapped options — created in
    /// the order urgent, high, medium, low. An option a field already has keeps its id, name,
    /// color and description: the whole option list goes back with every existing id, because
    /// a re-minted id clears every item's value.
    ///
    /// # Errors
    ///
    /// Refuses a board without a single-select `Status` field. After an apply the board is
    /// read again, and a pre-existing option or any item's value of either field that moved is
    /// refused with the complete pre-write assignments in the diagnostic, for recovery.
    // llmlint: ignore[changed_behavior_has_e2e] The `sources fields` journeys drive plan,
    // unchanged apply, a created field, an added option to each field, drift refusal, a board
    // with no Status field and a non-github-projects source through the compiled CLI against
    // the loopback board. Transport errors are the shared `graphql` boundary's behavior.
    pub async fn fields(&self, mode: SetupMode) -> Result<FieldsReport, SourceError> {
        let owned: Vec<BoardField> = if self.priorities.is_some() {
            vec![BoardField::Status, BoardField::Priority]
        } else {
            vec![BoardField::Status]
        };
        let before = self.board_snapshot(&owned).await?;
        let mut plans = vec![FieldPlan {
            field: BoardField::Status,
            wanted: self.statuses.wanted(),
        }];
        if !before.fields.contains_key(&BoardField::Status) {
            return Err(self.no_status_field());
        }
        if let Some(mapping) = &self.priorities {
            plans.push(FieldPlan {
                field: BoardField::Priority,
                wanted: mapping.names().map(str::to_owned).collect(),
            });
        }
        // The snapshot reads single-select fields alone, so a field it did not find may still
        // be on the board under the name, of another type: creating one beside it would fail
        // part way, or leave two fields of one name. Asked of the board's own field list, and
        // only when a field is missing.
        if plans
            .iter()
            .any(|plan| !before.fields.contains_key(&plan.field))
        {
            let board = self.board_fields().await?;
            for plan in plans
                .iter()
                .filter(|plan| !before.fields.contains_key(&plan.field))
            {
                if let Some(field) = Board::field(&board.fields, plan.field.name())? {
                    return Err(SourceError::Refused {
                        message: format!(
                            "source {}'s board has a {} field that is not a single-select field \
                             (it is a {}), so it cannot hold this source's options; next: rename \
                             or remove that field, then run this again",
                            self.name,
                            plan.field.name(),
                            optional_str(field, "__typename")?.unwrap_or("field of another type")
                        ),
                    });
                }
            }
        }
        let mut reports = Vec::new();
        for plan in &plans {
            let held = before.fields.get(&plan.field);
            let existing = held.map(|field| field.options.clone()).unwrap_or_default();
            let mut missing: Vec<String> = Vec::new();
            for wanted in &plan.wanted {
                let present = existing
                    .iter()
                    .any(|option| option.name.as_str().eq_ignore_ascii_case(wanted))
                    || missing
                        .iter()
                        .any(|named| named.eq_ignore_ascii_case(wanted));
                if !present {
                    missing.push(wanted.clone());
                }
            }
            let kinds = match plan.field {
                BoardField::Status => self.statuses.missing_by_kind(&existing),
                BoardField::Priority => Vec::new(),
            };
            reports.push(FieldReport {
                field: plan.field,
                exists: held.is_some(),
                kinds,
                outcome: match (mode, held.is_some(), missing.is_empty()) {
                    (SetupMode::Plan, _, _) => FieldOutcome::Planned,
                    (SetupMode::Apply, true, true) => FieldOutcome::Unchanged,
                    (SetupMode::Apply, true, false) => FieldOutcome::Applied,
                    (SetupMode::Apply, false, _) => FieldOutcome::Created,
                },
                missing,
                existing,
            });
        }
        let report = FieldsReport {
            source: self.name.clone(),
            fields: reports,
        };
        let writes: Vec<&FieldReport> = report
            .fields
            .iter()
            .filter(|field| !field.missing.is_empty() || !field.exists)
            .collect();
        if mode == SetupMode::Plan || writes.is_empty() {
            return Ok(report);
        }
        let mut landed: Vec<&str> = Vec::new();
        for field in &writes {
            let added = field
                .missing
                .iter()
                .map(|name| json!({"name": name, "color": "GRAY", "description": ""}));
            let sent = match before.fields.get(&field.field) {
                Some(held) => {
                    let mut options = held
                        .options
                        .iter()
                        .map(|option| {
                            json!({
                                "id": option.id, "name": option.name, "color": option.color,
                                "description": option.description,
                            })
                        })
                        .collect::<Vec<_>>();
                    options.extend(added);
                    self.graphql(
                        graphql::STATUS_OPTIONS_UPDATE,
                        json!({"input": {
                            "projectId": before.board_id, "fieldId": held.field_id,
                            "singleSelectOptions": options,
                        }}),
                    )
                    .await
                }
                None => {
                    self.graphql(
                        graphql::CREATE_FIELD,
                        json!({"input": {
                            "projectId": before.board_id, "dataType": "SINGLE_SELECT",
                            "name": field.field.name(),
                            "singleSelectOptions": added.collect::<Vec<_>>(),
                        }}),
                    )
                    .await
                }
            };
            // A mutation that failed does not establish that GitHub left its field as it was,
            // so every failure from here on carries the recovery data a drift refusal does.
            match sent {
                Ok(_) => landed.push(field.field.name()),
                Err(error) => {
                    let changed = if landed.is_empty() {
                        String::new()
                    } else {
                        format!("changed the {} field and then ", landed.join(" and "))
                    };
                    return Err(SourceError::Refused {
                        message: format!(
                            "the guarded field setup {changed}failed on the {} field, which it may \
                             have changed part way: {error}; the pre-write item assignments \
                             are:\n{}",
                            field.field.name(),
                            recovery(&report, &before)?
                        ),
                    });
                }
            }
        }
        // The board has been written, so a verification read that fails leaves it unverified
        // rather than unchanged, and says what to put back.
        let after = match self.board_snapshot(&owned).await {
            Ok(after) => after,
            Err(error) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "the guarded field setup changed the {} field and then could not read the \
                         board back to verify it: {error}; the pre-write item assignments are:\n{}",
                        landed.join(" and "),
                        recovery(&report, &before)?
                    ),
                });
            }
        };
        let mut moved = Vec::new();
        for field in &report.fields {
            let name = field.field.name();
            let now = after
                .fields
                .get(&field.field)
                .map(|held| held.options.as_slice())
                .unwrap_or_default();
            if !field.existing.iter().all(|old| now.contains(old)) {
                moved.push(format!(
                    "a pre-existing {name} option id, name, color or description"
                ));
            }
            if !field.missing.iter().all(|wanted| {
                now.iter()
                    .any(|option| option.name.as_str().eq_ignore_ascii_case(wanted))
            }) {
                moved.push(format!("an added {name} option"));
            }
            if after.assignments(field.field) != before.assignments(field.field) {
                moved.push(format!("an item's {name} value"));
            }
        }
        if !moved.is_empty() {
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub changed {} after the guarded field setup; the pre-write item \
                     assignments are:\n{}",
                    moved.join(", "),
                    recovery(&report, &before)?
                ),
            });
        }
        Ok(report)
    }

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
            statuses: BoardStatuses::resolve(&config.status_mapping, name)?,
            priorities: config
                .priority_mapping
                .map(|mapping| PriorityMapping::resolve(mapping, name))
                .transpose()?,
            client: Client::builder()
                .user_agent("onetaskgraph")
                .build()
                .map_err(|e| SourceError::Config {
                    message: format!("cannot build HTTP client: {e}"),
                })?,
            created: Mutex::new(Vec::new()),
            updated: Mutex::new(Vec::new()),
            pacing: Pacing::resolve(config.pacing, name)?,
            last_mutation: Mutex::new(None),
            board_cache: Mutex::new(None),
            search_cache: Mutex::new(None),
            narrowed_cache: Mutex::new(BTreeMap::new()),
            resolved_cache: Mutex::new(BTreeMap::new()),
            search_next: Mutex::new(BTreeMap::new()),
            fields_cache: Mutex::new(None),
            repository_cache: Mutex::new(BTreeMap::new()),
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
        if is_mutation(query)
            && ![
                graphql::ADD_COMMENT,
                graphql::UPDATE_COMMENT,
                graphql::DELETE_COMMENT,
            ]
            .contains(&query)
        {
            let mut cache = self.resolved_cache()?;
            for argument in ["input", "second", "third", "clear"] {
                if let Some(input) = variables.get(argument) {
                    cache.retain(|id, item| {
                        !["id", "issueId", "subjectId", "itemId"].iter().any(|key| {
                            input
                                .get(key)
                                .and_then(Value::as_str)
                                .is_some_and(|value| value == id.0 || value == item.item_id)
                        })
                    });
                }
            }
        }
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
    /// anybody remembering to count it, and
    /// `the_session_report_counts_every_request_the_board_served_and_what_each_cost` fails
    /// when one is not.
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
    /// of this board. That answer is given about an **exhausted** connection and never
    /// about an unread page: the entry is looked for on the page in hand, and only if that
    /// page reports more of the connection, in [`Self::board_membership`]'s walk of the
    /// rest of it.
    async fn resolve_issue(&self, issue: &Value) -> Result<Option<Resolved>, SourceError> {
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
        let held = match self.board_entry(nodes) {
            Some(held) => held.clone(),
            None => {
                let info = memberships
                    .get("pageInfo")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub issue projectItems has no pageInfo".into(),
                    })?;
                // The page held no entry for this board. Whether that means the issue is
                // not on it is a question about the rest of the connection, and only a
                // connection with no rest answers it here.
                if !required_bool(info, "hasNextPage")? {
                    return Ok(None);
                }
                let cursor = required_str(info, "endCursor")?;
                validate_cursor_progress(None, cursor)?;
                let issue_id = required_str(issue, "id")?;
                match self.board_membership(issue_id, cursor).await? {
                    Some(held) => held,
                    None => return Ok(None),
                }
            }
        };
        let item = json!({
            "id": required_str(&held, "id")?,
            "project": held.get("project"),
            "fieldValues": held.get("fieldValues"),
            "content": issue,
        });
        self.resolve(&item)
    }

    /// This board's own entry among one page of an issue's `Issue.projectItems`.
    ///
    /// One spelling of *which membership is this board's*, so the page a read carries and
    /// the pages [`Self::board_membership`] walks are searched by the same rule.
    fn board_entry<'a>(&self, nodes: &'a [Value]) -> Option<&'a Value> {
        nodes.iter().find(|node| {
            node.pointer("/project/number").and_then(Value::as_u64)
                == Some(u64::from(self.project_number))
        })
    }

    /// The rest of one issue's board memberships, from `after`, for this board's entry.
    ///
    /// The recovery read: a page of memberships that holds no entry for this board says
    /// nothing about the memberships past it, so the connection is walked to exhaustion
    /// before an issue is reported as one this board does not hold. `Ok(None)` is that
    /// positive answer — the whole connection was read and no entry named this board —
    /// rather than a failure, and the walk is held to
    /// [`validate_cursor_progress`] like every other page walk here, so a source answering
    /// with a cursor that does not advance is refused instead of spun on.
    async fn board_membership(
        &self,
        issue: &str,
        after: &str,
    ) -> Result<Option<Value>, SourceError> {
        let mut after = after.to_owned();
        loop {
            let data = self
                .graphql(
                    graphql::ISSUE_BOARD_ITEMS,
                    json!({"id":issue,"first":MAX_PAGE_SIZE,"after":after,
                           "nestedFirst":NESTED_PAGE_SIZE}),
                )
                .await?;
            let Some(connection) = data
                .pointer("/node/projectItems")
                .filter(|value| !value.is_null())
            else {
                // The id resolved to nothing, or to something with no memberships to walk —
                // which is the same answer as a connection holding no entry for this board.
                return Ok(None);
            };
            let nodes = connection
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub issue projectItems.nodes is not an array".into(),
                })?;
            if let Some(held) = self.board_entry(nodes) {
                return Ok(Some(held.clone()));
            }
            let info = connection
                .get("pageInfo")
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub issue projectItems has no pageInfo".into(),
                })?;
            let next = required_bool(info, "hasNextPage")?
                .then(|| required_str(info, "endCursor"))
                .transpose()?;
            match next {
                Some(next) => {
                    validate_cursor_progress(Some(&after), next)?;
                    after = next.to_owned();
                }
                None => return Ok(None),
            }
        }
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
            if let Some(resolved) = self.resolve_issue(node).await? {
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

    /// Every issue this board holds, completed with what this run wrote.
    ///
    /// The completion is not an optimisation and it is not a cache: GitHub's issue search
    /// is an index and is eventually consistent, so an issue this run created seconds ago
    /// can be absent from it, and a project listed straight after being written would
    /// otherwise be missing from its own board. What is added back is only what this
    /// process itself wrote, out of [`Self::created`], which lives and dies with the
    /// process.
    async fn board_issues(&self) -> Result<Vec<Resolved>, SourceError> {
        let found = self.searched_issues().await?;
        self.completed_with_written(found, |_| true)
    }

    /// Every issue this board's own search reports, walked to exhaustion, read once per
    /// source.
    ///
    /// The uncompleted half of [`Self::board_issues`], separated because [`Self::board`]
    /// needs it too and the two would otherwise walk the same search twice in one command.
    /// See [`Self::search_cache`] for why holding it is the same bargain holding the board
    /// is.
    async fn searched_issues(&self) -> Result<Vec<Resolved>, SourceError> {
        let cached = self.search_cache()?.clone();
        if let Some(held) = cached {
            return Ok(held);
        }
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
        *self.search_cache()? = Some(found.clone());
        Ok(found)
    }

    /// This process's own view of the board's issues, or the refusal a poisoned lock is.
    fn search_cache(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<Vec<Resolved>>>, SourceError> {
        self.search_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's view of the board's issues was left inconsistent by an \
                      earlier failure; next: run the command again"
                    .into(),
            })
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
    /// two: it is not an issue, so the issue fragment reads nothing of it, and a read of one
    /// is completed by a read of the draft itself rather than reported as nothing.
    async fn reach(&self, id: &NativeId) -> Result<Reached, SourceError> {
        let asked = self
            .graphql(
                graphql::ISSUE,
                json!({"id":id.0,"first":MAX_PAGE_SIZE,"nestedFirst":NESTED_PAGE_SIZE,
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
        Ok(match self.resolve_issue(node).await? {
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
            Reached::Draft => self.draft_by_id(id).await,
        }
    }

    /// Several items of this board, each by its own id, in order — what [`Self::item_by_id`]
    /// answers for each, read [`DETAIL_BATCH`] at a time with [`graphql::ISSUE_DETAILS`] rather
    /// than one request per id.
    ///
    /// What this run wrote answers first, as it does there, and only the rest is read. One id
    /// left to read is read by [`Self::item_by_id`] itself, which costs what a batch does. A
    /// batch GitHub refuses because one of its ids resolves to no node at all is read again one
    /// id at a time, so that id is answered as not held and the others as themselves; a draft
    /// is completed by a read of the draft, exactly as there.
    async fn items_by_ids(&self, ids: &[NativeId]) -> Result<Vec<Option<Resolved>>, SourceError> {
        let mut found: Vec<Option<Option<Resolved>>> = {
            let created = self.created()?;
            ids.iter()
                .map(|id| {
                    created
                        .iter()
                        .find(|own| own.id == *id)
                        .map(|own| Some(own.clone()))
                })
                .collect()
        };
        let unread: Vec<NativeId> = ids
            .iter()
            .zip(&found)
            .filter(|(_, found)| found.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        let mut read = Vec::with_capacity(unread.len());
        if let [one] = unread.as_slice() {
            read.push(self.item_by_id(one).await?);
        } else {
            for batch in unread.chunks(DETAIL_BATCH) {
                let data = match self
                    .graphql(graphql::ISSUE_DETAILS, detail_batch(batch, None))
                    .await
                {
                    Ok(data) => data,
                    Err(error) if unresolvable_node(&error) => {
                        for id in batch {
                            read.push(self.item_by_id(id).await?);
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                for (slot, id) in batch.iter().enumerate() {
                    let node =
                        data.get(format!("i{slot}"))
                            .ok_or_else(|| SourceError::Malformed {
                                message: format!(
                                    "GitHub answered a batch read with no item for {}",
                                    id.0
                                ),
                            })?;
                    read.push(if node.is_null() {
                        None
                    } else if optional_str(node, "__typename")? == Some("DraftIssue") {
                        self.draft_by_id(id).await?
                    } else {
                        if optional_str(node, "__typename")? == Some("Issue")
                            && required_str(node, "id")? != id.0
                        {
                            return Err(SourceError::Malformed {
                                message: format!(
                                    "GitHub answered the read of {} with issue {}",
                                    id.0,
                                    required_str(node, "id")?
                                ),
                            });
                        }
                        self.resolve_issue(node).await?
                    });
                }
            }
        }
        let mut read = read.into_iter();
        Ok(found
            .iter_mut()
            .map(|slot| slot.take().unwrap_or_else(|| read.next().flatten()))
            .collect())
    }

    fn resolved_cache(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<NativeId, Resolved>>, SourceError> {
        self.resolved_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "resolved item records were left inconsistent; run the command again"
                    .into(),
            })
    }

    /// Reuse a record this invocation already resolved. The mutation sender invalidates
    /// it before writing, so a partial failure cannot leave a pre-write binding behind.
    async fn bound_item(&self, id: &NativeId) -> Result<Option<Resolved>, SourceError> {
        let cached = self.resolved_cache()?.get(id).cloned();
        match cached {
            Some(item) => Ok(Some(item)),
            None => self.item_by_id(id).await,
        }
    }

    /// One board draft by its own id, with the board item it sits in — or `None` when no
    /// item of this board is that draft's.
    ///
    /// The same decision [`Self::resolve_issue`] makes for an issue, over the draft's own
    /// `projectV2Items`: an entry naming this board is what makes it this board's. GitHub
    /// links a draft to one board item, so the page this read carries is the whole of that
    /// connection, and a page that reports more than it holds is refused rather than read
    /// as an answer about memberships nobody read.
    async fn draft_by_id(&self, id: &NativeId) -> Result<Option<Resolved>, SourceError> {
        let data = self
            .graphql(
                graphql::DRAFT,
                json!({"id":id.0,"nestedFirst":NESTED_PAGE_SIZE,
                       "boardItems":BOARD_ITEMS_PAGE_SIZE}),
            )
            .await?;
        // Gone between the two reads is an answer — the draft is no longer there. Anything
        // else than the draft [`Self::reach`] was just told this id is, is not one.
        let Some(draft) = data.get("node").filter(|node| !node.is_null()) else {
            return Ok(None);
        };
        if optional_str(draft, "__typename")? != Some("DraftIssue") {
            return Err(SourceError::Malformed {
                message: format!(
                    "GitHub answered {} as a draft and then as something else",
                    id.0
                ),
            });
        }
        if required_str(draft, "id")? != id.0 {
            return Err(SourceError::Malformed {
                message: format!("GitHub answered a different draft for {}", id.0),
            });
        }
        let memberships = draft
            .get("projectV2Items")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub draft {} is missing projectV2Items", id.0),
            })?;
        let nodes = memberships
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub draft {} projectV2Items.nodes is not an array", id.0),
            })?;
        let info = memberships
            .get("pageInfo")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub draft {} projectV2Items has no pageInfo", id.0),
            })?;
        // Read whether or not this board's entry is on the page: a page claiming more than
        // the one item GitHub links a draft to is a malformed answer either way.
        if required_bool(info, "hasNextPage")? || nodes.len() > 1 {
            return Err(SourceError::Malformed {
                message: format!(
                    "GitHub draft {} reports more board items than the one GitHub links a draft \
                     to",
                    id.0
                ),
            });
        }
        if let Some(node) = nodes.first()
            && node
                .pointer("/project/number")
                .and_then(Value::as_u64)
                .is_none()
        {
            return Err(SourceError::Malformed {
                message: format!(
                    "GitHub draft {} board item has no numeric project number",
                    id.0
                ),
            });
        }
        let Some(held) = self.board_entry(nodes) else {
            return Ok(None);
        };
        if required_str(
            held.get("project").ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub draft {} board item has no project", id.0),
            })?,
            "id",
        )? != self.board_fields().await?.id.as_str()
        {
            return Ok(None);
        }
        let item = json!({
            "id": required_str(held, "id")?,
            "project": held.get("project"),
            "fieldValues": held.get("fieldValues"),
            "content": draft,
        });
        self.resolve(&item)
    }

    /// The board's own id and field definitions, for a write whose item does not carry
    /// them — never its items.
    ///
    /// A board this command has already listed supplies them, since it read them beside its
    /// items; otherwise they come from [`graphql::BOARD_FIELDS`], once per command. Neither
    /// is consulted about which items the board holds: see the module documentation for
    /// why a question about one known item is answered by reading that item.
    async fn board_fields(&self) -> Result<BoardFields, SourceError> {
        if let Some(board) = self.board_cache()?.as_ref() {
            return Ok(BoardFields {
                id: BoardId::parse(&board.id)?,
                fields: board.fields.clone(),
            });
        }
        if let Some(held) = self.fields_cache()?.clone() {
            return Ok(held);
        }
        let data = self
            .graphql(
                graphql::BOARD_FIELDS,
                json!({"owner":self.owner,"number":self.project_number,
                       "nestedFirst":NESTED_PAGE_SIZE}),
            )
            .await?;
        self.fields_read(&data)
    }

    /// The board's id and fields out of an answer carrying the `boardFields` root, held for
    /// the rest of this command.
    fn fields_read(&self, data: &Value) -> Result<BoardFields, SourceError> {
        let board = data
            .pointer("/boardFields/projectV2")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub project {}/{} was not found or is not visible to the token",
                    self.owner, self.project_number
                ),
            })?;
        let read = BoardFields {
            id: BoardId::parse(required_str(board, "id")?)?,
            fields: board.get("fields").cloned().unwrap_or(Value::Null),
        };
        *self.fields_cache()? = Some(read.clone());
        Ok(read)
    }

    /// Read what creating an issue in `repository` needs and this command has not read yet —
    /// the board's fields and the repository's node id — in one request when it needs both.
    ///
    /// When either is already known this sends nothing, and the other is read by its own
    /// document where it is asked for, so no create reads anything twice.
    async fn creation_context(
        &self,
        repository: &RepositoryTarget,
        incoming: &Incoming<'_>,
    ) -> Result<(), SourceError> {
        let fields_known = self.board_cache()?.is_some() || self.fields_cache()?.is_some();
        if fields_known || self.repository_cache()?.contains_key(repository) {
            return Ok(());
        }
        let data = self
            .graphql(
                graphql::CREATION_CONTEXT,
                json!({"owner":self.owner,"number":self.project_number,
                       "nestedFirst":NESTED_PAGE_SIZE,"repositoryOwner":repository.owner,
                       "repositoryName":repository.name}),
            )
            .await?;
        self.fields_read(&data)?;
        self.repository_read(&data, repository, incoming)?;
        Ok(())
    }

    /// This process's own view of the board's fields, or the refusal a poisoned lock is.
    fn fields_cache(&self) -> Result<std::sync::MutexGuard<'_, Option<BoardFields>>, SourceError> {
        self.fields_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's view of the board's fields was left inconsistent by an \
                      earlier failure; next: run the command again"
                    .into(),
            })
    }

    /// What a write to `item` needs of the board, read off that item when it says enough and
    /// off [`Self::board_fields`] when it does not.
    ///
    /// A node read of an item names its board and carries the definition of every field it
    /// holds a value of — so an item naming its board, holding a value of the origin field,
    /// and, when the write carries a status, holding a `Status` value, needs no read of the
    /// board at all. **Nothing the item does not say is guessed:** a field it holds no value
    /// of may still be on the board, and a view reading it as absent would refuse a write the
    /// board can take or skip a field write the board needs, so such an item — and a create,
    /// which has no item yet — takes the board's fields from their own read instead.
    async fn fields_for(
        &self,
        item: Option<&Resolved>,
        writes_status: bool,
        selects_priority: bool,
    ) -> Result<BoardFields, SourceError> {
        if let Some(board) = item.and_then(Resolved::carried_board) {
            return Ok(board);
        }
        if let Some(item) = item
            && let Some(board_id) = item.named_board()
            && item.defines(ORIGIN_FIELD)
            && (!writes_status || item.defines("Status"))
            && (!selects_priority || item.defines(PRIORITY_FIELD))
        {
            return Ok(BoardFields {
                id: board_id,
                fields: json!({"nodes": item.fields, "pageInfo": {"hasNextPage": false}}),
            });
        }
        self.board_fields().await
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
                if let Some(resolved) = self.resolve_issue(node).await? {
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
        let mut after = None;
        loop {
            let (candidates, next) = self
                .search_page(&search, SEARCH_PAGE_SIZE, after.as_deref())
                .await?;
            if let Some(item) = candidates.into_iter().find(|item| {
                item.kind == BoardKind::Work(ItemKind::Project)
                    && item.title.eq_ignore_ascii_case(name)
            }) {
                return Ok(Some(item.id));
            }
            match next {
                Some(next) => after = Some(next),
                None => return Ok(None),
            }
        }
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

    /// Every issue of this board GitHub's issue search reports updated at or after `since`,
    /// completed with what this run wrote — the candidates a comment-activity read confirms.
    ///
    /// Scoped by the board and by nothing else: `project:<owner>/<number>` reaches every issue
    /// on the board whatever repository, and whatever owner, it lives in, so no repository or
    /// owner qualifier is added and none is needed. What makes the `updated:` qualifier
    /// sufficient is a fact about GitHub rather than about this source: a comment written on an
    /// issue **and a comment edited on it** both move that issue's `updatedAt`. The credentialed
    /// journey `an_edited_comment_moves_its_issue_and_is_selected_since` in `tests/journey`
    /// re-takes that fact on every run of the lane, so a change on GitHub's side fails there
    /// rather than silently narrowing a caller's answer.
    ///
    /// The instant is written to the second, rounded down, which can only widen what the
    /// search returns; confirmation against each candidate's own comments is what makes the
    /// answer exact. The search is an index that lags a write by a second or two — the module
    /// documentation records it — so a caller that asks again from its last instant should
    /// overlap the two by more than that.
    async fn updated_since(&self, since: DateTime<Utc>) -> Result<Vec<Resolved>, SourceError> {
        let found = self.searched(&updated_qualifier(since)).await?;
        self.completed_with_written(found, |_| true)
    }

    /// Every issue of this board GitHub's issue search reports for the board-scoped search
    /// narrowed by `also`, in pages of [`SEARCH_PAGE_SIZE`].
    ///
    /// Uncompleted: what this process wrote is added by the caller, which knows whether its
    /// own record is the fresher of the two.
    async fn searched(&self, also: &str) -> Result<Vec<Resolved>, SourceError> {
        let search = self.board_search(Some(also));
        let mut after: Option<String> = None;
        let mut found = Vec::new();
        loop {
            let (page, next) = self
                .search_page(&search, SEARCH_PAGE_SIZE, after.as_deref())
                .await?;
            found.extend(page);
            match next {
                Some(next) => after = Some(next),
                None => return Ok(found),
            }
        }
    }

    /// A bounded task answer; the versioned cursor carries the connection position, how
    /// many rows of the page starting there were already handed out, and the own-write ids
    /// already observed, including across a new source instance.
    ///
    /// Every page is sent at [`SEARCH_PAGE_SIZE`] whatever the caller's limit, and a limit is
    /// sliced from the pages it needs; why is the module documentation's paging contract.
    async fn search_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
        also: &str,
    ) -> Result<Page<Task>, SourceError> {
        let mut position = match &page.cursor {
            None => SearchPosition::default(),
            Some(cursor) => serde_json::from_str::<SearchPosition>(&cursor.0)
                .ok()
                .filter(|position| {
                    position.version == SEARCH_CURSOR_VERSION
                        && position.connection.valid_resume(position.offset)
                })
                .ok_or_else(|| SourceError::Config {
                    message: "page cursor is invalid".into(),
                })?,
        };
        let search = self.board_search(Some(also));
        let limit = page.limit.min(MAX_PAGE_SIZE) as usize;
        let own = self.with_own_writes(Vec::new())?;
        for item in &own {
            if !position.own.contains(&item.id) {
                position.own.push(item.id.clone());
            }
        }
        let mut tasks = Vec::new();
        while !position.connection.exhausted() && tasks.len() < limit {
            let first = SEARCH_PAGE_SIZE;
            // Page size is part of the key: a short cached answer cannot answer a wider ask.
            let key =
                serde_json::to_string(&("page", &search, &position.connection.after(), first))
                    .expect("search page key is serializable");
            let cached = if query.commented_since.is_none() {
                self.narrowed_cache()?.get(&key).cloned()
            } else {
                None
            };
            let (found, next) = match cached {
                Some(found) => {
                    let next = self
                        .search_next
                        .lock()
                        .map_err(|_| SourceError::Unavailable {
                            message:
                                "search pagination was left inconsistent; run the command again"
                                    .into(),
                        })?
                        .get(&key)
                        .cloned()
                        .flatten();
                    (found, next)
                }
                None => {
                    let (found, next) = self
                        .search_page(&search, first, position.connection.after())
                        .await?;
                    if query.commented_since.is_none() {
                        self.search_next
                            .lock()
                            .map_err(|_| SourceError::Unavailable {
                                message:
                                    "search pagination was left inconsistent; run the command again"
                                        .into(),
                            })?
                            .insert(key.clone(), next.clone());
                        self.narrowed_cache()?.insert(key, found.clone());
                    }
                    (found, next)
                }
            };
            let rows = found.len();
            for mut item in found.into_iter().skip(position.offset) {
                if tasks.len() == limit {
                    break;
                }
                position.offset += 1;
                if position.own.contains(&item.id) {
                    if position.seen.contains(&item.id) {
                        continue;
                    }
                    position.seen.push(item.id.clone());
                    let updated_at = item.updated_at;
                    let Some(written) = self.search_written(&own, &item.id).await? else {
                        continue;
                    };
                    item = written;
                    item.updated_at = item.updated_at.max(updated_at);
                    self.resolved_cache()?.insert(item.id.clone(), item.clone());
                }
                if item.kind == BoardKind::Work(ItemKind::Task) {
                    let task = item.task()?;
                    if task_matches(&task, query, &query.project)
                        && self.commented_since(&item, query.commented_since).await?
                    {
                        tasks.push(task);
                    }
                }
            }
            if position.offset < rows {
                continue;
            }
            position.offset = 0;
            position.connection = match next {
                Some(after) => SearchConnection::Continuing {
                    after: Cursor(after),
                },
                None => SearchConnection::Exhausted {},
            };
        }
        if position.connection.exhausted() {
            for id in position.own.clone() {
                if position.seen.contains(&id) {
                    continue;
                }
                if tasks.len() == limit {
                    break;
                }
                position.seen.push(id.clone());
                let Some(item) = self.search_written(&own, &id).await? else {
                    continue;
                };
                if item.kind == BoardKind::Work(ItemKind::Task) {
                    let task = item.task()?;
                    if task_matches(&task, query, &query.project)
                        && self.commented_since(&item, query.commented_since).await?
                    {
                        tasks.push(task);
                    }
                }
            }
        }
        let more = !position.connection.exhausted()
            || position.own.iter().any(|id| !position.seen.contains(id));
        Ok(Page {
            items: tasks,
            next: more.then(|| {
                Cursor(serde_json::to_string(&position).expect("search position is serializable"))
            }),
        })
    }

    /// A resumed process has the ids but no write records; resolve only a record the
    /// current page needs, by its uncached node read rather than the lagging search index.
    async fn search_written(
        &self,
        own: &[Resolved],
        id: &NativeId,
    ) -> Result<Option<Resolved>, SourceError> {
        match own.iter().find(|item| item.id == *id) {
            Some(item) => Ok(Some(item.clone())),
            None => self.item_by_id(id).await,
        }
    }

    /// The candidates for a task query carrying a text, metadata or origin predicate, read
    /// without enumerating the board — or `None` for a query carrying none of the three, which
    /// keeps the reads it always had.
    ///
    /// An origin is answered by [`Self::origin_candidates`], whatever else the query carries,
    /// because it names at most a handful of items. Text and metadata are answered by one
    /// board-scoped issue search carrying every term — see [`narrowing_qualifiers`] — narrowed
    /// further by `updated:>=` when the query also asks for comment activity, since both
    /// qualifiers must hold of an issue the answer keeps. Every candidate is confirmed in
    /// process afterwards by the same predicates [`task_matches`] applies to every read.
    ///
    /// Completed with what this process wrote, its own record winning over the index's copy
    /// of the same item: see [`Self::with_own_writes`].
    async fn narrowed(&self, query: &TaskQuery) -> Result<Option<Vec<Resolved>>, SourceError> {
        let asked = match (&query.origin, narrowing_qualifiers(query)) {
            (Some(origin), _) => Narrowing::Origin(origin.clone()),
            (None, Some(qualifiers)) => Narrowing::Search(match query.commented_since {
                Some(since) => format!("{} {qualifiers}", updated_qualifier(since)),
                None => qualifiers,
            }),
            (None, None) => return Ok(None),
        };
        // A question about comment activity is asked afresh every time, as it always was: it
        // is the one a caller polls from one source while waiting for the index, and an
        // answer held from the first poll would be the answer to every later one.
        let key = query.commented_since.is_none().then(|| asked.key());
        let cached = match &key {
            Some(key) => self.narrowed_cache()?.get(key).cloned(),
            None => None,
        };
        let found = match cached {
            Some(found) => found,
            None => {
                let found = match &asked {
                    Narrowing::Origin(origin) => self.origin_candidates(origin).await?,
                    Narrowing::Search(also) => self.searched(also).await?,
                };
                if let Some(key) = key {
                    self.narrowed_cache()?.insert(key, found.clone());
                }
                found
            }
        };
        self.with_own_writes(found).map(Some)
    }

    /// The candidates for a project or unscoped document query carrying a searchable text,
    /// read without enumerating the board — or `None` for a query with no text or a blank one,
    /// which keeps the read it always had.
    ///
    /// The text is sent as the very phrase a task query's text is — see [`text_qualifiers`] —
    /// in one board-scoped issue search walked to its end at [`SEARCH_PAGE_SIZE`], so what it
    /// costs is the issues that match and never the board. Its answer is held for the command
    /// under the same key [`Self::narrowed`] holds that search under, so a walk of the caller's
    /// pages asks GitHub once. Every candidate is confirmed afterwards by its kind and by the
    /// substring rule, exactly as an item of the wider read was, and is completed with what this
    /// process wrote: see [`Self::with_own_writes`].
    async fn text_searched(
        &self,
        text: Option<&TextQuery>,
    ) -> Result<Option<Vec<Resolved>>, SourceError> {
        let Some(also) = text_qualifiers(text) else {
            return Ok(None);
        };
        let key = Narrowing::Search(also.clone()).key();
        let cached = self.narrowed_cache()?.get(&key).cloned();
        let found = match cached {
            Some(found) => found,
            None => {
                let found = self.searched(&also).await?;
                self.narrowed_cache()?.insert(key, found.clone());
                found
            }
        };
        self.with_own_writes(found).map(Some)
    }

    /// Every item of this board that may carry `origin` — a superset of those that do — found
    /// by [`graphql::ORIGIN_LOOKUP`] and never by enumerating the board.
    ///
    /// The union of the board's own field filter over the `onetaskgraph.origin` text field —
    /// which reads the field every carrier holds, whichever release wrote it — and the
    /// board-scoped issue search for the same id as a phrase in the body, where this source
    /// mirrors it. The caller adds the third read, this process's own writes. Candidates are
    /// returned unconfirmed; [`task_matches`] compares each one's own origin field with the
    /// query's, exactly.
    ///
    /// Both connections are walked to exhaustion, each from its own cursor. One that has
    /// already ended is sent its last cursor again, which answers an empty page, so the one
    /// document serves every page of either. What the two leave is stated in the module
    /// documentation: a carrier another process added within the last second or two, before
    /// either index has it.
    async fn origin_candidates(&self, origin: &str) -> Result<Vec<Resolved>, SourceError> {
        let filter = format!("{ORIGIN_FIELD}:{}", quoted(origin));
        let search = self.board_search(Some(&format!("in:body {}", quoted(&as_stored(origin)))));
        let mut items_after: Option<String> = None;
        let mut search_after: Option<String> = None;
        let mut found: Vec<Resolved> = Vec::new();
        let keep = |resolved: Resolved, found: &mut Vec<Resolved>| {
            if !found.iter().any(|held| held.id == resolved.id) {
                found.push(resolved);
            }
        };
        loop {
            let data = self
                .graphql(
                    graphql::ORIGIN_LOOKUP,
                    json!({"owner":self.owner,"number":self.project_number,"filter":filter,
                           "search":search,"type":"ISSUE","originFirst":ORIGIN_PAGE_SIZE,
                           "itemsAfter":items_after,"searchAfter":search_after,
                           "nestedFirst":NESTED_PAGE_SIZE,"boardItems":BOARD_ITEMS_PAGE_SIZE,
                           "duplicates":true}),
                )
                .await?;
            let items = data
                .pointer("/originItems/projectV2/items")
                .filter(|value| !value.is_null())
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "GitHub project {}/{} was not found or is not visible to the token",
                        self.owner, self.project_number
                    ),
                })?;
            for item in optional_nodes(Some(items), "project items")?
                .into_iter()
                .flatten()
            {
                // The board's own items list its drafts too, and a draft is not an issue: no
                // narrowed read answers with one, whatever its origin field holds.
                if let Some(resolved) = self.resolve(item)?
                    && resolved.content_kind == ContentKind::Issue
                {
                    keep(resolved, &mut found);
                }
            }
            let searched = data.get("search").ok_or_else(|| SourceError::Malformed {
                message: "GitHub search response has no search connection".into(),
            })?;
            for node in optional_nodes(Some(searched), "search")?
                .into_iter()
                .flatten()
            {
                if let Some(resolved) = self.resolve_issue(node).await? {
                    keep(resolved, &mut found);
                }
            }
            let items_next = resumed(items, items_after.as_deref())?;
            let search_next = resumed(searched, search_after.as_deref())?;
            if !items_next.has_more() && !search_next.has_more() {
                return Ok(found);
            }
            items_after = items_next.cursor();
            search_after = search_next.cursor();
        }
    }

    /// `found`, with every item this process created or wrote in its place, and every one of
    /// them the read did not report added.
    ///
    /// This process's own record wins over the read's copy of the same item, because a read
    /// of an item written moments ago can still be behind what was written onto it — the
    /// origin field included, which is the one a narrowed read is confirmed against — and a
    /// read that still names an item under a predicate this process's write moved it out of
    /// must not return it. The one thing the read knows that the record cannot is when GitHub
    /// last saw the item change, which is what a comment-activity read rules a candidate out
    /// by, so the read's `updatedAt` is kept when the record has none of its own. See
    /// [`Self::created`] and [`Self::updated`](GitHubProjectsSource::updated).
    fn with_own_writes(&self, mut found: Vec<Resolved>) -> Result<Vec<Resolved>, SourceError> {
        // A board draft is not an issue, so no narrowed read returns one, and this process
        // having written one does not make it an answer either.
        let own: Vec<Resolved> = self
            .created()?
            .iter()
            .chain(self.updated()?.iter())
            .filter(|own| own.content_kind == ContentKind::Issue)
            .cloned()
            .collect();
        for mut own in own {
            self.resolved_cache()?.insert(own.id.clone(), own.clone());
            match found.iter_mut().find(|read| read.id == own.id) {
                Some(read) => {
                    own.updated_at = own.updated_at.max(read.updated_at);
                    *read = own;
                }
                None => found.push(own),
            }
        }
        Ok(found)
    }

    /// Whether `item` has a comment created or last edited at or after `since` — always, when
    /// there is no instant to hold it to.
    ///
    /// The candidate's own `updatedAt` is read first, because a comment written or edited at
    /// or after the instant moved it there: an issue not updated since holds no such comment,
    /// and its comments are never asked for. Otherwise its comments are walked, oldest first,
    /// only as far as the first that matches. A board draft is not an issue and has no
    /// comments, so it never matches.
    async fn commented_since(
        &self,
        item: &Resolved,
        since: Option<DateTime<Utc>>,
    ) -> Result<bool, SourceError> {
        let Some(since) = since else {
            return Ok(true);
        };
        if item.content_kind == ContentKind::DraftIssue
            || item.updated_at.is_some_and(|updated| updated < since)
        {
            return Ok(false);
        }
        let query = TaskQuery {
            commented_since: Some(since),
            ..TaskQuery::default()
        };
        let mut after: Option<String> = None;
        loop {
            let data = self
                .graphql(
                    graphql::ISSUE_COMMENTS,
                    json!({"id":item.id.0,"first":MAX_PAGE_SIZE,"after":after}),
                )
                .await?;
            let Some(connection) = data
                .get("node")
                .filter(|value| !value.is_null())
                .and_then(|node| node.get("comments"))
                .filter(|value| !value.is_null())
            else {
                // Removed since the search reported it: no longer an issue with comments.
                return Ok(false);
            };
            let comments = optional_nodes(Some(connection), "issue comments")?
                .into_iter()
                .flatten()
                .map(comment_from)
                .collect::<Result<Vec<_>, _>>()?;
            if query.comments_match(&comments) {
                return Ok(true);
            }
            match next_cursor(connection)? {
                Some(next) => {
                    validate_cursor_progress(after.as_deref(), &next.0)?;
                    after = Some(next.0);
                }
                None => return Ok(false),
            }
        }
    }

    /// Every item on the board: the union of both enumerations GitHub offers of one.
    ///
    /// Neither contains the other, so neither is dropped — only `ProjectV2.items` lists a
    /// board **draft** and reads the board's own fields beside its items, and only the search
    /// reports an item that connection is behind on. The module documentation is where the lag and the
    /// measurements behind it are written down.
    ///
    /// A search result is admitted on the same terms as any other issue this source reaches
    /// directly — [`Self::resolve_issue`] keeps it only if that issue's own `projectItems`
    /// names *this* board — so an issue the index still believes is here after it was taken
    /// off is refused rather than reported.
    ///
    /// See [`Self::board_cache`]. Both completions happen on every call rather than once,
    /// which is what the cache could otherwise have broken.
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
        for held in self.searched_issues().await? {
            if !board.items.iter().any(|item| item.id == held.id) {
                board.items.push(held);
            }
        }
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
    /// "Where it sits" is three places, and missing an earlier one leaves a stale record
    /// that wins: an item this same run created is held in `created` and not in the cached
    /// board, and `board` completes the cached board *from* `created`, so replacing only
    /// the cached copy of such an item replaces nothing and the read still reports the
    /// title it was created with. The search is the third, and it is the one an item the
    /// board's own projection is behind on sits in *alone* — which is exactly the item this
    /// source is least able to re-read, so leaving it out would put the stale title back on
    /// the only items the completion in [`Self::board`] exists for.
    fn remember_written(&self, item: Resolved, created: bool) -> Result<(), SourceError> {
        self.resolved_cache()?.insert(item.id.clone(), item.clone());
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
        {
            let mut own = self.updated()?;
            match own.iter_mut().find(|held| held.id == item.id) {
                Some(held) => *held = item.clone(),
                None => own.push(item.clone()),
            }
        }
        if let Some(board) = self.board_cache()?.as_mut()
            && let Some(held) = board.items.iter_mut().find(|held| held.id == item.id)
        {
            *held = item.clone();
        }
        if let Some(found) = self.search_cache()?.as_mut()
            && let Some(held) = found.iter_mut().find(|held| held.id == item.id)
        {
            *held = item.clone();
        }
        for found in self.narrowed_cache()?.values_mut() {
            if let Some(held) = found.iter_mut().find(|held| held.id == item.id) {
                *held = item.clone();
            }
        }
        Ok(())
    }

    /// Forget one item this process has just deleted, from every half of its own view.
    fn forget(&self, id: &NativeId) -> Result<(), SourceError> {
        self.resolved_cache()?.remove(id);
        self.created()?.retain(|own| own.id != *id);
        self.updated()?.retain(|own| own.id != *id);
        if let Some(board) = self.board_cache()?.as_mut() {
            board.items.retain(|item| item.id != *id);
        }
        if let Some(found) = self.search_cache()?.as_mut() {
            found.retain(|item| item.id != *id);
        }
        for found in self.narrowed_cache()?.values_mut() {
            found.retain(|item| item.id != *id);
        }
        Ok(())
    }

    /// This process's own record of each narrowed answer, or the refusal a poisoned lock is.
    fn narrowed_cache(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Vec<Resolved>>>, SourceError> {
        self.narrowed_cache
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's view of a narrowed read was left inconsistent by an \
                      earlier failure; next: run the command again"
                    .into(),
            })
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

    /// The existing items this source has written, for completing a narrowed read that is
    /// behind; see [`Self::updated`](GitHubProjectsSource::updated).
    fn updated(&self) -> Result<std::sync::MutexGuard<'_, Vec<Resolved>>, SourceError> {
        self.updated.lock().map_err(|_| SourceError::Unavailable {
            message: "this source's record of what it wrote in this run was left inconsistent \
                      by an earlier failure; next: run the command again"
                .into(),
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
        let raw_body = optional_str(content, "body")?.map(str::to_owned);
        let (body, slot) = metadata_body(raw_body.clone())?;
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
            .map(|origin| Repository::try_from(format!("{}/{origin}", RepositoryTarget::HOST)))
            .transpose()
            .map_err(|message| SourceError::Malformed { message })?;
        let repositories = if slot.contains_key(Repository::METADATA_KEY) {
            Repository::from_metadata(&slot)
                .map_err(|message| SourceError::Malformed { message })?
        } else {
            own_repository.clone().into_iter().collect()
        };
        let id = NativeId(content_id.to_owned());
        // Read only for a task, because only a task has either list: a project or a
        // document holding one of these keys holds nothing this source reports, and the
        // keys are left out of its caller-visible metadata all the same.
        let (delivers, delivered_by) = if kind == BoardKind::Work(ItemKind::Task) {
            let listed = |key: &str| {
                TaskRef::from_value(key, &id, Some(&self.name), slot.get(key))
                    .map_err(|message| SourceError::Malformed { message })
            };
            (
                listed(TaskRef::DELIVERS_KEY)?,
                listed(TaskRef::DELIVERED_BY_KEY)?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let (option, closed, reason) = Self::status_parts(nodes, content)?;
        let priority = self.held_priority(nodes)?;
        // Present when the item was reached through its own issue, whose board entry
        // names the board; a read of the board's own items has the board already. An
        // empty id names nothing a field write could address, so it is read as absent and
        // the write goes back to reading the board.
        let board_id = item
            .pointer("/project/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let resolved = Resolved {
            item_id: required_str(item, "id")?.to_owned(),
            id,
            content_kind,
            kind,
            title,
            body: body.filter(|value| !value.is_empty()),
            raw_body,
            status: self
                .statuses
                .status(kind.status_kind(), option, closed, reason),
            option: option.map(str::to_owned),
            priority,
            closed,
            delivers,
            delivered_by,
            labels: labels(content)?,
            parent,
            origin: text_field(nodes, ORIGIN_FIELD)?.filter(|value| !value.is_empty()),
            number: match content_kind {
                ContentKind::Issue => Some(issue_number(content)?),
                // A draft is filed in no repository, so nothing ever numbered it:
                // `DraftIssue` declares no `number` at all, exactly as it declares no
                // `subIssuesSummary` the branch above reads.
                ContentKind::DraftIssue => None,
            },
            url: optional_str(content, "url")?.map(str::to_owned),
            created_at: optional_time(content, "createdAt")?,
            updated_at: optional_time(content, "updatedAt")?,
            own_repository,
            repositories,
            slot,
            board_id: board_id.map(str::to_owned),
            fields: field_definitions(nodes),
            board_fields: Self::carried_board_fields(content, board_id)?,
            blocked_by: carried_blocked_by(content)?,
        };
        self.resolved_cache()?
            .insert(resolved.id.clone(), resolved.clone());
        Ok(Some(resolved))
    }

    /// The field definitions of the board `board_id` names — the project this issue's own
    /// board item is on — off the `boards` page a read of an issue by its own id carries, or
    /// `None` when the read carried none, carried no entry for that board, or the board item
    /// named no board, which a write then answers by reading the board's fields itself.
    ///
    /// Matched by the board's node id and never by its number alone: a project number is
    /// unique only within its owner, so another owner's board numbered alike can sit on the
    /// same page, and its field and option ids address nothing on this one.
    fn carried_board_fields(
        content: &Value,
        board_id: Option<&str>,
    ) -> Result<Option<Value>, SourceError> {
        let (Some(nodes), Some(board_id)) = (
            content.pointer("/boards/nodes").and_then(Value::as_array),
            board_id,
        ) else {
            return Ok(None);
        };
        let Some(board) = nodes.iter().find_map(|node| {
            let project = node.get("project")?;
            (project.get("id").and_then(Value::as_str) == Some(board_id)).then_some(project)
        }) else {
            return Ok(None);
        };
        let Some(fields) = board.get("fields").filter(|fields| !fields.is_null()) else {
            return Ok(None);
        };
        complete_connection(fields, "board fields", NESTED_PAGE_SIZE)?;
        Ok(Some(fields.clone()))
    }

    /// What one board item's `Priority` field says, through this instance's mapping.
    ///
    /// An instance with no mapping holds no priority, so every item reads as `none` whatever
    /// its board holds. With one, no value is `none`, a mapped option is its level, and an
    /// option the mapping does not name is kept as itself — never read as a level or as
    /// `none` — for a read of the task to report by name.
    fn held_priority(&self, field_values: &[Value]) -> Result<HeldPriority, SourceError> {
        let Some(mapping) = &self.priorities else {
            return Ok(HeldPriority::Read(Priority::None));
        };
        // A value of the field that names no option — a text field someone called `Priority` —
        // is malformed rather than `none`: reading it as no priority would let the next copy
        // clear one a person set.
        let Some(option) = field_values
            .iter()
            .find(|value| {
                value.pointer("/field/name").and_then(Value::as_str) == Some(PRIORITY_FIELD)
            })
            .map(|value| required_str(value, "name"))
            .transpose()?
        else {
            return Ok(HeldPriority::Read(Priority::None));
        };
        Ok(mapping.priority_of(option).map_or_else(
            || HeldPriority::Unmapped(option.to_owned()),
            HeldPriority::Read,
        ))
    }

    /// What one board item's status is read from: its `Status` option, whether its issue
    /// is closed, and the reason it was closed with. [`BoardStatuses::status`] turns the
    /// three into the status it reports.
    fn status_parts<'a>(
        field_values: &'a [Value],
        content: &'a Value,
    ) -> Result<(Option<&'a str>, bool, Option<&'a str>), SourceError> {
        let option = field_values
            .iter()
            .find(|value| value.pointer("/field/name").and_then(Value::as_str) == Some("Status"))
            .map(|value| required_str(value, "name"))
            .transpose()?;
        let closed = optional_str(content, "state")? == Some("CLOSED");
        Ok((option, closed, optional_str(content, "stateReason")?))
    }

    /// The board Status option this write selects, or the refusal that says why not.
    ///
    /// The mapped option is required for both open and terminal targets. A terminal write
    /// validates it before changing either representation, so it can never fall back to
    /// closing an issue whose board cannot display the matching status.
    ///
    /// Answers the field's id, the option's id, and the option's name as the board spells
    /// it — which is the name a read of the item reports once it sits there.
    fn column_for(
        &self,
        fields: &Value,
        kind: ItemKind,
        category: StatusCategory,
        target: &StatusTarget,
    ) -> Result<Option<(String, String, String)>, SourceError> {
        let Some(wanted) = target.option() else {
            return Ok(None);
        };
        let missing = |detail: &str| SourceError::Refused {
            message: format!(
                "{} status {} of source {} needs the board Status option {wanted:?}, and \
                 {detail}; next: add that option to the board, which `onetaskgraph sources \
                 fields {} --apply` does, or point status_mapping.{}.{} of this source at one \
                 it has",
                kind.marker(),
                category_name(category),
                self.name,
                self.name,
                category_name(category),
                kind.marker()
            ),
        };
        let Some(field) = Board::field(fields, "Status")? else {
            return Err(missing("this board has no Status field"));
        };
        if required_str(field, "__typename")? != "ProjectV2SingleSelectField" {
            return Err(missing(
                "this board's Status field is not a single-select field",
            ));
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
            None => Err(missing("this board does not have it")),
            Some(option) => Ok(Some((
                required_str(field, "id")?.to_owned(),
                required_str(option, "id")?.to_owned(),
                required_str(option, "name")?.to_owned(),
            ))),
        }
    }

    /// The refusal a status that closes an issue is answered with over a board draft.
    fn closes_a_draft(&self, category: StatusCategory) -> SourceError {
        SourceError::Refused {
            message: format!(
                "status {} of source {} closes the item's issue, and GitHub draft items have \
                 no open or closed state",
                category_name(category),
                self.name
            ),
        }
    }

    /// What a status write to one item needs of the board: the board's id and the
    /// definition of its `Status` field, read off the item when the item says both.
    ///
    /// The same reasoning as [`Self::fields_for`]: a node read of the item names its board,
    /// and its `Status` value carries that field's definition, options and all. An item that
    /// does not say — no board id, or no `Status` value to read the field off — takes them
    /// from [`Self::board_fields`], which reads no item.
    async fn status_board(&self, item: &Resolved) -> Result<BoardFields, SourceError> {
        if let Some(board) = item.carried_board() {
            return Ok(board);
        }
        if item.defines("Status")
            && let Some(board_id) = item.named_board()
        {
            return Ok(BoardFields {
                id: board_id,
                fields: json!({"nodes": item.fields, "pageInfo": {"hasNextPage": false}}),
            });
        }
        self.board_fields().await
    }

    /// Set one task's status and nothing else; see [`TaskSource::set_task_status`].
    async fn set_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        // Refused before anything is read, in the words a write of the same status is.
        let target = self.resolved_target(ItemKind::Task, category)?;
        let Some(mut item) = self
            .bound_item(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        else {
            return Ok(None);
        };
        let board = self.status_board(&item).await?;
        let (field, option, name) = self
            .column_for(&board.fields, ItemKind::Task, category, &target)?
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "status {} of source {} names no board Status option",
                    category_name(category),
                    self.name
                ),
            })?;
        if item.status.category == category && item.option.as_deref() == Some(&name) {
            return Ok(Some(item.status));
        }
        match &target {
            StatusTarget::Terminal(_, reason) => {
                if item.content_kind == ContentKind::DraftIssue {
                    return Err(self.closes_a_draft(category));
                }
                self.set_item_field(
                    board.id.as_str(),
                    &item.item_id,
                    &field,
                    json!({"singleSelectOptionId": option}),
                )
                .await?;
                self.update_content(
                    ContentKind::Issue,
                    &item.id,
                    json!({"stateInput": state_input(Some(&target))}),
                )
                .await?;
                item.closed = true;
                item.status =
                    self.statuses
                        .status(ItemKind::Task, Some(&name), true, Some(reason.reason()));
                item.option = Some(name);
            }
            StatusTarget::Column(_) => {
                // An option is what an open item's status is, so a closed issue is reopened
                // first — sitting closed in the column, it would read back as closed. A draft has
                // no state to reopen.
                if item.content_kind == ContentKind::Issue && item.closed {
                    self.update_content(
                        ContentKind::Issue,
                        &item.id,
                        json!({"stateInput": state_input(Some(&target))}),
                    )
                    .await?;
                    item.closed = false;
                }
                self.set_item_field(
                    board.id.as_str(),
                    &item.item_id,
                    &field,
                    json!({"singleSelectOptionId": option}),
                )
                .await?;
                item.status = self
                    .statuses
                    .status(ItemKind::Task, Some(&name), false, None);
                item.option = Some(name);
            }
            StatusTarget::Disabled(_) => {
                unreachable!("resolved_target refused a disabled status")
            }
        }
        let status = item.status.clone();
        self.remember_written(item, false)?;
        Ok(Some(status))
    }

    /// Replace one task's `delivered_by` and nothing else; see
    /// [`TaskSource::set_delivered_by`].
    ///
    /// One update of the body, which differs from the body GitHub holds only inside the
    /// metadata slot — see [`with_slot`]. A body that would not change is not sent at all.
    async fn replace_delivered_by(
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
        let Some(mut item) = self
            .bound_item(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        else {
            return Ok(None);
        };
        let mut slot = item.slot.clone();
        set_task_list(&mut slot, TaskRef::DELIVERED_BY_KEY, &entries);
        self.write_slot(&mut item, &slot).await?;
        item.delivered_by = entries;
        self.remember_written(item, false)?;
        Ok(Some(()))
    }

    /// Set one caller key of the metadata slot of one issue of `kind`, and nothing else;
    /// see [`TaskSource::set_task_metadata`].
    ///
    /// `None` when this board holds no item by that id, or holds one of another kind. The
    /// answer is the item as this source now reads it, so what a caller is told the key
    /// holds is what the slot holds.
    ///
    /// A key already holding the value is answered without a write, compared as JSON rather
    /// than as the body's bytes: a slot a person spelled with other whitespace would
    /// otherwise be re-encoded, which is a write that changes nothing the caller asked for.
    async fn set_slot_key(
        &self,
        id: &NativeId,
        kind: BoardKind,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Resolved>, SourceError> {
        let Some(mut item) = self.bound_item(id).await?.filter(|item| item.kind == kind) else {
            return Ok(None);
        };
        if item.slot.get(key.as_str()) == Some(value) {
            return Ok(Some(item));
        }
        let mut slot = item.slot.clone();
        slot.insert(key.as_str().to_owned(), value.clone());
        self.write_slot(&mut item, &slot).await?;
        self.remember_written(item.clone(), false)?;
        Ok(Some(item))
    }

    /// Put `slot` in one item's metadata slot with a single update of its body, and bring
    /// `item` up to what that write left.
    ///
    /// The body sent differs from the body GitHub holds only inside the slot — see
    /// [`with_slot`] — and a body that would not change is not sent at all. It goes through
    /// the mutation the item's content takes, so a board draft's body is written with
    /// `updateProjectV2DraftIssue` exactly as an issue's is with `updateIssue`.
    async fn write_slot(
        &self,
        item: &mut Resolved,
        slot: &BTreeMap<String, Value>,
    ) -> Result<(), SourceError> {
        let held = item.raw_body.clone().unwrap_or_default();
        let body = with_slot(&held, slot)?;
        if body != held {
            self.update_content(item.content_kind, &item.id, json!({"body": body}))
                .await?;
        }
        let (visible, slot) = metadata_body(Some(body.clone()))?;
        item.body = visible.filter(|value| !value.is_empty());
        item.raw_body = Some(body);
        item.slot = slot;
        Ok(())
    }

    /// This instance's target for a category written to an item of `kind`, refusing one
    /// that kind has no option for — before anything is read or written.
    ///
    /// Nothing here mutates the board's option set to make room for a status. GitHub
    /// documents `UpdateProjectV2FieldInput.singleSelectOptions` as *"provided values
    /// overwrite existing options"*, so no addition is additive and a mistake destroys the
    /// field and every item's status.
    fn resolved_target(
        &self,
        kind: ItemKind,
        category: StatusCategory,
    ) -> Result<StatusTarget, SourceError> {
        let target = self.statuses.target(kind, category).clone();
        let StatusTarget::Disabled(why) = target else {
            return Ok(target);
        };
        let refusal = why.refusal(&self.name, category, kind);
        // Why there is no shipped default, which is the question a person meeting this
        // refusal on a source that never mentioned the category asks.
        let shipped_none = match category {
            StatusCategory::Draft => Some(
                "draft has no shipped default because GitHub draft issues cannot have \
                 sub-issues, and this source stores a project's tasks as its issue's sub-issues",
            ),
            StatusCategory::Unknown => Some(
                "unknown has no shipped default because this board keeps no open-ended status \
                 word: every word classified unknown is written to the one board Status option \
                 status_mapping.unknown names",
            ),
            _ => None,
        };
        Err(match (refusal, shipped_none, why) {
            (SourceError::Refused { message }, Some(note), UnmappedStatus::Unconfigured) => {
                SourceError::Refused {
                    message: format!("{message}; {note}"),
                }
            }
            (refusal, _, _) => refusal,
        })
    }

    /// What writing `priority` does to one item's `Priority` field on this board, or the
    /// refusal naming what the board lacks.
    ///
    /// `none` is no value, so it clears the field — and asks nothing of an item that holds
    /// none already, or of an item not created yet. Every other priority selects the option
    /// the mapping names, matched case-insensitively; a board with no `Priority` field, or
    /// without that option, is refused rather than given one: reads and writes never create
    /// a field or an option.
    fn priority_write(
        &self,
        fields: &Value,
        existing: Option<&Resolved>,
        priority: Priority,
    ) -> Result<Option<PriorityWrite>, SourceError> {
        let Some(mapping) = &self.priorities else {
            return Err(self.holds_no_priority());
        };
        let Some(wanted) = mapping.option(priority) else {
            if !existing.is_some_and(Resolved::holds_priority) {
                return Ok(None);
            }
            let field =
                Board::field(fields, PRIORITY_FIELD)?.ok_or_else(|| SourceError::Malformed {
                    message: format!(
                        "an item holding a {PRIORITY_FIELD} value was read without that field"
                    ),
                })?;
            return Ok(Some(PriorityWrite::Clear {
                field: required_str(field, "id")?.to_owned(),
            }));
        };
        let missing = |detail: &str| SourceError::Refused {
            message: format!(
                "priority {priority} of source {} needs the board {PRIORITY_FIELD} option \
                 {wanted:?}, and {detail}; run `onetaskgraph sources fields {} --apply` to add \
                 it, or point priority_mapping.{priority} of this source at an option the board \
                 has",
                self.name, self.name
            ),
        };
        let Some(field) = Board::field(fields, PRIORITY_FIELD)? else {
            return Err(missing(&format!(
                "this board has no {PRIORITY_FIELD} field"
            )));
        };
        if required_str(field, "__typename")? != "ProjectV2SingleSelectField" {
            return Err(missing(&format!(
                "this board's {PRIORITY_FIELD} field is not a single-select field"
            )));
        }
        // An options list that is absent or not a list is an answer this source cannot read,
        // not a board lacking the option: `sources fields --apply` is no remedy for it.
        let option = field
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub {PRIORITY_FIELD} field options is not an array"),
            })?
            .iter()
            .find(|option| {
                option
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
            })
            .ok_or_else(|| missing("this board does not have it"))?;
        Ok(Some(PriorityWrite::Select {
            field: required_str(field, "id")?.to_owned(),
            option: required_str(option, "id")?.to_owned(),
        }))
    }

    /// Apply one priority write to one board item.
    async fn write_priority(
        &self,
        board_id: &str,
        item_id: &str,
        write: &PriorityWrite,
    ) -> Result<(), SourceError> {
        match write {
            PriorityWrite::Select { field, option } => {
                self.set_item_field(
                    board_id,
                    item_id,
                    field,
                    json!({"singleSelectOptionId": option}),
                )
                .await
            }
            PriorityWrite::Clear { field } => {
                let data = self
                    .graphql(
                        graphql::CLEAR_FIELD,
                        json!({"input":{"projectId":board_id,"itemId":item_id,"fieldId":field},
                            "readPriority":false,"priorityName":PRIORITY_FIELD}),
                    )
                    .await?;
                let returned = data
                    .pointer("/clearProjectV2ItemFieldValue/projectV2Item")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub field clear returned no project item".into(),
                    })?;
                if required_str(returned, "id")? != item_id {
                    return Err(SourceError::Malformed {
                        message: "GitHub field clear returned the wrong project item".into(),
                    });
                }
                Ok(())
            }
        }
    }

    /// The refusal a priority is answered with by an instance configured with no
    /// `priority_mapping`, which holds none.
    fn holds_no_priority(&self) -> SourceError {
        SourceError::Refused {
            message: format!(
                "source {} holds no task priority: its configuration sets no priority_mapping; \
                 next: set priority_mapping on this source, then run `onetaskgraph sources \
                 fields {} --apply` to set its board up",
                self.name, self.name
            ),
        }
    }

    /// Set one task's priority and nothing else; see [`TaskSource::set_task_priority`].
    ///
    /// One field write — a select, or a clear for `none` — and no title, body, label, state
    /// or `Status` request. Clearing a priority an item does not hold sends nothing.
    async fn set_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        if self.priorities.is_none() {
            return Err(self.holds_no_priority());
        }
        let Some(mut item) = self
            .bound_item(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        else {
            return Ok(None);
        };
        if priority == Priority::None && !item.holds_priority() {
            return Ok(Some(priority));
        }
        // The item's own read carries the field's definition whenever it holds a value of
        // it, which a clear always does; a select onto an item holding none reads the board.
        let board = match (item.carried_board(), item.named_board()) {
            (Some(board), _) => board,
            (None, Some(id)) if item.defines(PRIORITY_FIELD) => BoardFields {
                id,
                fields: json!({"nodes": item.fields.clone(), "pageInfo": {"hasNextPage": false}}),
            },
            _ => self.board_fields().await?,
        };
        let Some(write) = self.priority_write(&board.fields, Some(&item), priority)? else {
            return Ok(Some(priority));
        };
        let (document, root, input) = match write {
            PriorityWrite::Select { field, option } => (
                graphql::UPDATE_FIELD,
                "updateProjectV2ItemFieldValue",
                json!({"projectId":board.id.as_str(),"itemId":item.item_id,"fieldId":field,"value":{"singleSelectOptionId":option}}),
            ),
            PriorityWrite::Clear { field } => (
                graphql::CLEAR_FIELD,
                "clearProjectV2ItemFieldValue",
                json!({"projectId":board.id.as_str(),"itemId":item.item_id,"fieldId":field}),
            ),
        };
        let data = self
            .graphql(
                document,
                json!({"input":input,"readPriority":true,"priorityName":PRIORITY_FIELD}),
            )
            .await?;
        let returned = data
            .get(root)
            .and_then(|value| value.get("projectV2Item"))
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub priority write returned no project item".into(),
            })?;
        if required_str(returned, "id")? != item.item_id {
            return Err(SourceError::Malformed {
                message: "GitHub priority write returned the wrong project item".into(),
            });
        }
        let value = returned
            .get("fieldValueByName")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub priority write returned no priority read-back".into(),
            })?;
        if !value.is_null()
            && value.pointer("/field/name").and_then(Value::as_str) != Some(PRIORITY_FIELD)
        {
            return Err(SourceError::Malformed {
                message: "GitHub priority read-back is not a Priority field value".into(),
            });
        }
        let values = if value.is_null() {
            Vec::new()
        } else {
            vec![value.clone()]
        };
        item.priority = self.held_priority(&values)?;
        let answer = item.task()?.priority;
        self.remember_written(item, false)?;
        Ok(Some(answer))
    }

    /// Replace one task's visible body and nothing else; see
    /// [`TaskSource::set_task_content`].
    ///
    /// One update of the body, which differs from the body GitHub holds only outside the
    /// metadata slot — the slot is kept byte for byte, so every caller key and every list
    /// this source keeps there reads back as it was. A body that would not change is not
    /// sent at all.
    async fn replace_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        let Some(mut item) = self
            .bound_item(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        else {
            return Ok(None);
        };
        let held = item.raw_body.clone().unwrap_or_default();
        let body = with_content(&held, content)?;
        // Checked before anything is sent: content ending in what this source reads as its own
        // metadata slot would read back as metadata rather than as the content it was.
        let (visible, slot) = metadata_body(Some(body.clone()))?;
        if visible.as_deref().unwrap_or_default() != content || slot != item.slot {
            return Err(SourceError::Refused {
                message: format!(
                    "this content ends in what source {} reads as its own metadata slot \
                     ({METADATA_OPEN:?}), so part of it would read back as metadata rather than \
                     as content; next: remove that trailing block from the content",
                    self.name
                ),
            });
        }
        if body != held {
            self.update_content(item.content_kind, &item.id, json!({"body": body}))
                .await?;
        }
        item.body = visible.filter(|value| !value.is_empty());
        item.raw_body = Some(body);
        item.slot = slot;
        self.remember_written(item, false)?;
        Ok(Some(()))
    }

    /// Apply one targeted update to one task; see [`TaskSource::update_task`].
    ///
    /// One read of the item — which carries the board's field definitions and the issue's
    /// `blockedBy`, so neither is read again — and then only what differs from it: the
    /// `Status` option and the `Priority` field together in one request, the `blockedBy`
    /// additions and removals the named edges differ by, and last one `updateIssue` carrying
    /// the title, the body — visible content and metadata slot together — and a state change.
    /// So an update naming any of title, body, metadata, status and priority is one read and
    /// at most two writes. The body goes last so that a write refused part-way leaves it, and
    /// the metadata in it, as it stood. A terminal status selects its option and then closes,
    /// as a whole write does; an open one selects its option and then reopens. The origin
    /// field is never written: an update is of an item that already exists, whose origin is
    /// what it is.
    ///
    /// The task answered is the item as those writes left it, built from the read and what was
    /// sent rather than read again — the same record a later read in this run answers from.
    async fn targeted_update(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        // Everything this source can refuse without reading the item is refused first, in the
        // words a whole write of the same fields is refused with.
        update.consistent()?;
        if update
            .title
            .as_deref()
            .is_some_and(|title| title.starts_with(DESIGN_TITLE_PREFIX))
        {
            return Err(SourceError::Refused {
                message: format!(
                    "the title of this task begins {DESIGN_TITLE_PREFIX:?}, which is how source {} \
                     spells a document, so it would read back as one rather than as a task; \
                     retitle it",
                    self.name
                ),
            });
        }
        if let Some(delivers) = &update.delivers {
            TaskRef::listed(
                TaskRef::DELIVERS_KEY,
                id,
                Some(&self.name),
                delivers.clone(),
            )
            .map_err(|message| SourceError::Refused { message })?;
        }
        if self.priorities.is_none()
            && update
                .priority
                .is_some_and(|priority| priority != Priority::None)
        {
            return Err(self.holds_no_priority());
        }
        let target = update
            .status
            .as_ref()
            .map(|status| self.resolved_target(ItemKind::Task, status.category))
            .transpose()?;
        let Some(mut item) = self
            .bound_item(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        else {
            return Ok(None);
        };
        let before = item.task()?;

        let mut status_move = None;
        if let (Some(status), Some(target)) = (&update.status, target) {
            let board = self.status_board(&item).await?;
            let (field, option, name) = self
                .column_for(&board.fields, ItemKind::Task, status.category, &target)?
                .ok_or_else(|| SourceError::Malformed {
                    message: format!(
                        "status {} of source {} names no board Status option",
                        category_name(status.category),
                        self.name
                    ),
                })?;
            let terminal = matches!(target, StatusTarget::Terminal(_, _));
            if terminal && item.content_kind == ContentKind::DraftIssue {
                return Err(self.closes_a_draft(status.category));
            }
            let landed = match &target {
                StatusTarget::Terminal(_, reason) => {
                    self.statuses
                        .status(ItemKind::Task, Some(&name), true, Some(reason.reason()))
                }
                _ => self
                    .statuses
                    .status(ItemKind::Task, Some(&name), false, None),
            };
            let option_moves = item
                .option
                .as_deref()
                .is_none_or(|held| !held.eq_ignore_ascii_case(&name));
            let state_moves = item.content_kind == ContentKind::Issue
                && (item.closed != terminal || (terminal && item.status != landed));
            if let Some(moves) = Moves::of(option_moves, state_moves) {
                status_move = Some(StatusMove {
                    board: board.id,
                    field,
                    option,
                    name,
                    target,
                    landed,
                    moves,
                });
            }
        }

        let mut priority_move = None;
        if let Some(priority) = update.priority
            && self.priorities.is_some()
            && item.priority != HeldPriority::Read(priority)
        {
            let board = match (item.carried_board(), item.named_board()) {
                (Some(board), _) => board,
                (None, Some(board)) if item.defines(PRIORITY_FIELD) => BoardFields {
                    id: board,
                    fields: json!({"nodes": item.fields, "pageInfo": {"hasNextPage": false}}),
                },
                _ => self.board_fields().await?,
            };
            if let Some(write) = self.priority_write(&board.fields, Some(&item), priority)? {
                priority_move = Some((board.id, write, priority));
            }
        }

        // Resolved before the body is composed, because a far end `blockedBy` cannot name is
        // recorded in the slot, and the slot travels in the one body update below.
        let edges = match &update.depends_on {
            Some(edges) => Some(
                self.partition_edges(
                    BoardKind::Work(ItemKind::Task),
                    item.content_kind,
                    item.blocked_by.as_deref(),
                    edges,
                )
                .await?,
            ),
            None => None,
        };

        let mut slot = item.slot.clone();
        for (key, value) in &update.metadata_set {
            slot.insert(key.as_str().to_owned(), value.clone());
        }
        for key in &update.metadata_remove {
            slot.remove(key.as_str());
        }
        if let Some(delivers) = &update.delivers {
            set_task_list(&mut slot, TaskRef::DELIVERS_KEY, delivers);
        }
        if let Some((_, recorded)) = &edges {
            record_edges(&mut slot, recorded);
        }
        let held = item.raw_body.clone().unwrap_or_default();
        let content = match &update.content {
            Some(content) => with_content(&held, content)?,
            None => held.clone(),
        };
        // A slot holding what it held is kept byte for byte, compared as JSON rather than as
        // the body's bytes, as a metadata write compares it: a slot a person spelled with
        // other whitespace would otherwise be re-encoded, which is a write nobody asked for.
        let body = if slot == item.slot {
            content
        } else {
            with_slot(&content, &slot)?
        };
        // Checked before anything is sent, as a content write checks it: content ending in
        // what this source reads as its own slot would read back as metadata.
        let (visible, read) = metadata_body(Some(body.clone()))?;
        let wanted = update.content.as_deref().or(item.body.as_deref());
        if visible.as_deref().unwrap_or_default() != wanted.unwrap_or_default() || read != slot {
            return Err(SourceError::Refused {
                message: format!(
                    "this content ends in what source {} reads as its own metadata slot \
                     ({METADATA_OPEN:?}), so part of it would read back as metadata rather than \
                     as content; next: remove that trailing block from the content",
                    self.name
                ),
            });
        }
        let recorded_moves =
            slot.get(DependencyEdge::RECORDED_KEY) != item.slot.get(DependencyEdge::RECORDED_KEY);

        // One `updateIssue` carries all three, because every mutation spends the secondary
        // limiter and the title, body and state are one mutation's inputs.
        let mut fields = serde_json::Map::new();
        if let Some(title) = update.title.as_ref().filter(|title| **title != item.title) {
            fields.insert("title".to_owned(), json!(title));
        }
        if body != held {
            fields.insert("body".to_owned(), json!(body));
        }
        if let Some(moving) = status_move.as_ref().filter(|moving| moving.moves.state()) {
            fields.insert("stateInput".to_owned(), state_input(Some(&moving.target)));
        }
        // **The body is written last, and that is the guarantee a refusal part-way keeps.**
        // GitHub runs no two requests as one, and runs one document's mutation fields in order
        // without undoing an earlier field when a later one fails — so a body written before a
        // board field the board then refused would be left changed. Written after every other
        // write has landed, a refusal anywhere leaves the item's body, and every metadata key
        // it carries, exactly as they stood. So the `Status` option and the `Priority` field go
        // first, together in one request — a terminal option selected before the issue
        // closes, as a whole write does — then the `blockedBy` difference, then the body.
        let mut board_writes: Vec<(&BoardId, (String, Value))> = Vec::new();
        let mut clear: Option<(&BoardId, &str)> = None;
        if let Some(moving) = status_move.as_ref().filter(|moving| moving.moves.option()) {
            board_writes.push((
                &moving.board,
                (
                    moving.field.clone(),
                    json!({"singleSelectOptionId": moving.option}),
                ),
            ));
        }
        match &priority_move {
            Some((board, PriorityWrite::Select { field, option }, _)) => board_writes.push((
                board,
                (field.clone(), json!({"singleSelectOptionId": option})),
            )),
            Some((board, PriorityWrite::Clear { field }, _)) => clear = Some((board, field)),
            None => {}
        }
        let mut boards: Vec<&BoardId> = board_writes.iter().map(|(board, _)| *board).collect();
        boards.extend(clear.map(|(board, _)| board));
        boards.dedup_by(|one, other| one.as_str() == other.as_str());
        for board in boards {
            let writes = board_writes
                .iter()
                .filter(|(on, _)| on.as_str() == board.as_str())
                .map(|(_, write)| write.clone())
                .collect::<Vec<_>>();
            let cleared = clear
                .filter(|(on, _)| on.as_str() == board.as_str())
                .map(|(_, field)| field);
            self.set_item_fields(board.as_str(), &item.item_id, &writes, cleared)
                .await?;
        }
        let mut blocked_by_moved = false;
        if let Some((native, _)) = &edges
            && item.content_kind == ContentKind::Issue
        {
            blocked_by_moved = self
                .reconcile_blocked_by(
                    &item.id,
                    native,
                    Issue::Existing(item.blocked_by.as_deref()),
                )
                .await?;
        }
        if !fields.is_empty() {
            self.update_content(item.content_kind, &item.id, Value::Object(fields))
                .await?;
        }

        if let Some(title) = &update.title {
            item.title.clone_from(title);
        }
        item.body = visible.filter(|value| !value.is_empty());
        item.raw_body = (!body.is_empty() || item.raw_body.is_some()).then_some(body);
        item.slot = slot;
        if let Some(delivers) = &update.delivers {
            item.delivers.clone_from(delivers);
        }
        if let Some(moving) = status_move {
            item.closed = matches!(moving.target, StatusTarget::Terminal(_, _))
                && item.content_kind == ContentKind::Issue;
            item.status = moving.landed;
            item.option = Some(moving.name);
        }
        if let Some((_, _, priority)) = priority_move {
            item.priority = HeldPriority::Read(priority);
        }
        let task = item.task()?;
        let mut written = update.changed(&before, &task);
        if blocked_by_moved || recorded_moves {
            written.insert(UpdatedField::DependsOn);
        }
        self.remember_written(item, false)?;
        Ok(Some(TaskUpdateOutcome {
            task,
            written,
            delivers_before: before.delivers,
        }))
    }

    /// Replace one issue's visible body and its [`MetadataKey::TEMPLATE_KEY`] slot entry
    /// together, and nothing else; see [`TaskSource::set_task_rendering`].
    ///
    /// One update of the body: the content outside the slot, and inside it that one entry,
    /// every other entry kept as it was. This source keeps no template answers — an issue has
    /// no room beside itself that is not its body, and answers written there would duplicate
    /// what the content already says and count against GitHub's body limit — so `answers`
    /// reaches nothing here. A body that would not change is not sent at all.
    async fn replace_rendering(
        &self,
        id: &NativeId,
        kind: BoardKind,
        content: &str,
        provenance: &Value,
    ) -> Result<Option<()>, SourceError> {
        let Some(mut item) = self.bound_item(id).await?.filter(|item| item.kind == kind) else {
            return Ok(None);
        };
        let held = item.raw_body.clone().unwrap_or_default();
        let mut slot = item.slot.clone();
        slot.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance.clone());
        let body = with_slot(&with_content(&held, content)?, &slot)?;
        // Checked before anything is sent, as a content write checks it.
        let (visible, read) = metadata_body(Some(body.clone()))?;
        if visible.as_deref().unwrap_or_default() != content || read != slot {
            return Err(SourceError::Refused {
                message: format!(
                    "this content ends in what source {} reads as its own metadata slot \
                     ({METADATA_OPEN:?}), so part of it would read back as metadata rather than \
                     as content; next: remove that trailing block from the template",
                    self.name
                ),
            });
        }
        if body != held {
            self.update_content(item.content_kind, &item.id, json!({"body": body}))
                .await?;
        }
        item.body = visible.filter(|value| !value.is_empty());
        item.raw_body = Some(body);
        item.slot = read;
        self.remember_written(item, false)?;
        Ok(Some(()))
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
                },"readPriority":false,"priorityName":PRIORITY_FIELD}),
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

    /// GitHub accepts one value per field mutation; aliases combine those mutations in
    /// one request. Every returned item id is checked, including optional aliases.
    async fn set_item_fields(
        &self,
        board: &str,
        item: &str,
        fields: &[(String, Value)],
        clear: Option<&str>,
    ) -> Result<(), SourceError> {
        if fields.len() <= 1 && clear.is_none() {
            if let Some((field, value)) = fields.first() {
                self.set_item_field(board, item, field, value.clone())
                    .await?;
            }
            return Ok(());
        }
        if fields.is_empty() {
            if let Some(field) = clear {
                self.write_priority(
                    board,
                    item,
                    &PriorityWrite::Clear {
                        field: field.to_owned(),
                    },
                )
                .await?;
            }
            return Ok(());
        }
        let input = |index: usize| {
            let (field, value) = fields.get(index).unwrap_or(&fields[0]);
            json!({"projectId":board,"itemId":item,"fieldId":field,"value":value})
        };
        let data = self.graphql(graphql::UPDATE_FIELDS, json!({
            "input":input(0),"second":input(1),"third":input(2),
            "writeSecond":fields.len()>1,"writeThird":fields.len()>2,"writeClear":clear.is_some(),
            "clear":{"projectId":board,"itemId":item,"fieldId":clear.unwrap_or(&fields[0].0)}
        })).await?;
        for alias in [
            Some("updateProjectV2ItemFieldValue"),
            (fields.len() > 1).then_some("second"),
            (fields.len() > 2).then_some("third"),
            clear.map(|_| "cleared"),
        ]
        .into_iter()
        .flatten()
        {
            let returned = data
                .get(alias)
                .and_then(|value| value.get("projectV2Item"))
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("GitHub field update {alias} returned no project item"),
                })?;
            if required_str(returned, "id")? != item {
                return Err(SourceError::Malformed {
                    message: format!("GitHub field update {alias} returned the wrong project item"),
                });
            }
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
        // What this issue is blocked by, when a read of it by its own id in this command
        // already carried the whole connection — a copy reads the item it writes before it
        // reads its edges — and the page asked for is the whole of it, or the recorded tail
        // after it. Answered from that read, in the shape the dependency read answers in;
        // anything else is asked of GitHub.
        let carried = match direction {
            Direction::DependsOn => self
                .resolved_cache()?
                .get(id)
                .filter(|item| item.content_kind == ContentKind::Issue)
                .and_then(|item| Some((item.blocked_by.clone()?, item.raw_body.clone()))),
            Direction::DependedOnBy => None,
        }
        .filter(|(nodes, _)| recorded.is_some() || (cursor.is_none() && nodes.len() <= limit));
        // Asked for even in the recorded phase, whose page reads nothing from the
        // connection: `__typename` is what says whether this item has a native
        // relationship at all, and that is what decides which far ends the reserved key is
        // allowed to hold.
        let data = match carried {
            Some((nodes, body)) => json!({"node":{"__typename":"Issue","body":body,
                "blockedBy":{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
            None => {
                self.graphql(
                    graphql::ISSUE_DEPENDENCIES,
                    json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),
                           "after":if recorded.is_some() {None} else {cursor}}),
                )
                .await?
            }
        };
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
                self.recorded_edges(id, near_kind, direction, natively_names, node)
                    .await?,
                offset,
                limit,
            ));
        }
        if natively_names.is_none() {
            return Ok(recorded_page(
                self.recorded_edges(id, near_kind, direction, natively_names, node)
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
                .recorded_edges(id, near_kind, direction, natively_names, node)
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
    /// The metadata lives in the item's own body slot, and `node` is the dependency read's
    /// own answer, which carries an issue's body — so an issue's recorded edges cost no
    /// request beyond the read already made, and reading the board for them would be a
    /// walk of every item for one field of one. A draft has no body in that answer, because
    /// a draft is not an issue, so a draft's are read off its own read by id — never off a
    /// listing of the board, which can be behind on the very item asked about.
    async fn recorded_edges(
        &self,
        id: &NativeId,
        near_kind: ItemKind,
        direction: Direction,
        natively_names: Option<ItemKind>,
        node: &Value,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        if direction != Direction::DependsOn {
            return Ok(Vec::new());
        }
        let slot = match node.get("body") {
            Some(body) if natively_names.is_some() => {
                metadata_body(body.as_str().map(str::to_owned))?.1
            }
            _ => {
                let Some(item) = self.bound_item(id).await? else {
                    return Ok(Vec::new());
                };
                item.slot
            }
        };
        DependencyEdge::recorded(&slot, id, near_kind, &self.name, natively_names)
            .map_err(|message| SourceError::Malformed { message })
    }

    fn configured_repository(&self) -> Result<&RepositoryTarget, SourceError> {
        self.repository
            .as_ref()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "source {} has no repository configured, and a GitHub Projects board has no \
                 repository of its own to create an issue in; set repository: owner/name on \
                 this source",
                    self.name
                ),
            })
    }

    /// The repository one new issue is created in, under the rule [`RepositoryTarget`]
    /// states.
    ///
    /// The fallback is demanded first, whichever arm answers: a write without a configured
    /// repository is refused naming the field exactly as it was before the rule existed,
    /// so a source that could not write before cannot write now, rather than writing for
    /// the one item whose own field happens to decide it.
    ///
    /// Everything this refuses is refused before `createIssue`, so a refusal leaves no
    /// issue behind: an entry that is not a repository on [`RepositoryTarget::HOST`], an
    /// entry owned by someone other than the owner of the parent issue's repository —
    /// GitHub accepts a sub-issue from another repository of the same owner and from no
    /// other, so `addSubIssue` would refuse it after the issue existed — a parent the
    /// board does not hold, and a parent that is a draft, which GitHub gives no sub-issues,
    /// both of which `addSubIssue` would likewise refuse too late. Whether the entry exists
    /// and is visible to the token is checked where its node id is resolved, still before
    /// `createIssue`. The parent is read by its own id through [`Self::item_by_id`] — never
    /// looked up in a listing of the board, which can be minutes behind an issue its own
    /// `projectItems` already places on it — and that read answers first from this process's
    /// own record, so a project created moments ago in this command answers though GitHub
    /// has not caught up.
    async fn creation_target(
        &self,
        incoming: &Incoming<'_>,
    ) -> Result<RepositoryTarget, SourceError> {
        let fallback = self.configured_repository()?;
        let what = |incoming: &Incoming<'_>| {
            format!(
                "{} {:?}",
                incoming.written.kind().describes(),
                incoming.title
            )
        };
        let parent = match incoming.parent {
            Some(parent) => Some(self.bound_item(parent).await?.ok_or_else(|| {
                SourceError::Refused {
                    message: format!(
                        "GitHub project issue {} was not found on the board of source {}, so {} \
                         cannot be filed under it",
                        parent.0,
                        self.name,
                        what(incoming)
                    ),
                }
            })?),
            None => None,
        };
        let parents_repository = parent
            .as_ref()
            .map(|parent| {
                // A draft is on the board and so is found, but it has no repository to
                // place a task in and GitHub gives it no sub-issues, so `addSubIssue`
                // would refuse the task only once `createIssue` had made it.
                if parent.content_kind == ContentKind::DraftIssue {
                    return Err(SourceError::Refused {
                        message: format!(
                            "GitHub project item {} on the board of source {} is a draft, \
                             which cannot have sub-issues, so {} cannot be filed under it",
                            parent.id.0,
                            self.name,
                            what(incoming)
                        ),
                    });
                }
                // An issue's repository is where a sub-issue is placed and whose owner it
                // is compared against, so a parent whose repository this source cannot
                // spell as `owner/name` — GitHub's login grammar is wider than this
                // source's floor — is one nothing can be filed under.
                parent
                    .own_repository
                    .as_ref()
                    .and_then(|origin| RepositoryTarget::from_origin(origin).ok())
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!(
                            "GitHub project issue {} on the board of source {} is in {}, which \
                             is not a {}/owner/name repository this source can place {} in",
                            parent.id.0,
                            self.name,
                            parent
                                .own_repository
                                .as_ref()
                                .map_or("no repository", Repository::as_str),
                            RepositoryTarget::HOST,
                            what(incoming)
                        ),
                    })
            })
            .transpose()?;
        match incoming.repositories {
            [named] => {
                let target =
                    RepositoryTarget::from_origin(named).map_err(|_| SourceError::Refused {
                        message: format!(
                            "{} names repository {}, which is not a {}/owner/name repository \
                             source {} can create an issue in; name one that is, or name none",
                            what(incoming),
                            named.as_str(),
                            RepositoryTarget::HOST,
                            self.name
                        ),
                    })?;
                if let Some(parents) = &parents_repository
                    && parents.owner != target.owner
                {
                    return Err(SourceError::Refused {
                        message: format!(
                            "{} names repository {}, owned by {}, but its project's issue is in \
                             {}, owned by {}, and GitHub files a sub-issue only in a repository \
                             of the same owner as its parent issue; name a repository of {}, or \
                             name none",
                            what(incoming),
                            target.slug(),
                            target.owner,
                            parents.slug(),
                            parents.owner,
                            parents.owner
                        ),
                    });
                }
                Ok(target)
            }
            _ => Ok(parents_repository.unwrap_or_else(|| fallback.clone())),
        }
    }

    /// The node id of the repository `incoming` is being created in, or the refusal naming
    /// the item and the repository the token cannot see.
    ///
    /// Resolved once per command per repository; see [`Self::repository_cache`].
    async fn repository_id(
        &self,
        repository: &RepositoryTarget,
        incoming: &Incoming<'_>,
    ) -> Result<String, SourceError> {
        if let Some(id) = self.repository_cache()?.get(repository).cloned() {
            return Ok(id);
        }
        let data = self
            .graphql(
                graphql::REPOSITORY,
                json!({"owner":repository.owner,"name":repository.name}),
            )
            .await?;
        self.repository_read(&data, repository, incoming)
    }

    /// The repository's node id out of an answer carrying the `repository` root, held for
    /// the rest of this command, or the refusal naming the item that cannot be created in it.
    fn repository_read(
        &self,
        data: &Value,
        repository: &RepositoryTarget,
        incoming: &Incoming<'_>,
    ) -> Result<String, SourceError> {
        let node = data
            .get("repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub repository {} was not found or is not visible to the token, so {} \
                     {:?} cannot be created in it",
                    repository.slug(),
                    incoming.written.kind().describes(),
                    incoming.title
                ),
            })?;
        let id = required_str(node, "id")?.to_owned();
        self.repository_cache()?
            .insert(repository.clone(), id.clone());
        Ok(id)
    }

    fn repository_cache(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<RepositoryTarget, String>>, SourceError> {
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
        // The destination is read by its own id, and whether this board holds it is decided
        // by that read — its own `projectItems` — rather than by whether a listing of the
        // board happens to include it yet. See the module documentation.
        let existing = match target {
            Some(target) => {
                Some(
                    self.bound_item(target)
                        .await?
                        .ok_or_else(|| SourceError::Refused {
                            message: format!("GitHub destination item {} was not found", target.0),
                        })?,
                )
            }
            None => None,
        };
        let existing = existing.as_ref();
        // An existing issue is never moved; a new one is created where the rule says — and
        // knowing where is what lets the board's fields and that repository's id be read
        // together, before anything below needs either.
        let creation_target = match existing {
            Some(_) => None,
            None => {
                let target = self.creation_target(incoming).await?;
                self.creation_context(&target, incoming).await?;
                Some(target)
            }
        };
        let board = self
            .fields_for(
                existing,
                incoming.written.status().is_some(),
                incoming
                    .priority
                    .is_some_and(|priority| priority != Priority::None),
            )
            .await?;
        let status_target = incoming
            .written
            .work_status()
            .map(|(kind, status)| self.resolved_target(kind, status.category))
            .transpose()?;
        let column = match (incoming.written.work_status(), status_target.as_ref()) {
            (Some((kind, status)), Some(target)) => {
                self.column_for(&board.fields, kind, status.category, target)?
            }
            _ => None,
        };
        // Resolved before anything is created, for the reason the column above is: a
        // priority this board has no option for is refused while nothing has been written.
        let priority_write = match incoming.priority {
            Some(priority) => self.priority_write(&board.fields, existing, priority)?,
            None => None,
        };
        let content_kind = existing.map_or(ContentKind::Issue, |item| item.content_kind);
        if content_kind == ContentKind::DraftIssue {
            if let (Some(StatusTarget::Terminal(_, _)), Some(status)) =
                (status_target.as_ref(), incoming.written.status())
            {
                return Err(self.closes_a_draft(status.category));
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

        // The repository the issue really lives in is what the slot below is written against,
        // so a single entry that is where the issue is created travels as no key at all, and
        // the read side derives it back from the issue.
        let own_repository = match (existing, &creation_target) {
            (Some(item), _) => item.own_repository.clone(),
            (None, Some(target)) => Some(
                Repository::try_from(target.origin())
                    .map_err(|message| SourceError::Config { message })?,
            ),
            (None, None) => None,
        };
        let (native, fallback) = self
            .partition_edges(
                incoming.written.kind(),
                content_kind,
                existing.and_then(|item| item.blocked_by.as_deref()),
                depends_on,
            )
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

        let Landed {
            content_id,
            item_id,
            url,
            number,
        } = match existing {
            // Its content is written last, below, once everything else has landed.
            Some(item) => Landed {
                content_id: item.id.clone(),
                item_id: item.item_id.clone(),
                url: item.url.clone(),
                number: item.number,
            },
            None => {
                let target = creation_target
                    .as_ref()
                    .ok_or_else(|| SourceError::Malformed {
                        message: "a new item was decided without a repository to create it in"
                            .into(),
                    })?;
                self.create_and_file_issue(board.id.as_str(), target, incoming, &body)
                    .await?
            }
        };

        let written_option = column.as_ref().map(|(_, _, name)| name.clone());
        let column = column
            .filter(|(_, _, name)| existing.is_none_or(|item| item.option.as_ref() != Some(name)))
            .map(|(field, option, _)| (field, option));
        // Creating an item here is several calls — `createIssue`, which files it on the
        // board, then its board fields, the parent and the dependencies — and GitHub can fail
        // at any of them. Everything this source can refuse *before* the first of those is
        // already checked above, so what is left is GitHub itself failing part way. When it
        // does over an item this call created, the issue is taken back: a write that
        // refused must not leave an item behind that nobody asked for, and one that does
        // makes the retry create a second.
        // Whether the board-field write carrying a moved origin was answered as landing whole.
        // When it was refused, GitHub does not say which of its fields ran before the one that
        // failed, so the origin may or may not have moved.
        let mut origin_landed = false;
        let landed = self
            .finish_write(
                board.id.as_str(),
                incoming,
                &content_id,
                &item_id,
                content_kind,
                existing,
                origin_field.as_deref(),
                origin,
                column,
                status_target.as_ref(),
                priority_write.as_ref(),
                &native,
                &mut origin_landed,
            )
            .await;
        // An existing item's title, body and state go last, in one `updateIssue`, once its board
        // fields and its relationships have landed: a refusal of any of those then leaves its
        // body — and the metadata slot inside it — exactly as it stood.
        let landed = match (landed, existing) {
            (Ok(()), Some(item)) => {
                self.update_existing(item, incoming, &body, status_target.as_ref())
                    .await
            }
            (landed, _) => landed,
        };
        if let Err(error) = landed {
            match existing {
                // Best effort, and the write's own failure is what the caller is told: a
                // refusal naming the tidy-up would hide why the write failed at all.
                None => {
                    let _ = self.delete_issue(&content_id).await;
                }
                // The origin field is the one piece of an existing item's metadata written
                // before its body, so a write refused after it puts it back as it was. When
                // that is refused too, the write's own failure is still what the caller is
                // told — with what it left behind added, because the item's metadata is then
                // not as it stood and a caller retrying has to know which key moved.
                Some(item) => {
                    let before = item.origin.as_deref().unwrap_or("");
                    if let Some(field) = origin_field.as_deref()
                        && before != origin
                        && let Err(restore) = self
                            .set_item_field(
                                board.id.as_str(),
                                &item.item_id,
                                field,
                                json!({"text": before}),
                            )
                            .await
                    {
                        let left = if origin_landed {
                            format!(
                                "its {ORIGIN_KEY} was moved to {origin:?} before that and could \
                                 not be put back to {before:?} ({restore}), so item {} still \
                                 holds {origin:?} there",
                                item.id.0
                            )
                        } else {
                            format!(
                                "the refused write carried its {ORIGIN_KEY} from {before:?} to \
                                 {origin:?}, GitHub does not say whether that part of it ran, \
                                 and putting it back to {before:?} was refused ({restore}), so \
                                 item {} holds {origin:?} or {before:?} there",
                                item.id.0
                            )
                        };
                        return Err(noting(
                            error,
                            &format!(
                                "; {left}; next: set {ORIGIN_KEY} on it back to {before:?}, or \
                                 run the write again"
                            ),
                        ));
                    }
                }
            }
            return Err(error);
        }

        let written_status = match (incoming.written.work_status(), status_target.as_ref()) {
            (Some((kind, _)), Some(StatusTarget::Terminal(_, reason))) => {
                self.statuses
                    .status(kind, written_option.as_deref(), true, Some(reason.reason()))
            }
            (Some((kind, _)), Some(StatusTarget::Column(_))) => {
                self.statuses
                    .status(kind, written_option.as_deref(), false, None)
            }
            (Some((_, status)), _) => status.clone(),
            (None, _) => Status {
                category: StatusCategory::Unknown,
                name: "Open".to_owned(),
            },
        };

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
            raw_body: body.clone(),
            // A document has no status of its own; what it reads back as is whatever
            // the issue's own state says, which is what a re-read reports.
            status: written_status,
            option: written_option.or_else(|| existing.and_then(|item| item.option.clone())),
            priority: match incoming.priority {
                Some(priority) => HeldPriority::Read(priority),
                None => existing.map_or(HeldPriority::Read(Priority::None), |item| {
                    item.priority.clone()
                }),
            },
            // What `state_input` asked for: closed for a terminal target, open for any other
            // status, and the issue's own state left as it was by a document write.
            closed: content_kind == ContentKind::Issue
                && match status_target.as_ref() {
                    Some(StatusTarget::Terminal(_, _)) => true,
                    Some(_) => false,
                    None => existing.is_some_and(|item| item.closed),
                },
            delivers: incoming.delivers.to_vec(),
            delivered_by: incoming.delivered_by.to_vec(),
            labels: incoming.labels.to_vec(),
            parent: incoming.parent.cloned(),
            origin: (!origin.is_empty()).then(|| origin.to_owned()),
            number,
            // In the update path this is the item's own url, read off `existing` where the
            // record above was bound, so one expression serves both halves.
            url,
            created_at: existing.and_then(|item| item.created_at),
            updated_at: existing.and_then(|item| item.updated_at),
            own_repository,
            repositories: incoming.repositories.to_vec(),
            slot,
            board_id: Some(board.id.as_str().to_owned()),
            fields: board
                .fields
                .get("nodes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            board_fields: Some(board.fields.clone()),
            // What this write left the relationship holding is known by id alone, and a
            // later read of its edges needs each far end's kind, so it reads them again.
            blocked_by: None,
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
        board_id: &str,
        incoming: &Incoming<'_>,
        content_id: &NativeId,
        item_id: &str,
        content_kind: ContentKind,
        existing: Option<&Resolved>,
        origin_field: Option<&str>,
        origin: &str,
        column: Option<(String, String)>,
        status_target: Option<&StatusTarget>,
        priority: Option<&PriorityWrite>,
        native: &[String],
        origin_landed: &mut bool,
    ) -> Result<(), SourceError> {
        let mut fields = Vec::new();
        if let Some(field_id) = origin_field
            && existing.map_or(!origin.is_empty(), |item| {
                item.origin.as_deref().unwrap_or("") != origin
            })
        {
            fields.push((field_id.to_owned(), json!({"text":origin})));
        }
        if let Some((field_id, option_id)) = column {
            fields.push((field_id, json!({"singleSelectOptionId":option_id})));
        }
        let clear = match priority {
            Some(PriorityWrite::Select { field, option }) => {
                fields.push((field.clone(), json!({"singleSelectOptionId":option})));
                None
            }
            Some(PriorityWrite::Clear { field }) => Some(field.as_str()),
            None => None,
        };
        self.set_item_fields(board_id, item_id, &fields, clear)
            .await?;
        *origin_landed = true;

        // An existing issue closes in the `updateIssue` its write ends with; one created just
        // now closes here, once its option is selected.
        if existing.is_none()
            && content_kind == ContentKind::Issue
            && matches!(status_target, Some(StatusTarget::Terminal(_, _)))
        {
            self.update_content(
                ContentKind::Issue,
                content_id,
                json!({"stateInput":state_input(status_target)}),
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
                let issue = match existing {
                    Some(item) => Issue::Existing(item.blocked_by.as_deref()),
                    None => Issue::Created,
                };
                self.reconcile_blocked_by(content_id, native, issue).await?;
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
    /// gone, which is the state this asks for. Which that is, is decided by reading the item
    /// by its own id — a listing of the board can still be missing an item it holds, and
    /// reading that as *already gone* would leave behind the very item this was asked to
    /// take back.
    async fn delete_item(&self, id: &NativeId) -> Result<(), SourceError> {
        let Some(item) = self.bound_item(id).await? else {
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

    /// The issue a comment call on `task` is about, or `None` when this board holds no such
    /// task.
    ///
    /// Resolved exactly as [`TaskSource::get_task`] resolves it, so the comment verbs and a
    /// read of the task cannot disagree about which ids name one: a project or a document of
    /// this board is not a task here either.
    ///
    /// A **draft** is a task with nowhere to keep a comment, because GitHub keeps comments on
    /// issues and a draft is not one. It is refused rather than answered with an empty page,
    /// which would read as a task nobody has commented on yet.
    async fn commented_issue(&self, task: &NativeId) -> Result<Option<NativeId>, SourceError> {
        let cached = self.resolved_cache()?.get(task).cloned();
        let Some(item) = (match cached {
            Some(item) => Some(item),
            None => self.item_by_id(task).await?,
        })
        .filter(|item| item.kind == BoardKind::Work(ItemKind::Task)) else {
            return Ok(None);
        };
        if item.content_kind == ContentKind::DraftIssue {
            return Err(self.draft_has_no_comments(task));
        }
        Ok(Some(item.id))
    }

    /// The refusal a comment call on a board draft is answered with: GitHub keeps comments on
    /// issues, and a draft is not one.
    fn draft_has_no_comments(&self, task: &NativeId) -> SourceError {
        SourceError::Refused {
            message: format!(
                "task {} of source {} is a draft item on the board, and GitHub keeps \
                 comments on issues alone, so a draft has none to read or write; next: \
                 convert the draft to an issue on the board, then comment on the issue it \
                 becomes",
                task.0, self.name
            ),
        }
    }

    /// One task and a page of its comments, read with [`graphql::ISSUE_DETAIL`] in one
    /// request — or `None` when this board holds no task by that id.
    ///
    /// What `task show` and a comment listing read. A draft is a task with no comments, so it
    /// is answered with the draft and the refusal, at the price of the draft's own read.
    async fn issue_detail(
        &self,
        id: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<TaskDetailRead>, SourceError> {
        let after = page.cursor.as_ref().map(|cursor| cursor.0.as_str());
        let asked = self
            .graphql(
                graphql::ISSUE_DETAIL,
                json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),"after":after,
                       "nestedFirst":NESTED_PAGE_SIZE,"boardItems":BOARD_ITEMS_PAGE_SIZE,
                       "duplicates":true}),
            )
            .await;
        let data = match asked {
            Ok(data) => data,
            Err(error) if unresolvable_node(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        // `node` is null for an id that names nothing, and absent only from an answer this
        // source cannot read — never the same thing.
        let node = data.get("node").ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub answered the read of {} with no node", id.0),
        })?;
        self.detail_of(id, node, true, after).await
    }

    /// Several tasks, each with the first page of its comments when `comments` is set, read
    /// [`DETAIL_BATCH`] at a time with [`graphql::ISSUE_DETAILS`] — one answer per id, in
    /// order.
    ///
    /// A batch GitHub refuses because one of its ids resolves to no node at all is read again
    /// one item at a time, so that id is answered as missing and the others as themselves; any
    /// other refusal is every id of that batch's answer.
    async fn issue_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        let mut read = Vec::with_capacity(ids.len());
        for batch in ids.chunks(DETAIL_BATCH) {
            match self
                .graphql(graphql::ISSUE_DETAILS, detail_batch(batch, comments))
                .await
            {
                Ok(data) => {
                    for (slot, id) in batch.iter().enumerate() {
                        // Every alias asked for is answered, null for an id naming nothing;
                        // one missing is an answer this source cannot read.
                        let read_one = match data.get(format!("i{slot}")) {
                            Some(node) => self.detail_of(id, node, comments.is_some(), None).await,
                            None => Err(SourceError::Malformed {
                                message: format!(
                                    "GitHub answered a batch read with no item for {}",
                                    id.0
                                ),
                            }),
                        };
                        read.push(read_one);
                    }
                }
                Err(error) if unresolvable_node(&error) => {
                    for id in batch {
                        read.push(match comments {
                            Some(page) => self.issue_detail(id, page).await,
                            None => self.task_read(id).await,
                        });
                    }
                }
                Err(error) => read.extend(batch.iter().map(|_| Err(error.clone()))),
            }
        }
        read
    }

    /// One task and nothing of its comments, as [`TaskSource::get_task`] reads it.
    async fn task_read(&self, id: &NativeId) -> Result<Option<TaskDetailRead>, SourceError> {
        Ok(self.get_task(id).await?.map(|task| TaskDetailRead {
            task,
            comments: None,
        }))
    }

    /// What one node a detail read reached says: the task this board holds by `id`, with the
    /// page of comments the node carries when `commented` — or `None` for a node that is no
    /// task of this board.
    ///
    /// Resolved as [`Self::item_by_id`] resolves an item: a draft is read again as a draft,
    /// and an item this process created answers from this process's own record, which a node
    /// read taken moments after the write can still be behind.
    async fn detail_of(
        &self,
        id: &NativeId,
        node: &Value,
        commented: bool,
        after: Option<&str>,
    ) -> Result<Option<TaskDetailRead>, SourceError> {
        if node.is_null() {
            return Ok(None);
        }
        let draft = optional_str(node, "__typename")? == Some("DraftIssue");
        // An issue answered under one id is that id's, or the answer is not one this source
        // can report: reporting another issue's task and comments under the qualified id asked
        // for would be the one wrong answer here. A draft's own read checks the same.
        if !draft
            && optional_str(node, "__typename")? == Some("Issue")
            && required_str(node, "id")? != id.0
        {
            return Err(SourceError::Malformed {
                message: format!(
                    "GitHub answered the read of {} with issue {}",
                    id.0,
                    required_str(node, "id")?
                ),
            });
        }
        let item = if draft {
            self.draft_by_id(id).await?
        } else {
            self.resolve_issue(node).await?
        };
        let Some(item) = item.filter(|item| item.kind == BoardKind::Work(ItemKind::Task)) else {
            return Ok(None);
        };
        let own = self.created()?.iter().find(|own| own.id == *id).cloned();
        let task = own.unwrap_or(item).task()?;
        let comments = match (commented, draft) {
            (false, _) => None,
            (true, true) => Some(Err(self.draft_has_no_comments(id))),
            (true, false) => Some(comment_page(node, &id.0, after).map(Some)),
        };
        Ok(Some(TaskDetailRead { task, comments }))
    }

    /// Whether the comment `comment` is one of `issue`'s own.
    ///
    /// Read before an edit or a removal is sent, because GitHub's comment mutations take the
    /// comment's id and nothing else: a comment id given against the wrong task would
    /// otherwise change a comment on some other issue entirely. An id that names nothing, or
    /// names something that is not an issue comment, is a comment this task does not have —
    /// which is what GitHub refusing to resolve it means too.
    async fn comment_is_on(
        &self,
        issue: &NativeId,
        comment: &NativeId,
    ) -> Result<bool, SourceError> {
        let asked = self
            .graphql(graphql::COMMENT_ISSUE, json!({"id":comment.0}))
            .await;
        let data = match asked {
            Ok(data) => data,
            Err(error) if unresolvable_node(&error) => return Ok(false),
            Err(error) => return Err(error),
        };
        let Some(node) = data.get("node").filter(|value| !value.is_null()) else {
            return Ok(false);
        };
        if optional_str(node, "__typename")? != Some("IssueComment") {
            return Ok(false);
        }
        let on = node.get("issue").ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub issue comment {} names no issue", comment.0),
        })?;
        Ok(required_str(on, "id")? == issue.0)
    }

    /// Which far ends this item's own `blockedBy` relationship holds, and which it cannot.
    async fn partition_edges(
        &self,
        near_kind: BoardKind,
        near_content: ContentKind,
        carried: Option<&[Value]>,
        depends_on: &[DependencyEdge],
    ) -> Result<(Vec<String>, Vec<DependencyEdge>), SourceError> {
        let mut native = Vec::new();
        let mut fallback = Vec::new();
        let far_ends: Vec<(&DependencyEdge, &str, bool, Option<&Value>)> = depends_on
            .iter()
            .map(|edge| {
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
                // One that already blocks the near issue was answered by that issue's own
                // read, which carried each of its blockers' kinds — an issue every one — so it
                // is not read again.
                let blocking = carried.and_then(|nodes| {
                    nodes
                        .iter()
                        .find(|node| node.get("id").and_then(Value::as_str) == Some(far_id))
                });
                (edge, far_id, same_source, blocking)
            })
            .collect();
        // Every other same-source far end is read by its own id, exactly as the item it is a
        // far end of is: whether this board holds it is that read's answer, never a listing's.
        // They are read together, [`DETAIL_BATCH`] to a request, rather than one each.
        let mut unread: Vec<NativeId> = Vec::new();
        for (_, far_id, same_source, blocking) in &far_ends {
            let id = NativeId((*far_id).to_owned());
            if *same_source && blocking.is_none() && !unread.contains(&id) {
                unread.push(id);
            }
        }
        let read: BTreeMap<NativeId, Option<Resolved>> = unread
            .iter()
            .cloned()
            .zip(self.items_by_ids(&unread).await?)
            .collect();
        for (edge, far_id, same_source, blocking) in far_ends {
            let far = match (same_source, blocking) {
                (false, _) => None,
                (true, Some(node)) => Some(FarEnd {
                    kind: if required_str(node, "title")?.starts_with(DESIGN_TITLE_PREFIX) {
                        BoardKind::Document
                    } else {
                        BoardKind::Work(related_kind(node)?)
                    },
                    content_kind: ContentKind::Issue,
                }),
                (true, None) => {
                    let read = read
                        .get(&NativeId(far_id.to_owned()))
                        .cloned()
                        .flatten()
                        .ok_or_else(|| SourceError::Refused {
                            message: format!("GitHub dependency item {far_id} was not found"),
                        })?;
                    Some(FarEnd {
                        kind: read.kind,
                        content_kind: read.content_kind,
                    })
                }
            };
            let far = far.as_ref();
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
        // A terminal status closes the issue here, in the same mutation as its body: its board
        // option was selected before this, so a close never lands on an item whose board cannot
        // show it.
        let fields = match item.content_kind {
            ContentKind::DraftIssue => json!({"title":title,"body":body}),
            ContentKind::Issue => json!({"title":title,"body":body,
                                         "stateInput":state_input(status_target)}),
        };
        self.update_content(item.content_kind, &item.id, fields)
            .await
    }

    /// Update one board item's content with exactly `fields` beside its id, through the
    /// mutation its kind takes: `updateIssue` for an issue, `updateProjectV2DraftIssue` for
    /// a draft.
    ///
    /// Every input field either mutation leaves out is a field GitHub leaves as it is, which
    /// is what lets a narrow write carry the one thing it changes and nothing else.
    async fn update_content(
        &self,
        kind: ContentKind,
        id: &NativeId,
        fields: Value,
    ) -> Result<(), SourceError> {
        let (operation, id_key, pointer) = match kind {
            ContentKind::DraftIssue => (
                graphql::UPDATE_DRAFT,
                "draftIssueId",
                "/updateProjectV2DraftIssue/draftIssue",
            ),
            ContentKind::Issue => (graphql::UPDATE_ISSUE, "id", "/updateIssue/issue"),
        };
        let mut input = fields;
        input[id_key] = json!(id.0);
        let data = self.graphql(operation, json!({"input":input})).await?;
        let returned = data
            .pointer(pointer)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub item update returned no item".into(),
            })?;
        if required_str(returned, "id")? != id.0 {
            return Err(SourceError::Malformed {
                message: "GitHub item update returned the wrong item".into(),
            });
        }
        Ok(())
    }

    /// Creates one issue, files it on the board, and reports what a read of it would say:
    /// its content id, its board item id, and the web address GitHub gave it.
    ///
    /// Two calls rather than one: `createIssue` answers with an issue that is on no board,
    /// and `addProjectV2ItemById` is what puts it there. Filing it at creation through
    /// `CreateIssueInput.projectV2Ids` was tried and is not done: GitHub answered with no
    /// board item, and the `addProjectV2ItemById` that then had to follow was refused
    /// "Content already exists in this project". A terminal status is not written here:
    /// `finish_write` selects its option first and closes the issue after, so a close never
    /// lands on an item whose board cannot show it.
    ///
    /// The address and the number come back here because this is the only place either is
    /// known before GitHub's own board read catches up — an item this run created answers
    /// the reads that follow it out of the record below, and one remembered without them
    /// would report no location and no key for the rest of the run.
    async fn create_and_file_issue(
        &self,
        board_id: &str,
        repository: &RepositoryTarget,
        incoming: &Incoming<'_>,
        body: &Option<String>,
    ) -> Result<Landed, SourceError> {
        let repository_id = self.repository_id(repository, incoming).await?;
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
        // The issue exists from here on, so an unreadable number and a refused board
        // filing below each try, best effort, to take it back: an issue in the repository
        // that is on no board is an item nobody asked for and nothing here would find again.
        //
        // Its number is optional on the same terms its address is — a landed write is not
        // worth failing over a member that came back missing, and such an item reports no
        // handle until a board read catches up. A number that is *present* and is not an
        // unsigned integer is still a response this source cannot read.
        let number = match created_issue_number(created) {
            Ok(number) => number,
            Err(error) => {
                let _ = self.delete_issue(&content_id).await;
                return Err(error);
            }
        };
        let added = match self
            .graphql(
                graphql::ADD_TO_BOARD,
                json!({"input":{"projectId":board_id,"contentId":content_id.0}}),
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
        Ok(Landed {
            content_id,
            item_id: required_str(item, "id")?.to_owned(),
            url,
            number,
        })
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

    /// Bring one issue's `blockedBy` to exactly `native`, sending only the difference, and say
    /// whether there was one.
    ///
    /// An issue [`Issue::Created`] by this very write is blocked by nothing yet, so its
    /// relationships are not read: there is nothing a read of them could find.
    async fn reconcile_blocked_by(
        &self,
        content_id: &NativeId,
        native: &[String],
        issue: Issue<'_>,
    ) -> Result<bool, SourceError> {
        let current = match issue {
            Issue::Created => Vec::new(),
            Issue::Existing(Some(held)) => held
                .iter()
                .map(|far| required_str(far, "id").map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()?,
            Issue::Existing(None) => self.native_dependency_ids(content_id).await?,
        };
        let mut changed = false;
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
            changed = true;
        }
        Ok(changed)
    }
}

/// What a write needs to know of one far end it names: which kind of item it is, and whether
/// it is an issue a native relationship can name.
struct FarEnd {
    kind: BoardKind,
    content_kind: ContentKind,
}

/// Whether the issue one write reconciles was created by that write or was already there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Issue<'a> {
    /// Created by this write, so it holds no relationships yet.
    Created,
    /// On the board before this write, holding whatever relationships it holds — the far
    /// ends of its whole `blockedBy`, when the read that reached it carried them.
    Existing(Option<&'a [Value]>),
}

/// What resolving one node id reached; see [`GitHubProjectsSource::reach`].
enum Reached {
    /// An issue this board holds, resolved into everything this source reports about it.
    Held(Box<Resolved>),
    /// Nothing this board holds: no such node, or a node on some other board.
    Nothing,
    /// A board draft, which [`graphql::ISSUE`] reaches and reads nothing of, so it is read
    /// again by [`GitHubProjectsSource::draft_by_id`].
    Draft,
}

/// What GitHub says when a string is not a node id it can resolve.
///
/// Matched because it is the ordinary answer to a project selector naming a project by its
/// *name*, and reporting that as a failure would make naming one impossible. It is read
/// off the refusal GitHub sent, never guessed from the shape of the string: this source
/// does not define the syntax of a GitHub node id and would be wrong about it.
const UNRESOLVABLE_NODE: &str = "could not resolve to a node";

/// `error` with `note` added to the end of what it says, its kind and every other member
/// unchanged — so a caller still branches on the failure that happened, and reads beside it
/// what that failure left behind.
fn noting(error: SourceError, note: &str) -> SourceError {
    match error {
        SourceError::Config { message } => SourceError::Config {
            message: message + note,
        },
        SourceError::Auth { message } => SourceError::Auth {
            message: message + note,
        },
        SourceError::Refused { message } => SourceError::Refused {
            message: message + note,
        },
        SourceError::RateLimited {
            retry_after_seconds,
            message,
        } => SourceError::RateLimited {
            retry_after_seconds,
            message: Some(message.unwrap_or_default() + note),
        },
        SourceError::Unavailable { message } => SourceError::Unavailable {
            message: message + note,
        },
        SourceError::Malformed { message } => SourceError::Malformed {
            message: message + note,
        },
    }
}

/// The variables of one [`graphql::ISSUE_DETAILS`] request over `batch` — at most
/// [`DETAIL_BATCH`] ids — each item with the first page of its comments when `comments` asks
/// for them.
///
/// The document is fixed-size, so a slot `batch` has no id for is bound to its last id, which
/// is read again at no added price.
fn detail_batch(batch: &[NativeId], comments: Option<&PageRequest>) -> Value {
    let mut variables = serde_json::Map::new();
    for slot in 0..DETAIL_BATCH {
        let id = batch.get(slot).or(batch.last()).map(|id| id.0.clone());
        variables.insert(format!("id{slot}"), json!(id));
    }
    variables.insert(
        "first".to_owned(),
        json!(comments.map_or(MAX_PAGE_SIZE, |page| page.limit.min(MAX_PAGE_SIZE))),
    );
    variables.insert("comments".to_owned(), json!(comments.is_some()));
    variables.insert("nestedFirst".to_owned(), json!(NESTED_PAGE_SIZE));
    variables.insert("boardItems".to_owned(), json!(BOARD_ITEMS_PAGE_SIZE));
    variables.insert("duplicates".to_owned(), json!(true));
    Value::Object(variables)
}

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
    format!("in:title {}", quoted(name))
}

/// `value` as one quoted phrase of a GitHub search or a board filter, with the two
/// characters GitHub's quoting grammar gives a meaning inside a quoted phrase escaped the way
/// it documents — so a value holding a qualifier's spelling is searched for rather than
/// obeyed.
fn quoted(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The search qualifier for the issues updated at or after `since`.
///
/// Written to the second, rounded down, which can only widen what the search returns.
fn updated_qualifier(since: DateTime<Utc>) -> String {
    format!("updated:>={}", since.format("%Y-%m-%dT%H:%M:%S+00:00"))
}

/// The search terms that narrow a board-scoped issue search to a task query's text and
/// metadata predicates, or `None` when it carries neither.
///
/// The text is one quoted phrase, searched `in:title`, `in:body` or both as its fields say,
/// and each metadata value is one more quoted phrase, which GitHub finds in the body because
/// its index covers the metadata comment the value is stored in. GitHub ANDs the phrases and
/// matches each in any field the `in:` qualifier names, so a query naming a title search and
/// a metadata value searches both fields for both — wider than asked, never narrower, and
/// every candidate is confirmed in process afterwards.
///
/// **This narrows a text search, and that is this source's declared semantics.** GitHub
/// matches whole tokens where a substring rule would match inside a word, so an item holding
/// the text only inside a longer word is not returned. A text of nothing but whitespace
/// matches every item, so it narrows nothing and is not sent.
fn narrowing_qualifiers(query: &TaskQuery) -> Option<String> {
    let text = query
        .text
        .as_ref()
        .filter(|text| !text.terms.trim().is_empty());
    if text.is_none() && query.metadata.is_empty() {
        return None;
    }
    let (title, body) = match text.map(|text| text.fields) {
        None => (false, true),
        Some(TextFields::Title) => (true, !query.metadata.is_empty()),
        Some(TextFields::Content) => (false, true),
        Some(TextFields::TitleOrContent) => (true, true),
    };
    let fields = match (title, body) {
        (true, true) => "in:title,body",
        (true, false) => "in:title",
        _ => "in:body",
    };
    let phrases = text
        .map(|text| text.terms.clone())
        .into_iter()
        .chain(
            query
                .metadata
                .iter()
                .map(|wanted| as_stored(wanted.value())),
        )
        .map(|phrase| quoted(&phrase))
        .collect::<Vec<_>>();
    Some(format!("{fields} {}", phrases.join(" ")))
}

/// The search terms that narrow a board-scoped issue search to a project or document query's
/// text, or `None` when it has none or a blank one: the phrase, in the fields, a task query
/// carrying that text alone is sent as by [`narrowing_qualifiers`].
fn text_qualifiers(text: Option<&TextQuery>) -> Option<String> {
    narrowing_qualifiers(&TaskQuery {
        text: text.cloned(),
        ..TaskQuery::default()
    })
}

/// Refuses a project or document query's text GitHub's issue search cannot find, before
/// anything is asked of GitHub, on exactly the terms [`refuse_unsearchable`] refuses a task
/// query's.
fn refuse_unsearchable_text(text: Option<&TextQuery>) -> Result<(), SourceError> {
    refuse_unsearchable(&TaskQuery {
        text: text.cloned(),
        ..TaskQuery::default()
    })
}

/// Refuses a task query naming a text or a metadata value GitHub's issue search cannot find,
/// before anything is asked of GitHub.
///
/// GitHub's index holds words, so a phrase with no letter or digit names none to find, and no
/// bounded query answers it: sent, GitHub's answer to it is nothing this source may rely on;
/// left out, the search is every issue of the board. So this source says it cannot answer
/// rather than reading the board or answering nothing. A blank text is not refused: it narrows
/// nothing GitHub could search for, and keeps the board read it always had.
fn refuse_unsearchable(query: &TaskQuery) -> Result<(), SourceError> {
    const WHY: &str = "GitHub's issue search indexes words, so it cannot answer a value with no \
                       letter or digit with a bounded query";
    if let Some(text) = &query.text
        && !text.terms.trim().is_empty()
        && !has_words(&text.terms)
    {
        return Err(SourceError::Refused {
            message: format!(
                "cannot search for the text {:?}: {WHY}; search for a text holding a letter or a digit",
                text.terms
            ),
        });
    }
    if let Some(wanted) = query
        .metadata
        .iter()
        .find(|wanted| !has_words(wanted.value()))
    {
        return Err(SourceError::Refused {
            message: format!(
                "cannot filter by the metadata value {:?} at {:?}: {WHY}; filter by a value holding a letter or a digit",
                wanted.value(),
                std::iter::once(wanted.key())
                    .chain(wanted.path().iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("/"),
            ),
        });
    }
    Ok(())
}

/// Whether GitHub's index could hold a word of `phrase`: whether it has a letter or a digit.
fn has_words(phrase: &str) -> bool {
    phrase.chars().any(char::is_alphanumeric)
}

/// `value` spelled the way the metadata slot stores it: as the inside of its JSON string.
///
/// What GitHub indexes is the slot's JSON text, so a value holding a character JSON escapes —
/// a newline, a tab, a quote — is found by the escape the body holds and not by the character,
/// which GitHub's word match would read as different words.
fn as_stored(value: &str) -> String {
    let encoded = Value::String(value.to_owned()).to_string();
    encoded[1..encoded.len() - 1].to_owned()
}

/// The one narrower question a task query carrying a text, metadata or origin predicate is
/// sent as.
enum Narrowing {
    /// Every carrier of this origin: [`graphql::ORIGIN_LOOKUP`].
    Origin(String),
    /// The board-scoped issue search narrowed by these qualifiers.
    Search(String),
}

impl Narrowing {
    /// What this question is remembered under for the length of one command.
    fn key(&self) -> String {
        match self {
            Self::Origin(origin) => format!("origin {origin}"),
            Self::Search(also) => format!("search {also}"),
        }
    }
}

/// Where one connection of [`graphql::ORIGIN_LOOKUP`] resumes.
enum Resumed {
    /// It reported another page, which starts after this cursor.
    More(String),
    /// It has ended. Sending this cursor again — the page's own end when it had one, and
    /// otherwise the cursor it was reached from — answers an empty page, so the one document
    /// can go on walking the other connection.
    Ended(Option<String>),
}

impl Resumed {
    /// Whether the connection has another page.
    const fn has_more(&self) -> bool {
        matches!(self, Self::More(_))
    }

    /// The cursor to send this connection next.
    fn cursor(self) -> Option<String> {
        match self {
            Self::More(next) => Some(next),
            Self::Ended(last) => last,
        }
    }
}

/// Where `connection`, reached from `after`, resumes — refused when it reports another page
/// with no cursor to it, or from a cursor that does not advance.
fn resumed(connection: &Value, after: Option<&str>) -> Result<Resumed, SourceError> {
    let info = connection
        .get("pageInfo")
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub connection has no pageInfo".into(),
        })?;
    let end = optional_str(info, "endCursor")?;
    if required_bool(info, "hasNextPage")? {
        let next = end.ok_or_else(|| SourceError::Malformed {
            message: "GitHub connection reports another page and no endCursor".into(),
        })?;
        validate_cursor_progress(after, next)?;
        return Ok(Resumed::More(next.to_owned()));
    }
    Ok(Resumed::Ended(
        end.map(str::to_owned).or_else(|| after.map(str::to_owned)),
    ))
}

/// The board, and every item on it this source reports.
#[derive(Clone)]
struct Board {
    id: String,
    fields: Value,
    items: Vec<Resolved>,
}

/// What a write needs of the board and nothing more: its node id and its field
/// definitions, in the shape a read of the board's own `fields` gives them.
///
/// Deliberately no items. A write decides which item it writes, which parent it files
/// under and which far ends it names by reading each of them by its own id; this is the
/// half of the board those reads cannot carry, and holding no item is what keeps it from
/// ever being asked whether an item is there.
#[derive(Clone)]
struct BoardFields {
    id: BoardId,
    fields: Value,
}

/// A board's node id: what a field write and `addProjectV2ItemById` address.
///
/// Never blank, because a blank one addresses no board — so an id GitHub answers blank is
/// refused where it is read, and one an item names blank is read as not named at all.
#[derive(Clone)]
struct BoardId(String);

/// Where one write left its item, for the record the rest of the command reads it out of.
///
/// A named record rather than a tuple because the update arm and the create arm each fill
/// all four, and two `Option`s of different meaning side by side in a tuple are two
/// positions a reader has to count.
struct Landed {
    /// The issue's own node id, which is the [`NativeId`] this source reports.
    content_id: NativeId,
    /// The board item's id, which is what a field write addresses.
    // llmlint: ignore[invalid_states_unrepresentable] This field and the one below are `Resolved::item_id` and `Resolved::url` carried out of one call: the update arm assigns them from an existing `Resolved` and the whole record is assigned straight back into one. A newtype introduced here alone would be wrapped at both of those boundaries and unwrapped at every use, and would make this private record disagree with the type the same values have on the struct they come from and return to. Where the board item id gets a newtype is on `Resolved`, which is the contract's own shape and not this change's to move.
    item_id: String,
    /// The web address GitHub gave the issue, when it gave one.
    // llmlint: ignore[invalid_states_unrepresentable] The answer `Resolved::url` and the contract's `Task::url` already record: a web address this source never parses, resolves or compares — it reads GitHub's string and hands it back, and `Location::Url` is where the contract gives it a shape. Validating it here would have this plugin decide what GitHub may call an address.
    url: Option<String>,
    /// The issue's number on its repository, when GitHub reported one.
    number: Option<u64>,
}

impl BoardId {
    fn parse(id: &str) -> Result<Self, SourceError> {
        if id.trim().is_empty() {
            return Err(SourceError::Malformed {
                message: "GitHub named a board with a blank node id".into(),
            });
        }
        Ok(Self(id.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
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
    /// The body exactly as GitHub holds it, metadata slot and all, which is what a write
    /// that changes the slot alone has to keep byte for byte outside it.
    raw_body: Option<String>,
    status: Status,
    /// The name of the board `Status` option this item sits in, as the board spells it.
    option: Option<String>,
    /// What its `Priority` field says, read through this instance's mapping.
    priority: HeldPriority,
    /// Whether this item's issue is closed. A draft has no such state and is never closed.
    closed: bool,
    /// The tasks this one delivers, read out of its slot. Empty for anything not a task.
    delivers: Vec<TaskRef>,
    /// Every task that delivers this one, read out of its slot. Empty for anything not a
    /// task.
    delivered_by: Vec<TaskRef>,
    labels: Vec<Label>,
    parent: Option<NativeId>,
    // llmlint: ignore[invalid_states_unrepresentable] The write side's reason, read back: this is the engine's qualified id, taken out of a board text field and handed on untouched. A newtype here would have this plugin define the syntax of an id `docs/metadata.md` says no plugin ever constructs or interprets.
    origin: Option<String>,
    /// The issue's own number on its repository, as GitHub reports it.
    ///
    /// `None` in exactly two cases: a draft, which has no number at all — `DraftIssue`
    /// declares none, and a draft is not filed in a repository to be numbered by one — and
    /// an issue this run created whose creating mutation answered without one, which is a
    /// response GitHub's own schema says cannot happen and which a landed write is not
    /// worth failing over. An `Issue` read off the board always has one.
    number: Option<u64>,
    url: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    own_repository: Option<Repository>,
    repositories: Vec<Repository>,
    slot: BTreeMap<String, Value>,
    /// The node id of the board this item sits on, when the read that reached it said.
    board_id: Option<String>,
    /// The definition of every board field this item holds a value of, in the shape a read
    /// of the board's own `fields` gives one.
    ///
    /// Only the fields this item has a value in: a field it holds nothing of is not here,
    /// which says nothing about whether the board has it.
    fields: Vec<Value>,
    /// Every field the board this item sits on defines, as its own read of the board's
    /// `fields` gives them — when the read that reached the item carried them, which a read
    /// of it by its own id does. What a write of it needs of the board, then, needs no read
    /// of the board.
    board_fields: Option<Value>,
    /// The far ends of this issue's whole `blockedBy` connection, each as a dependency read
    /// selects one — when the read that reached it carried the connection to its end, which a
    /// read of it by its own id does for any issue blocked by no more than a page. What a
    /// write reconciles that relationship against, and what a read of its forward edges in
    /// the same command answers with.
    blocked_by: Option<Vec<Value>>,
}

impl Resolved {
    /// The board this item's own read names it on, when that read named one this source can
    /// address.
    fn named_board(&self) -> Option<BoardId> {
        self.board_id
            .as_deref()
            .and_then(|id| BoardId::parse(id).ok())
    }

    /// The board's id and every field it defines, when the read that reached this item
    /// carried both — which a read of it by its own id does.
    fn carried_board(&self) -> Option<BoardFields> {
        Some(BoardFields {
            id: self.named_board()?,
            fields: self.board_fields.clone()?,
        })
    }

    /// Whether this item holds a value of the board field called `name`, and so carries
    /// that field's definition. `false` says nothing about whether the board has the field.
    fn defines(&self, name: &str) -> bool {
        self.fields
            .iter()
            .any(|field| field.get("name").and_then(Value::as_str) == Some(name))
    }

    /// The metadata a caller sees: their own keys, plus the copy origin this source keeps
    /// in a field of its own, and none of the five keys that are only an encoding.
    ///
    /// The two delivery keys are left out for every kind, not only for a task: they are
    /// the encoding of [`Task::delivers`] and [`Task::delivered_by`], and a project or a
    /// document carrying one holds nothing a caller's own metadata could mean by it.
    fn metadata(&self) -> BTreeMap<String, Value> {
        let mut metadata = self.slot.clone();
        metadata.remove(Repository::METADATA_KEY);
        metadata.remove(DependencyEdge::RECORDED_KEY);
        metadata.remove(ItemKind::METADATA_KEY);
        metadata.remove(TaskRef::DELIVERS_KEY);
        metadata.remove(TaskRef::DELIVERED_BY_KEY);
        // The board field is the origin, and the body's copy of it is only a mirror for the
        // issue search to find: an item whose field holds none has none, whatever its body
        // says, so no reader ever sees two answers.
        metadata.remove(ORIGIN_KEY);
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

    /// The short handle this board's backend shows people for a task: the issue's number
    /// alone, as a decimal string.
    ///
    /// The number alone rather than `owner/repo#1043`, because that is the contract's
    /// value for this backend. A draft has no number and so no handle, which is the
    /// contract's *absent* rather than a handle of some other shape — and the native
    /// [`Task::id`] here is the issue's GraphQL node id, which this neither replaces nor
    /// derives from.
    fn key(&self) -> Option<String> {
        self.number.map(|number| number.to_string())
    }

    /// Whether its `Priority` field holds a value at all, mapped or not.
    fn holds_priority(&self) -> bool {
        self.priority != HeldPriority::Read(Priority::None)
    }

    /// The task this item is.
    ///
    /// Fails for an item whose `Priority` field holds an option the mapping does not name:
    /// reading that as a level would be a guess, and reading it as `none` would let the next
    /// copy clear a priority a person set.
    fn task(&self) -> Result<Task, SourceError> {
        let priority = match &self.priority {
            HeldPriority::Read(priority) => *priority,
            HeldPriority::Unmapped(option) => {
                return Err(SourceError::Malformed {
                    message: format!(
                        "task {}{} sits in the board {PRIORITY_FIELD} option {option:?}, which \
                         this source's priority_mapping does not name, so its priority cannot be \
                         read; next: name {option:?} under priority_mapping, or move the item to \
                         a mapped option",
                        self.id,
                        self.number
                            .map(|number| format!(" (#{number})"))
                            .unwrap_or_default()
                    ),
                });
            }
        };
        Ok(Task {
            id: self.id.clone(),
            key: self.key(),
            title: self.title.clone(),
            content: self.body.clone(),
            status: self.status.clone(),
            priority,
            labels: self.labels.clone(),
            project: self.parent.clone(),
            url: self.url.clone(),
            location: self.location(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
            delivers: self.delivers.clone(),
            delivered_by: self.delivered_by.clone(),
        })
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

/// Where one targeted update moves an item's status, and which of its two halves move.
struct StatusMove {
    /// The board the item's `Status` field is on.
    board: BoardId,
    /// The `Status` field's id.
    field: String,
    /// The option's id.
    option: String,
    /// The option's name, as the board spells it.
    name: String,
    /// What the status asks of the issue's state.
    target: StatusTarget,
    /// The status the item reads as once it is there.
    landed: Status,
    /// Which of the status's two halves differ from what the item holds.
    moves: Moves,
}

/// Which halves of an item's status one targeted update moves: its `Status` option, the open or
/// closed state of its issue, or both. A status neither half of which differs is no move at all,
/// and is not a value of this type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Moves {
    /// The option alone.
    Option,
    /// The issue's state alone: open, closed, or closed with another reason.
    State,
    /// Both.
    Both,
}

impl Moves {
    /// What differs, or `None` when nothing does.
    const fn of(option: bool, state: bool) -> Option<Self> {
        match (option, state) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Option),
            (false, true) => Some(Self::State),
            (false, false) => None,
        }
    }

    /// Whether the option moves.
    const fn option(self) -> bool {
        matches!(self, Self::Option | Self::Both)
    }

    /// Whether the issue's state moves.
    const fn state(self) -> bool {
        matches!(self, Self::State | Self::Both)
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

    /// The status this write carries with the kind whose half of `status_mapping` it is
    /// written through.
    const fn work_status(&self) -> Option<(ItemKind, &Status)> {
        match self {
            Self::Document => None,
            Self::Work(kind, status) => Some((*kind, status)),
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
    /// [`Task::delivers`], already checked. Empty for a project or a document, which is
    /// what keeps either key out of their slot.
    delivers: &'a [TaskRef],
    /// [`Task::delivered_by`], already checked. Empty for a project or a document.
    delivered_by: &'a [TaskRef],
    /// [`Task::priority`], for a task written to an instance that holds one; `None` for a
    /// project, a document, and every write to an instance with no `priority_mapping` —
    /// which is what keeps such a write's requests exactly what they were before.
    priority: Option<Priority>,
}

/// What one write does to an item's `Priority` field.
enum PriorityWrite {
    /// Select this option of this field.
    Select {
        /// The `Priority` field's id.
        field: String,
        /// The mapped option's id.
        option: String,
    },
    /// Clear the field's value, which is what `none` is.
    Clear {
        /// The `Priority` field's id.
        field: String,
    },
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
    /// Whose half of `status_mapping` an item of this kind reads its status through. A
    /// document has no status of its own, so the task half stands in for whatever the issue
    /// holds; nothing reports it.
    const fn status_kind(self) -> ItemKind {
        match self {
            Self::Document => ItemKind::Task,
            Self::Work(kind) => kind,
        }
    }

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
        && (query.priorities.is_empty() || query.priorities.contains(&task.priority))
        && match project {
            ProjectFilter::Any => true,
            ProjectFilter::Orphans => task.project.is_none(),
            ProjectFilter::Is(id) => task.project.as_ref() == Some(id),
        }
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&task.title, task.content.as_deref(), text))
        // Against the parsed metadata slot, and against the origin field, which is where
        // `Resolved::metadata` reads each of them from.
        && query.metadata_matches(&task.metadata)
        && query.origin_matches(&task.metadata)
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
            comments: Support::Native,
            priority: if self.priorities.is_some() {
                Support::Native
            } else {
                Support::Unsupported
            },
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
        self.item_by_id(id)
            .await?
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
            .map(|item| item.task())
            .transpose()
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
        refuse_unsearchable(query)?;
        if query.origin.is_none() && !matches!(query.project, ProjectFilter::Is(_)) {
            let qualifiers = match (narrowing_qualifiers(query), query.commented_since) {
                (Some(also), Some(since)) => Some(format!("{} {also}", updated_qualifier(since))),
                (Some(also), None) => Some(also),
                (None, Some(since)) => Some(updated_qualifier(since)),
                (None, None) => None,
            };
            if let Some(also) = qualifiers {
                return self.search_tasks(query, page, &also).await;
            }
        }

        // A read narrowed to one project asks that project for its own tasks, so nothing
        // about it costs what the rest of the board holds. A read carrying a text, metadata
        // or origin predicate asks GitHub the narrower question those predicates are, and a
        // read narrowed to comment activity alone asks the board's own issue search for the
        // issues updated since, which is every issue a comment could have been written or
        // edited on since. Every other task read is a question about the whole board and is
        // answered by reading it.
        let (held, membership) = match (&query.project, query.commented_since) {
            (ProjectFilter::Is(project), _) => (
                self.project_children(project).await?,
                // Answered by where these items came from; see `task_matches`.
                &ProjectFilter::Any,
            ),
            (ProjectFilter::Any | ProjectFilter::Orphans, since) => {
                match (self.narrowed(query).await?, since) {
                    (Some(narrowed), _) => (narrowed, &query.project),
                    (None, Some(since)) => (self.updated_since(since).await?, &query.project),
                    (None, None) => (self.board().await?.items, &query.project),
                }
            }
        };
        // Filtered before paged: a page of a filtered result is a page of the survivors,
        // never the survivors of a page.
        let mut tasks = Vec::new();
        for item in held
            .iter()
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        {
            let task = item.task()?;
            if task_matches(&task, query, membership)
                && self.commented_since(item, query.commented_since).await?
            {
                tasks.push(task);
            }
        }
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
        refuse_unsearchable_text(query.text.as_ref())?;
        // The projects a board holds are found by an issue search scoped to that board,
        // never by walking the board's own item connection: what tells a project from a
        // task is the `parent` each issue carries, which costs nothing to read. A query
        // carrying a text asks that search for the text too, so it reads the issues that
        // hold it rather than every issue of the board.
        let held = match self.text_searched(query.text.as_ref()).await? {
            Some(searched) => searched,
            None => self.board_issues().await?,
        };
        let projects = held
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
        // and which of them come back is the kind this caller asked for. Unscoped, a query
        // carrying a text asks the board-scoped issue search for it, as a task query does,
        // and only one carrying none reads the board.
        let (held, membership) = match &query.project {
            ProjectFilter::Is(project) => (
                self.project_children(project).await?,
                // Answered by where these items came from; see `task_matches`.
                &ProjectFilter::Any,
            ),
            ProjectFilter::Any | ProjectFilter::Orphans => {
                refuse_unsearchable_text(query.text.as_ref())?;
                match self.text_searched(query.text.as_ref()).await? {
                    Some(searched) => (searched, &query.project),
                    None => (self.board().await?.items, &query.project),
                }
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

    /// Create or update one task.
    ///
    /// Its `delivers` and `delivered_by` are checked before anything is read or written —
    /// neither may name the task itself or name one task twice — and land in the body's
    /// metadata slot under their reserved keys, in place of any caller metadata of those
    /// names.
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        let near = write.target.as_ref().unwrap_or(&write.item.id);
        for (key, entries) in [
            (TaskRef::DELIVERS_KEY, &write.item.delivers),
            (TaskRef::DELIVERED_BY_KEY, &write.item.delivered_by),
        ] {
            TaskRef::listed(key, near, Some(&self.name), entries.clone())
                .map_err(|message| SourceError::Refused { message })?;
        }
        if self.priorities.is_none() && write.item.priority != Priority::None {
            return Err(self.holds_no_priority());
        }
        self.write_item(
            &Incoming {
                written: Written::Work(ItemKind::Task, &write.item.status),
                title: &write.item.title,
                content: write.item.content.as_deref(),
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: write.item.project.as_ref(),
                delivers: &write.item.delivers,
                delivered_by: &write.item.delivered_by,
                priority: self.priorities.as_ref().map(|_| write.item.priority),
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
                delivers: &[],
                delivered_by: &[],
                priority: None,
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
                delivers: &[],
                delivered_by: &[],
                priority: None,
            },
            write.target.as_ref(),
            &[],
        )
        .await
    }

    /// Refused exactly as the write refuses it, from what the write reads: the mapping first,
    /// which reads nothing; then the board's `Status` option. Over an existing item that is
    /// read off the item, as the write reads it, and the item is held among this command's
    /// resolved records so the write that follows reuses that read rather than repeating it;
    /// an item that does not carry the field takes the board's fields, which are held once
    /// read. A create is checked against the board's fields only when this command already
    /// holds them, because a create reads them together with its repository, in one request,
    /// and refuses a missing option before it writes anything.
    async fn check_status_write(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        target: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        let status = self.resolved_target(kind, category)?;
        if status.option().is_none() {
            return Ok(());
        }
        let fields = match target {
            Some(target) => {
                // A target this board does not hold is the write's own refusal to make.
                let Some(item) = self.bound_item(target).await? else {
                    return Ok(());
                };
                self.resolved_cache()?.insert(target.clone(), item.clone());
                self.fields_for(Some(&item), true, false).await?.fields
            }
            None => {
                let held = self
                    .board_cache()?
                    .as_ref()
                    .map(|board| board.fields.clone());
                match held.or_else(|| {
                    self.fields_cache()
                        .ok()
                        .and_then(|cache| cache.as_ref().map(|board| board.fields.clone()))
                }) {
                    Some(fields) => fields,
                    None => return Ok(()),
                }
            }
        };
        self.column_for(&fields, kind, category, &status)
            .map(|_| ())
    }

    /// Set one task's status alone.
    ///
    /// An open target reopens a closed issue with an `updateIssue` carrying only its
    /// `stateInput`, then selects the board option with `updateProjectV2ItemFieldValue`; a
    /// terminal target selects its mapped option, then closes with its fixed reason. No
    /// request carries a title, a body or a label. The status
    /// answered is what [`BoardStatuses::status`] reads off the state just written, which is
    /// what a re-read reports.
    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        self.set_status(id, category).await
    }

    /// Set one task's priority alone: one `updateProjectV2ItemFieldValue` selecting the
    /// mapped option of the board's `Priority` field, or one `clearProjectV2ItemFieldValue`
    /// for `none`. Refused by an instance with no `priority_mapping`.
    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        self.set_priority(id, priority).await
    }

    /// Replace one task's content with a single body update that keeps the metadata slot
    /// byte for byte.
    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        self.replace_content(id, content).await
    }

    /// Replace one task issue's content and its provenance slot entry with a single body
    /// update. The answers are not kept: see `replace_rendering`.
    async fn set_task_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        _answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.replace_rendering(id, BoardKind::Work(ItemKind::Task), content, provenance)
            .await
    }

    /// Replace one design-document issue's content and its provenance slot entry, on exactly
    /// the terms of [`set_task_rendering`](TaskSource::set_task_rendering).
    async fn set_document_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        _answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.replace_rendering(id, BoardKind::Document, content, provenance)
            .await
    }

    /// Apply a targeted update with one read of the item and a write only for what differs:
    /// the `Status` and `Priority` field writes in one request, the `blockedBy` difference,
    /// and last one `updateIssue` for title, body and state. See `targeted_update`.
    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.targeted_update(id, update).await
    }

    /// Replace one task's `delivered_by` with a single body update that changes the
    /// metadata slot and nothing outside it.
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.replace_delivered_by(id, delivered_by).await
    }

    /// Set one key of one task issue's metadata with a single body update that changes the
    /// metadata slot and nothing outside it — no title, label, state or board field request —
    /// and sends nothing when the task already holds that value under the key.
    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        Ok(self
            .set_slot_key(id, BoardKind::Work(ItemKind::Task), key, value)
            .await?
            .map(|item| item.task())
            .transpose()?)
    }

    /// Set one key of one project issue's metadata, on exactly the terms of
    /// [`set_task_metadata`](TaskSource::set_task_metadata).
    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        Ok(self
            .set_slot_key(id, BoardKind::Work(ItemKind::Project), key, value)
            .await?
            .map(|item| item.project()))
    }

    /// Set one key of one design-document issue's metadata, on exactly the terms of
    /// [`set_task_metadata`](TaskSource::set_task_metadata).
    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        Ok(self
            .set_slot_key(id, BoardKind::Document, key, value)
            .await?
            .map(|item| item.document()))
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

    /// One page of the task issue's own comments, walked by GitHub's own cursor.
    ///
    /// Nothing here filters, so nothing has to be read ahead of the page: the caller's limit is
    /// the page GitHub is asked for and GitHub's `endCursor` is the cursor handed back.
    ///
    /// One request, [`graphql::ISSUE_DETAIL`]: the read that says the id names a task of this
    /// board is the read of its comments. A draft this process already resolved is refused
    /// without one.
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        validate_page(page)?;
        let cached = self.resolved_cache()?.get(task).cloned();
        if let Some(item) = cached {
            if item.kind != BoardKind::Work(ItemKind::Task) {
                return Ok(None);
            }
            if item.content_kind == ContentKind::DraftIssue {
                return Err(self.draft_has_no_comments(task));
            }
        }
        match self.issue_detail(task, page).await? {
            Some(TaskDetailRead {
                comments: Some(comments),
                ..
            }) => comments,
            _ => Ok(None),
        }
    }

    /// Every id's task, with the first page of its comments when `comments` names it:
    /// [`DETAIL_BATCH`] items per [`graphql::ISSUE_DETAILS`] request, and one item with its
    /// comments in one [`graphql::ISSUE_DETAIL`] request.
    async fn get_task_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        if let Some(page) = comments
            && let Err(error) = validate_page(page)
        {
            return ids.iter().map(|_| Err(error.clone())).collect();
        }
        match (ids, comments) {
            ([id], Some(page)) => vec![self.issue_detail(id, page).await],
            ([id], None) => vec![self.task_read(id).await],
            _ => self.issue_details(ids, comments).await,
        }
    }

    /// Add one comment to the task's issue, as the account the token belongs to.
    ///
    /// The author is refused before anything is sent — not even the task is read — because
    /// no answer GitHub could give would make posting under another name than the one asked
    /// for the right outcome.
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        if let Some(author) = &comment.author {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot post a comment as {author:?}: GitHub records the account \
                     the token signs in as the author of every comment; next: leave --author \
                     out, and the comment is posted as that account",
                    self.name
                ),
            });
        }
        let Some(issue) = self.commented_issue(task).await? else {
            return Ok(None);
        };
        let data = self
            .graphql(
                graphql::ADD_COMMENT,
                json!({"input":{"subjectId":issue.0,"body":comment.body.as_str()}}),
            )
            .await?;
        let subject = data
            .pointer("/addComment/subject")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment addition returned no subject".into(),
            })?;
        if required_str(subject, "id")? != issue.0 {
            return Err(SourceError::Malformed {
                message: "GitHub comment addition answered about another issue".into(),
            });
        }
        let added = data
            .pointer("/addComment/commentEdge/node")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment addition returned no comment".into(),
            })?;
        comment_from(added).map(Some)
    }

    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        let Some(issue) = self.commented_issue(task).await? else {
            return Ok(None);
        };
        if !self.comment_is_on(&issue, comment).await? {
            return Ok(None);
        }
        let data = self
            .graphql(
                graphql::UPDATE_COMMENT,
                json!({"input":{"id":comment.0,"body":body.as_str()}}),
            )
            .await?;
        let edited = data
            .pointer("/updateIssueComment/issueComment")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment update returned no comment".into(),
            })?;
        let edited = comment_from(edited)?;
        if edited.id != *comment {
            return Err(SourceError::Malformed {
                message: "GitHub comment update returned the wrong comment".into(),
            });
        }
        Ok(Some(edited))
    }

    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        let Some(issue) = self.commented_issue(task).await? else {
            return Ok(None);
        };
        if !self.comment_is_on(&issue, comment).await? {
            return Ok(None);
        }
        let data = self
            .graphql(graphql::DELETE_COMMENT, json!({"input":{"id":comment.0}}))
            .await?;
        // The payload says nothing about the comment it removed, so what is checked is that
        // GitHub answered the mutation at all rather than leaving it unanswered.
        data.get("deleteIssueComment")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment deletion returned no payload".into(),
            })?;
        Ok(Some(comment.clone()))
    }

    /// Every request this source has recorded, and what each of GitHub's two budgets was
    /// attributed — read off the same accounting the session report is rendered from, so
    /// the two cannot count one request two ways.
    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        Ok(Some(self.ledger.snapshot().metering()))
    }

    /// Drop every item, search answer and board read this source holds, so the next command
    /// reads the board as a person has since left it.
    ///
    /// Every one of those is held on the assumption that nothing but this source writes the
    /// board while a command runs, which stops being true the moment the command is over: a
    /// body a person edited would be overwritten from the record held here, and a card they
    /// moved would be read as still where this source left it. The board's own field
    /// definitions go too, because a person can add or delete a `Status` option and a write
    /// resolved against the held list would not re-read on a miss. What stays is what stays
    /// valid in normal use: each repository's node id, which a miss re-reads, the pacing of
    /// mutations, which is about GitHub's limiter rather than anybody's work, and the running
    /// accounting [`metering`](TaskSource::metering) answers from.
    ///
    /// Infallible in practice: a lock an earlier failure poisoned is cleared rather than
    /// refused, because clearing it is what puts it right.
    async fn end_command(&self) -> Result<(), SourceError> {
        fn clear<T: Default>(held: &Mutex<T>) {
            *held
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = T::default();
            held.clear_poison();
        }
        clear(&self.created);
        clear(&self.updated);
        clear(&self.board_cache);
        clear(&self.search_cache);
        clear(&self.narrowed_cache);
        clear(&self.search_next);
        clear(&self.resolved_cache);
        clear(&self.fields_cache);
        Ok(())
    }
}

/// One issue comment as the contract carries it.
///
/// `author` is absent both when GitHub answers `null` for an account that no longer exists
/// and when it answers an actor with no login, because either way the source did not say who
/// wrote it — which is what an absent author means, rather than an author called nothing.
fn comment_from(value: &Value) -> Result<Comment, SourceError> {
    Ok(Comment {
        id: NativeId(required_str(value, "id")?.to_owned()),
        author: optional_str(value.get("author").unwrap_or(&Value::Null), "login")?
            .map(str::to_owned),
        created_at: optional_time(value, "createdAt")?,
        updated_at: optional_time(value, "updatedAt")?,
        body: required_str(value, "body")?.to_owned(),
        url: optional_str(value, "url")?.map(str::to_owned),
    })
}

/// The page of comments one issue node carries, resumed from `after`.
fn comment_page(
    node: &Value,
    issue: &str,
    after: Option<&str>,
) -> Result<Page<Comment>, SourceError> {
    let connection = node
        .get("comments")
        .filter(|value| !value.is_null())
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub issue {issue} answered with no comments connection"),
        })?;
    let items = optional_nodes(Some(connection), "issue comments")?
        .into_iter()
        .flatten()
        .map(comment_from)
        .collect::<Result<Vec<_>, _>>()?;
    let next = next_cursor(connection)?;
    if let Some(next) = &next {
        validate_cursor_progress(after, &next.0)?;
    }
    Ok(Page { items, next })
}

/// The far ends of an issue's whole `blockedBy` connection, when the read carried it to its
/// end — `None` when it carried none, or a page with more past it.
fn carried_blocked_by(content: &Value) -> Result<Option<Vec<Value>>, SourceError> {
    let Some(connection) = content.get("blockedBy").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if next_cursor(connection)?.is_some() {
        return Ok(None);
    }
    Ok(Some(
        optional_nodes(Some(connection), "blocked-by issues")?
            .into_iter()
            .flatten()
            .cloned()
            .collect(),
    ))
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
        Some(StatusTarget::Terminal(_, reason)) => {
            json!({"value":"CLOSED","stateReason":reason.reason()})
        }
        Some(StatusTarget::Column(_) | StatusTarget::Disabled(_)) => json!({"value":"OPEN"}),
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
///
/// The copy origin is the one typed field that is also mirrored here, and only as a
/// mirror: it lands in the board's origin field as well, which stays the one every reader
/// takes it from, and it is here so that GitHub's issue search — which indexes this comment
/// and catches up with a write in seconds rather than minutes — can find the item by it.
/// A reader of the release before this one drops the slot's copy and reads the field, so an
/// item written here still reads with exactly one origin there.
fn slot_metadata(
    incoming: &Incoming<'_>,
    own_repository: Option<&Repository>,
    fallback: &[DependencyEdge],
) -> BTreeMap<String, Value> {
    let mut metadata = incoming.metadata.clone();
    match metadata.remove(ORIGIN_KEY) {
        Some(Value::String(origin)) if !origin.is_empty() => {
            metadata.insert(ORIGIN_KEY.to_owned(), Value::String(origin));
        }
        _ => {}
    }
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
    // The typed lists are what land, whatever the caller's own metadata held under their
    // keys: a key of either name travelling beside the field would otherwise be a second
    // answer to the same question, and the field is the one the contract names.
    for (key, entries) in [
        (TaskRef::DELIVERS_KEY, incoming.delivers),
        (TaskRef::DELIVERED_BY_KEY, incoming.delivered_by),
    ] {
        set_task_list(&mut metadata, key, entries);
    }
    record_edges(&mut metadata, fallback);
    metadata
}

/// Hold the far ends no relationship here can name under [`DependencyEdge::RECORDED_KEY`] in
/// one slot's metadata, or no such key when there are none.
fn record_edges(metadata: &mut BTreeMap<String, Value>, fallback: &[DependencyEdge]) {
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
}

/// Every label one item carries, from its content's own connection and nowhere else.
///
/// There is no second place to read one from: no document this source sends selects the
/// board's built-in `Labels` field, because GitHub derives it from the content and a draft
/// cannot carry one at all. The module documentation records the three schema facts that
/// settle it.
fn labels(content: &Value) -> Result<Vec<Label>, SourceError> {
    optional_nodes(content.get("labels"), "content labels")?
        .into_iter()
        .flatten()
        .map(|v| {
            Ok(Label {
                id: NativeId(required_str(v, "id")?.to_owned()),
                name: required_str(v, "name")?.to_owned(),
                color: optional_str(v, "color")?.map(str::to_owned),
            })
        })
        .collect()
}

/// The definition of each board field one item's values are values of, in the shape a read
/// of the board's own `fields` gives one.
///
/// A value names its field through a fragment on that field's own type, so the type is
/// known from which kind of value it is: a single-select value's field is a
/// `ProjectV2SingleSelectField`, options and all, and a text value's is a `ProjectV2Field`.
/// A value whose field carried no id, or an empty one, says nothing usable and is left out.
fn field_definitions(field_values: &[Value]) -> Vec<Value> {
    field_values
        .iter()
        .filter_map(|value| {
            let field = value.get("field")?.as_object()?;
            field.get("id")?.as_str().filter(|id| !id.is_empty())?;
            let typename = if value.get("text").is_some() {
                "ProjectV2Field"
            } else if value.get("name").is_some() {
                "ProjectV2SingleSelectField"
            } else {
                return None;
            };
            let mut defined = field.clone();
            defined.insert("__typename".to_owned(), json!(typename));
            Some(Value::Object(defined))
        })
        .collect()
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

/// One issue's own `number`.
///
/// An issue always has one: GitHub declares `Issue.number` as `Int!` and every selection of
/// an issue in this module asks for it. So a read of one that comes back without it, or
/// with something that is not an unsigned integer, is a response this source cannot read —
/// absence here is **not** "this issue has no number". A draft is the content that has
/// none, and a draft never reaches this: the caller decides on `__typename` first, the way
/// it does for `subIssuesSummary`, which `DraftIssue` equally declares nothing for.
fn issue_number(issue: &Value) -> Result<u64, SourceError> {
    issue
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue number is missing or is not an unsigned integer".into(),
        })
}

/// The `number` a creating mutation answered with, and `None` when it answered without one;
/// why a missing one is tolerated is at the call in `create_and_file_issue`.
fn created_issue_number(created: &Value) -> Result<Option<u64>, SourceError> {
    match created.get("number") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub created issue number is not an unsigned integer".into(),
            }),
    }
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing string field {field}"),
        })
}

fn required_nonblank_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    let found = required_str(value, field)?;
    if found.trim().is_empty() {
        return Err(SourceError::Malformed {
            message: format!("GitHub response has blank string field {field}"),
        });
    }
    Ok(found)
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

/// What the composer puts between a non-empty visible body and the slot, and the one thing
/// the parser takes off the visible body when it takes the slot off — exactly once, so every
/// other trailing byte of the body comes back as it was written.
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] How a composer lays the slot after prose is this source's own; `docs/metadata.md` and its gate settle only the delimiters, and no other source declares a separator to reconcile against.
const METADATA_SEPARATOR: &str = "\n\n";

/// The visible body and the metadata slot at the end of it.
///
/// The encoding is the one `docs/metadata.md` settles for Linear, which is where its
/// reasons are. Only a comment at the very end is a slot; one in the middle is a person's
/// own content and is left alone. The visible body is everything before the slot less the
/// one [`METADATA_SEPARATOR`] the composer put there, byte for byte.
fn metadata_body(
    body: Option<String>,
) -> Result<(Option<String>, BTreeMap<String, Value>), SourceError> {
    let Some(body) = body else {
        return Ok((None, BTreeMap::new()));
    };
    let Some(slot) = slot_span(&body)? else {
        return Ok((Some(body), BTreeMap::new()));
    };
    let metadata =
        serde_json::from_str(&body[slot.encoded_start..slot.encoded_end]).map_err(|error| {
            SourceError::Malformed {
                message: format!(
                    "invalid canonical JSON in GitHub issue onetaskgraph metadata slot: {error}"
                ),
            }
        })?;
    let before = &body[..slot.start];
    let visible = before.strip_suffix(METADATA_SEPARATOR).unwrap_or(before);
    Ok(((!visible.is_empty()).then(|| visible.to_owned()), metadata))
}

/// Where the metadata slot sits in one body, as byte offsets into it.
struct SlotSpan {
    /// Where [`METADATA_OPEN`] begins.
    start: usize,
    /// Where the encoded JSON begins, just past [`METADATA_OPEN`].
    encoded_start: usize,
    /// Where the encoded JSON ends, at the start of [`METADATA_CLOSE`].
    encoded_end: usize,
    /// Just past [`METADATA_CLOSE`].
    end: usize,
}

/// The slot at the very end of `body`, or `None` when it has none.
///
/// The one reading of *where the slot is*, shared by [`metadata_body`], which reads it, and
/// [`with_slot`], which rewrites it — so the two cannot disagree about which comment is the
/// slot.
fn slot_span(body: &str) -> Result<Option<SlotSpan>, SourceError> {
    let Some(start) = body.rfind(METADATA_OPEN) else {
        return Ok(None);
    };
    let encoded_start = start + METADATA_OPEN.len();
    let Some(relative_end) = body[encoded_start..].find(METADATA_CLOSE) else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in GitHub issue body".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    let end = encoded_end + METADATA_CLOSE.len();
    if !body[end..].trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(SlotSpan {
        start,
        encoded_start,
        encoded_end,
        end,
    }))
}

/// `body` with its metadata slot holding exactly `metadata`, and every byte outside the
/// slot as it was.
///
/// A slot that is there has its JSON replaced in place; one that becomes empty is removed
/// together with the one [`METADATA_SEPARATOR`] separating it from the prose before it. A
/// body with no slot gains one the way [`compose_body`] writes it — after that separator,
/// or alone in an empty body — and a body with no slot that is given no metadata is
/// returned as it is.
fn with_slot(body: &str, metadata: &BTreeMap<String, Value>) -> Result<String, SourceError> {
    let encoded = if metadata.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(metadata).map_err(|error| SourceError::Malformed {
                message: error.to_string(),
            })?,
        )
    };
    Ok(match (slot_span(body)?, encoded) {
        (Some(slot), Some(encoded)) => format!(
            "{}{encoded}{}",
            &body[..slot.encoded_start],
            &body[slot.encoded_end..]
        ),
        (Some(slot), None) => {
            let before = &body[..slot.start];
            format!(
                "{}{}",
                before.strip_suffix(METADATA_SEPARATOR).unwrap_or(before),
                &body[slot.end..]
            )
        }
        (None, None) => body.to_owned(),
        (None, Some(encoded)) if body.is_empty() => {
            format!("{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
        }
        (None, Some(encoded)) => {
            format!("{body}{METADATA_SEPARATOR}{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
        }
    })
}

/// `body` with everything before its metadata slot replaced by `content`, and the slot
/// itself kept byte for byte.
///
/// The inverse of how [`metadata_body`] splits a body: the slot, when there is one, follows
/// `content` after the one [`METADATA_SEPARATOR`] the composer puts there — or alone, when
/// `content` is empty — so a read of the result reports `content` as the visible body and
/// the slot's metadata exactly as it was.
fn with_content(body: &str, content: &str) -> Result<String, SourceError> {
    let Some(slot) = slot_span(body)? else {
        return Ok(content.to_owned());
    };
    let kept = &body[slot.start..];
    Ok(if content.is_empty() {
        kept.to_owned()
    } else {
        format!("{content}{METADATA_SEPARATOR}{kept}")
    })
}

/// Hold `entries` under `key` in one slot's metadata, or no such key when there are none.
fn set_task_list(metadata: &mut BTreeMap<String, Value>, key: &str, entries: &[TaskRef]) {
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
        format!("{visible}{METADATA_SEPARATOR}{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
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
/// The version of this plugin's opaque narrowing-search cursor.
pub const SEARCH_CURSOR_VERSION: u32 = 4;

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum SearchConnection {
    Initial {},
    Continuing { after: Cursor },
    Exhausted {},
}
impl SearchConnection {
    fn after(&self) -> Option<&str> {
        match self {
            Self::Continuing { after } => Some(&after.0),
            _ => None,
        }
    }
    fn exhausted(&self) -> bool {
        matches!(self, Self::Exhausted { .. })
    }
    /// Whether a cursor naming this position, `offset` rows into its page, is one this
    /// plugin could have handed out: a page is resumed only part of the way through it — an
    /// offset of a whole page or more would skip rows nobody was given — an initial page
    /// only once some of it was handed out, and an exhausted connection has no page to be
    /// part of the way through.
    fn valid_resume(&self, offset: usize) -> bool {
        let within = offset < SEARCH_PAGE_SIZE as usize;
        match self {
            Self::Initial { .. } => offset > 0 && within,
            Self::Continuing { after } => !after.0.is_empty() && within,
            Self::Exhausted { .. } => offset == 0,
        }
    }
}

/// Versioned source cursor. A zero offset and empty own-write ids are omitted.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchPosition {
    version: u32,
    connection: SearchConnection,
    /// How many rows of the page `connection` starts were already handed out.
    #[serde(default, skip_serializing_if = "is_zero")]
    offset: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    seen: Vec<NativeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    own: Vec<NativeId>,
}
impl Default for SearchPosition {
    fn default() -> Self {
        Self {
            version: SEARCH_CURSOR_VERSION,
            connection: SearchConnection::Initial {},
            offset: 0,
            seen: Vec::new(),
            own: Vec::new(),
        }
    }
}

fn is_zero(offset: &usize) -> bool {
    *offset == 0
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
