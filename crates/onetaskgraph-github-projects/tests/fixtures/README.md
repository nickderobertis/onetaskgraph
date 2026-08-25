# Recorded GraphQL shapes

`project.json` follows GitHub's published `ProjectV2`, `ProjectV2Item`,
`ProjectV2ItemContent`, `ProjectV2ItemFieldSingleSelectValue`, and label-connection schema.
`dependencies.json` follows the published `Issue.blockedBy: IssueConnection` and
`Issue.blocking: IssueConnection` shapes, which provide both dependency directions.

The values are synthetic and stable; the object, union, and connection shapes are recorded from
the official GraphQL references at <https://docs.github.com/en/graphql/reference/projects> and
<https://docs.github.com/en/graphql/reference/issues>. Tests serve these files through an actual
loopback HTTP server and exercise request construction, authentication, parsing, and mapping.

`schema.graphql` is the authoritative read-contract subset obtained from GitHub.com's GraphQL
introspection endpoint on 2026-08-24. The pinned-schema test validates every production operation's
selected fields, arguments, variable types, fragment type conditions, and fixture keys against it;
the credentialed live lane provides the freshness check against GitHub's current schema.
