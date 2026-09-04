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
