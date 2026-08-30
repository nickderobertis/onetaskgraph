# Recorded GraphQL shapes

`project.json` follows GitHub's published `ProjectV2`, `ProjectV2Item`,
`ProjectV2ItemContent`, `ProjectV2ItemFieldSingleSelectValue`, `Issue.parent`,
`Issue.subIssuesSummary` and label-connection schema. It is one board holding two projects
— an issue with a sub-issue, and an issue that carries `onetaskgraph.item_kind` as well as
one — a task under each of those projects, a task under neither, and one pull request the
source ignores. Two projects each holding a task of its own is what lets the crate's suite
prove that a task read scoped to one project answers with that project's tasks alone,
which is the whole of what a board holding more than one plan needs; the task under no
parent is what the project-less selection keeps. Their labels, statuses, titles and bodies
differ from one another so that every predicate a query carries separates them.
`dependencies.json` follows the published `Issue.blockedBy: IssueConnection` and
`Issue.blocking: IssueConnection` shapes, which provide both dependency directions, with
each far end carrying the fields that say which kind of item it is.

The values are synthetic and stable; the object, union, and connection shapes are recorded from
the official GraphQL references at <https://docs.github.com/en/graphql/reference/projects> and
<https://docs.github.com/en/graphql/reference/issues>. Tests serve these files through an actual
loopback HTTP server and exercise request construction, authentication, parsing, and mapping.

`schema.graphql` is the authoritative contract subset. The read surface was obtained from
GitHub.com's GraphQL introspection endpoint on 2026-08-24; the sub-issue, state-reason and
write surface — `Issue.parent`, `Issue.subIssues`, `Issue.subIssuesSummary`,
`Issue.stateReason(enableDuplicate:)`, `IssueStateReason`, `IssueClosedStateReason`,
`CreateIssueInput`, `UpdateIssueInput.stateInput`, `IssueStateUpdateInput`,
`AddSubIssueInput`, `RemoveSubIssueInput`, `AddProjectV2ItemByIdInput` and their payloads —
was read from GitHub.com's own published schema artifact
<https://docs.github.com/public/fpt/schema.docs.graphql> on 2026-08-27. The pinned-schema
test validates every production operation's selected fields, arguments, variable types,
fragment type conditions, and fixture keys against it; the credentialed live lane
introspects the current mutation fields and input types as its mutation freshness check,
then exercises reads without changing the configured board.

`deleteIssue`, `DeleteIssueInput` and `DeleteIssuePayload` are here from the same
observation the credentialed live lane's mutation-freshness check reads: `live.rs` pins
`DeleteIssueInput{issueId}` and `DeleteIssuePayload{repository}` and introspects the real
API for them on every credentialed run. The source sends it in one situation only — undoing
a copy that could not finish, over the items that same copy created — and deleting the
issue takes its board item with it, which is why no `deleteProjectV2Item` is here beside it.

Two absences in `schema.graphql` are load-bearing rather than incidental.
`ProjectV2.shortDescription` and `ProjectV2.readme` are not there, and neither is
`updateProjectV2`: a board is a container of projects and this source never writes the
board's own fields, so a document that tried to would fail the pinned-schema test rather
than rename a user's board. `updateProjectV2Field` is absent because its
`singleSelectOptions` is documented as overwriting a field's existing options, so no
addition is additive and a mistake would destroy every item's status.
