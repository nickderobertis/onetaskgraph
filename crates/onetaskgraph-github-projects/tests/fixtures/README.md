# Recorded GraphQL shapes

`project.json` follows GitHub's published `ProjectV2`, `ProjectV2Item`,
`ProjectV2ItemContent`, `ProjectV2ItemFieldSingleSelectValue`, and label-connection schema.
`dependencies.json` follows the published `Issue.blockedBy: IssueConnection` shape. GitHub's
current schema documents `Issue.blocking` as being removed and always empty, which is why the
source declares forward-only dependencies.

The values are synthetic and stable; the object, union, and connection shapes are recorded from
the official GraphQL references at <https://docs.github.com/en/graphql/reference/projects> and
<https://docs.github.com/en/graphql/reference/issues>. Tests serve these files through an actual
loopback HTTP server and exercise request construction, authentication, parsing, and mapping.
