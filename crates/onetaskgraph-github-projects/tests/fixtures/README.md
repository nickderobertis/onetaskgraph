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

## `rate-limits.json`

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] This artifact *is* the pinned source, and `the_rate_limit_vocabulary_and_published_limits_match_their_pinned_artifact` is its drift gate; the rule's remaining ask — an automated freshness check against GitHub itself — cannot be a required check here, because GitHub publishes its rate-limit vocabulary and its content-creation ceilings as prose rather than as a versioned artifact, and no required check may reach the network. -->

`rate-limits.json` pins the two things this source restates from GitHub rather than
decides for itself: the wordings a rate-limit refusal arrives in, and the published
ceiling on content-creating requests. Both were read on 2026-09-01 from
<https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api> and
<https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api>,
which is where GitHub documents the secondary limiter, the wordings it refuses with, and
the "no more than 80 content-generating requests per minute and 500 per hour" ceilings.
It is documentation-derived; it carries no captured response and no identifier.

`the_rate_limit_vocabulary_and_published_limits_match_their_pinned_artifact` in
`../schema.rs` reconciles it against `SECONDARY_WORDINGS`, `PRIMARY_WORDINGS`,
`CONTENT_CREATION_PER_MINUTE` and `CONTENT_CREATION_PER_HOUR` **both ways**, so neither a
wording this source quietly starts matching on nor one it quietly stops matching on can
land without moving the pin — and moving the pin means going back to those pages and
recording what they say now. `MIN_MUTATION_INTERVAL_MS` is derived from the per-minute
ceiling rather than written out beside it, so the pacing this source runs at is gated by
the same pin.

GitHub's own wording changes are why this is a list rather than one phrase: `abuse
detection` is what the limiter was called before it was renamed and is still what some
endpoints return, and a refusal carrying it has to keep classifying as a secondary limit.
