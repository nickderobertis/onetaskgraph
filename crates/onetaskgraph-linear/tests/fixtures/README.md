# Linear GraphQL fixture provenance

These committed responses follow Linear's published GraphQL schema and Relay connection
shape as documented in Linear's API documentation and schema explorer on 2026-08-24.
`issues.json` covers `Issue`, `WorkflowState`, `IssueLabel`, and `PageInfo`;
`projects.json` covers `Project`, its status, and `ProjectLabel`; `labels.json` covers the
`issueLabels` connection; the two relation fixtures cover documented forward and inverse
issue/project relation connections. They are documentation-derived, not captured from a
user workspace, and contain only invented identifiers and content.
