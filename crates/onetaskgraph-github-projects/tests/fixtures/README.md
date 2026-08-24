# Recorded GraphQL shapes

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] These external-contract
fixtures are an explicit acceptance deliverable. Their authoritative sources are linked below;
production validates every required field and the credential-gated live lane detects API drift. -->

`project.json` follows GitHub's published `ProjectV2`, `ProjectV2Item`,
`ProjectV2ItemContent`, `ProjectV2ItemFieldSingleSelectValue`, and label-connection schema.
`dependencies.json` follows the published `Issue.blockedBy: IssueConnection` and
`Issue.blocking: IssueConnection` shapes, which provide both dependency directions.

The values are synthetic and stable; the object, union, and connection shapes are recorded from
the official GraphQL references at <https://docs.github.com/en/graphql/reference/projects> and
<https://docs.github.com/en/graphql/reference/issues>. Tests serve these files through an actual
loopback HTTP server and exercise request construction, authentication, parsing, and mapping.
