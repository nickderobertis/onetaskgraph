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
behind GraphQL, where introspection cannot see it. The create input describes each only as
"the type of the anchor", and the `ProjectRelation` output type says each indicates
"whether it is anchored to the project itself or a specific milestone" — which reads as a
choice between a value meaning the project and one meaning a milestone. It is not. The
values are `start`, `end` and `milestone`: `start` and `end` are the whole-project
anchors, naming which of a project's own two ends the dependency line touches, and
`milestone` is the one that pairs with the milestone ids above. Four things establish it,
and the first is Linear's own:

- Linear's **project-dependency documentation** states "Currently we only support a end ->
  start dependency", and describes the line as running from the blocking project's end
  date to the blocked project's start date — the vocabulary is `start` and `end`, over
  whole projects, with milestones mentioned nowhere on that page.
- Linear's own **`linear/linear-solutions`** import script sends `anchorType: "end"` and
  `relatedAnchorType: "start"` for a whole-project dependency, passing no milestone id.
- A **live read-back** published by `formtrieb/flotilla`, dated 2026-08-16, records
  `anchorType: "project"` being refused, and the corrected write creating a relation that
  read back as `{"anchorType":"end","relatedAnchorType":"start"}` with `projectMilestone:
  null` — the whole-project anchor, spelled `end`/`start`.
- **`linearis-oss/linearis`** and **`smorinlabs/agent2linear`** both type the field as
  `"start" | "end"`, the first noting the values were observed live and that they describe
  a point on the project rather than project-versus-milestone.

Two published callers do send `project`, and both look derived from the field description
rather than from a call: one of them also lists the relation types as `blocks`, `dependsOn`
and `related`, a set the live read-back above found refused in favour of `dependency`.

**A project relation is typed `dependency`, and that is what the live lane was refused for
next.** With both anchors present the same write was refused again on **2026-09-04**, this
time with `Argument Validation Error` and an HTTP 200 — the class of message Linear's input
validator raises for a value outside an accepted set, *after* GraphQL has coerced every
field of the input by name, which is what tells this refusal apart from the missing-field
one before it. The field it names is `type`. `blocks` is the issue vocabulary:
`IssueRelationCreateInput` takes `blocks`, `duplicate`, `related` and `similar`, and a
project relation is a different relation with a different set. The live read-back recorded
above is the observation that decides it — the caller that listed `blocks`, `dependsOn` and
`related` had all three refused in favour of `dependency`, which is also the only kind of
project relation Linear's own product has: a timeline dependency, which is why the input
carries anchors at all. So `DependencyKind::Blocks` is written as `dependency` at the
project level and `blocks` at the issue level, and each level's read accepts only its own
word.

Two things about that are **not** settled, and both are written down beside the code rather
than left to be rediscovered:

- **Whether Linear also constrains the anchor pair by position.** Nothing offline can
  decide whether the validator requires `anchorType` itself to be `end`, or only that the
  pair describe an end -> start line. This source sends the second reading. If the pair is
  constrained by position, the ids have to swap with it — and `relation_page`'s reading of
  `relations` against `inverseRelations` has to swap too, or the source stops reading back
  what it wrote.
- **Whether a project relation may be typed `related` at all.** `DependencyKind::Related`
  still sends `related` rather than being refused locally: the read-back above is
  second-hand and records what Linear refused on a relation it was asked to *create* as a
  dependency, and withdrawing a capability this source has on that evidence would claim
  more than the evidence carries.

Both are now one live run from being settled, because a Linear refusal carries Linear's own
`extensions` alongside its `message`. The message is a category name — `Argument Validation
Error` named neither the field nor the value — and `extensions` is where the validator says
which property it rejected.

**That live run happened, and the project relation was accepted.** On **2026-09-04** the
required check reached the live journey again with `dependency` and both anchors, and it no
longer failed at a project write: it got past `write_project`'s relation — the only write in
the journey that sends `projectRelationCreate` — and failed two writes later, at the first
`write_task`, for an unrelated reason recorded below. So `dependency` is the type Linear
accepts for a project relation, and `start`/`end` are values it accepts for the two anchors.
What that run does **not** settle is the first bullet above: an accepted anchor pair is not
a correctly *oriented* one, because Linear takes a backwards dependency as readily as the
right one. The orientation still rests on the reasoning under "This source anchors by role,
not by position" below, and on nothing observed.

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

**This source anchors by role, not by position.** Linear's callers put the blocking project
in `projectId`; this source puts the item that depends there, so it sends the mirror —
`start` on the near end, `end` on the far end it waits on. Copying the measured pair across
by position instead would state the dependency backwards, and Linear takes a backwards
dependency as readily as the right one, so that error would land in the workspace rather
than be refused at the write.

The contract test parses every production operation against the pinned
field and argument types and recursively validates each selected response-fixture shape.
`issues.json` covers `Issue`, `WorkflowState`, `IssueLabel`, and `PageInfo`;
`projects.json` covers `Project`, its status, and `ProjectLabel`; `labels.json` covers the
`issueLabels` connection; the two relation fixtures cover documented forward and inverse
issue/project relation connections; `documents.json` covers the `documents` connection,
`Document` and its `project`. They are documentation-derived, not captured from a
user workspace, and contain only invented identifiers and content.
