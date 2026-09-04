# Linear GraphQL fixture provenance

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear publishes no versioned, offline schema artifact this gate can deterministically pin; the exact production operation documents are shared with the e2e responder through the graphql module, and every response fixture is parsed through the real HTTP plugin tests. -->

`schema.graphql` pins the read-only subset from Linear's published GraphQL schema and
Relay connection shape as documented in Linear's API documentation and schema explorer
on 2026-08-24. One mutation carries a later date: `projectDelete(id: String!):
ProjectArchivePayload!` was re-observed in that same published schema on 2026-08-29 and
pinned then, which is what let `delete_project` stop refusing — a copy into Linear can
now take back a project it created, the way `issueDelete` already let it take back an
issue.

The whole document contract carries a third date. `document(id: String!): Document!`,
`documents(first: Int, after: String, filter: DocumentFilter): DocumentConnection!`, the
`Document` type, `DocumentConnection`, `DocumentFilter`, `DocumentCreateInput`,
`DocumentUpdateInput`, `DocumentPayload`, `DocumentArchivePayload` and the three
mutations `documentCreate`, `documentUpdate` and `documentDelete` were re-observed in the
same published schema on **2026-09-01** and pinned then, with the nullability that schema
states rather than the nullability this plugin happens to always send. Two observations
of that day decided how this source reads a document, so they are written down rather
than left to be rediscovered:

- **`Document` has no `labels` field.** The types of Linear's published schema carrying
  `labels` are `Issue`, `Project`, `Team`, `Initiative` and `Organization`, and `Document`
  is not among them. So this source reports a Linear document's labels as none, and
  refuses by name a document write that carries one.
- **`DocumentFilter.project` is a `ProjectFilter`,** where `IssueFilter.project` is a
  `NullableProjectFilter`. Only the nullable one carries `null:`, so Linear cannot be
  asked for the documents belonging to no project, and this source applies that one
  predicate to a fetched page itself.

`ProjectRelationCreateInput` carries a fourth date. Linear added two required fields to it
— `anchorType: String!` and `relatedAnchorType: String!` — and the live journey's project
write began failing with `Field "anchorType" of required type "String!" was not provided.`
on `main` and on the branch alike. Both were re-observed in the published schema on
**2026-09-04** and pinned then. The optional `id`, `projectMilestoneId` and
`relatedProjectMilestoneId` that input also declares stay out of this subset, as every
field this plugin does not send does.

**What the two anchors accept is not in that schema, and the field descriptions mislead.**
Both are bare `String!` enumerating nothing, and Linear validates them as an enum a layer
behind GraphQL where introspection cannot see it. The create input describes each only as
"the type of the anchor" and the `ProjectRelation` output type says each indicates "whether
it is anchored to the project itself or a specific milestone" — which reads as a choice
between a value meaning the project and one meaning a milestone. It is not: the values are
`start`, `end` and `milestone`, the first two being the whole-project anchors and the third
the one that pairs with the milestone ids above. Linear's project-dependency documentation
("Currently we only support a end -> start dependency") and its own `linear-solutions`
import script are where this file first derived them; Linear's validator stated them
outright on **2026-09-04**, refusing `project` in both anchors with `anchorType must be one
of the following values: start, end, milestone`. `milestone` needs an id this source never
sends — `A milestone is required for a dependency with a milestone anchor.` — so the two
whole-project anchors are the whole of what it can send.

**A project relation is typed `dependency`, and that is what the live lane was refused for
next.** With both anchors present the same write was refused again, this time with HTTP 200
and `Argument Validation Error` — the class Linear's input validator raises for a value
outside an accepted set, *after* GraphQL has coerced every field of the input by name,
which is what tells this refusal apart from the missing-field one before it. The same
2026-09-04 probe named the field and the set: `blocks`, `dependsOn`, `related` and
`DEPENDENCY` were each refused with `property: "type"` and `constraints: {"isEnum": "type
must be one of the following values: dependency"}`, and `dependency` was accepted. `blocks`
is the issue vocabulary — `IssueRelationCreateInput` takes `blocks`, `duplicate`, `related`
and `similar` — and a project relation is a different relation whose one member is an
ordering: the timeline dependency Linear's product has, which is why the input carries
anchors at all. So `DependencyKind::Blocks` is written `dependency` at the project level and
`blocks` at the issue level, each level's read accepts only its own word, and a `Related`
project edge is refused by this source before the write, naming both ends and what Linear
does accept. That withdraws nothing at the issue level, where `related` is real. With those
two corrections the live journey got past `write_project`'s relation — the only write in it
that sends `projectRelationCreate` — and failed further on, at the refusals recorded below.

**The anchors carry the direction, and the two id slots do not.** Linear stores whatever
pair it is given and reads a backwards dependency as readily as the right one, so
acceptance settles nothing; what does is Linear's own reading of a stored relation, which
it publishes as the computed `ProjectFilter` members `hasBlockingRelations` ("projects
which are blocking") and `hasBlockedByRelations` ("projects which are blocked"). Three
relations between two scratch projects, written and read back through them on 2026-09-04 —
written down here because reproducing it costs a credential and a workspace:

| `projectId` | `anchorType` | `relatedProjectId` | `relatedAnchorType` | blocked | blocking |
| ----------- | ------------ | ------------------ | ------------------- | ------- | -------- |
| A           | `start`      | B                  | `end`               | A       | B        |
| A           | `end`        | B                  | `start`             | B       | A        |
| B           | `end`        | A                  | `start`             | A       | B        |

Rows one and three exchange the ids and the anchors together and Linear reads them alike;
rows one and two hold the ids still and exchange only the anchors, and the reading flips.
So the project anchored `start` is the one that waits, whichever slot it sits in, and row
one is what this source sends — `near`, the item that depends, in `projectId`. Linear's own
callers put the blocker there instead, so copying their `end`/`start` pair across by
position would state every dependency backwards in the workspace and nothing would refuse
it. Nothing about `relations` against `inverseRelations` had to move: the ids stayed where
they were, so this source still reads back what it writes.

**A filter's `team` is an `ID`, not a `String`, and that is what the same run failed on
next.** The live journey's first task write was refused with HTTP 400 and
`Variable "$team" of type "String!" used in position expecting type "ID".` This file had
carried `WorkflowStateFilter.team` as an invented `IdComparator { id: IdEquality { eq:
String } }`, derived rather than observed, and the plugin's `ISSUE_STATE` declared
`$team: String!` to match. Linear's real shape is `NullableTeamFilter`, whose `id` is an
`IDComparator`, whose `eq` is an `ID`, and both are now pinned from that refusal — the third
place in this file where the real API outranks the documentation it was derived from.
GraphQL admits a variable at a location only when the variable's type is the location's type
or that type's non-null form, so `String!` cannot stand at an `ID` however the value is
spelled, while `ID!` can. The sibling `name` in the very same filter reaches a
`StringComparator.eqIgnoreCase`, which really is a `String`, so `$name: String!` was right
all along: the two variables differ because their locations do, not because their values do.

It reached Linear at all because a variable written inside an inline filter literal is not a
root argument, and the pinned-schema checks compared only root arguments — the one document
in this plugin whose variables sit inside a literal was the one document whose variable
types nothing checked. `pinned_schema_names_every_write_operation_the_plugin_sends` now
walks those literals against the pinned input types, and refuses exactly the pair Linear
refused, naming the variable and the location.

## The 2026-09-04 audit, and why it was not five more round trips

Five contract drifts had been found here one at a time, each by pushing and waiting for
the live lane to be refused: the two anchors `ProjectRelationCreateInput` requires, the
`dependency` a project relation is typed, `$team` declared `String!` at an `ID` location,
and `StringComparator`'s missing `inIgnoreCase`. On **2026-09-04** the rest of what this
source sends was audited in one pass instead, against Linear's real schema and its real
API, and everything below came out of that pass. The instrument is
`every_variables_object_this_source_sends_conforms_to_the_pinned_schema`.

**What the two older checks could not see.** They parse the production documents in
`graphql`, and a filter is in no document: it is built field by field at runtime and handed
over whole as `$filter`. So `IssueFilter` and `ProjectFilter` sat in this file carrying
nothing but `and`/`or`, and nothing but Linear ever read what went into them. Every write
input had the same hole. The new check drives this source's whole surface against a server
that answers everything, records what really went out, and walks each variables object
against the pinned type of the argument it stands at — every key against that input type's
members, every list against its element type, every enum value against its members, every
scalar against its kind. It asserts that it provoked every operation this source can send,
so an unexercised one is a failure rather than a gap. The filters and everything they reach
are pinned member by member for it to read.

- **`ProjectFilter` is not `IssueFilter`, and one builder produced both.** That line is
  byte-identical on `main`, so it long predates the branch this was found on. Linear refused
  the first of its two wrong members outright: `Field "team" is not defined by type
  "ProjectFilter". Did you mean "lead"?` A project has no team; it has the teams it is
  accessible from, so the configured key reaches
  `accessibleTeams:{some:{key:{eqIgnoreCase:…}}}` — not `leadTeam`, which is one designated
  team rather than every team a project is in. The second member was next in line:
  `ProjectFilter.status` is the counterpart of `IssueFilter.state`, while
  `ProjectFilter.state` exists and is a bare `StringComparator` over something else. It is
  pinned here although nothing sends it, because its existence is the trap — with it absent
  the check would report the issue's spelling as a member Linear does not have, which is
  untrue, and with it present the report is what is actually wrong.
- **A project's status vocabulary is not an issue's.** `ProjectStatus.type` is the
  `ProjectStatusType` enum — `backlog`, `planned`, `started`, `paused`, `completed`,
  `canceled` — where a workflow state is `backlog`, `unstarted`, `started`, `completed`,
  `canceled`, `triage`. Read back from the real workspace on 2026-09-04, its statuses carry
  `planned`, which this source had been reporting as an unknown category, and a filter
  spelled `unstarted` would have matched no project while being refused by nothing. So
  `planned` is where `unstarted` is, and `paused` — a project started and neither finished
  nor cancelled — reads as in progress at both ends, because a category reported and not
  askable for is capability rule 1 broken.

**Three drifts of the same pass are not in any schema, and the check cannot reach them.**
Each was found by running the live journey, and what holds each now is named beside it.

- **A connection maximum is not the only bound on a page.** Linear scores every document
  for complexity and refuses one over 10000: `The query is too complex. Complexity: 17475.
  Maximum allowed complexity: 10000.` The `projects` document is charged for its nested
  `labels` connection at Linear's default of 50 per node, so `first: 250` scores 17475 and
  the largest `first` it is accepted at is **143**, exactly; the filter it carries adds
  nothing, and the `issues` document is accepted at 250. `MAX_PAGE_SIZE` is 100 — the
  tighter of the two connections, with 30% of the budget spare, because 143 is a cliff a
  single added field would move.
- **`documentCreate` counts a key that is present, not a value that is set.** It refuses an
  input naming more than one home — `Exactly one of initiativeId, teamId, issueId,
  releaseId, cycleId or projectId must be defined.` — and `{projectId: null, teamId: …}` is
  refused where `{teamId: …}` is accepted. So a document filed under no project carries no
  `projectId` at all. `documentUpdate` keeps its explicit null, and that is the opposite
  rule for the opposite reason: there the null is the instruction, and Linear answered
  `project: null` to exactly that update.
- **None of Linear's three `delete` verbs removes anything.** `issueDelete`,
  `projectDelete` and `documentDelete` each answered `success: true` and the item still read
  back by id, carrying `archivedAt` and `trashed: true`. Its separate archive verb is a
  third state — `archivedAt` set, `trashed` null — and Linear excludes both from every
  connection, so a listing had already stopped returning such an item while a read by id
  still did. The three by-id reads now select `archivedAt` and answer with nothing when it
  is set: it is the marker both states share, so a read by id answers what a listing
  answers, and a delete means what a copy's undo needs it to mean.

**What that pass did not settle.** Complexity, the one-home rule and the trash are Linear
runtime behaviour and appear nowhere in its schema, so no offline check can reach them; the
live journey is what found all three and what guards them. And the pinned filters carry the
members this source sends plus the one recorded absence above — a member Linear adds, or one
it removes that nothing here sends, is invisible until this file is re-observed.

The contract test parses every production operation against the pinned
field and argument types and recursively validates each selected response-fixture shape.
`issues.json` covers `Issue`, `WorkflowState`, `IssueLabel`, and `PageInfo`;
`projects.json` covers `Project`, its status, and `ProjectLabel`; `labels.json` covers the
`issueLabels` connection; the two relation fixtures cover documented forward and inverse
issue/project relation connections; `documents.json` covers the `documents` connection,
`Document` and its `project`. They are documentation-derived, not captured from a
user workspace, and contain only invented identifiers and content.
