# Linear GraphQL fixture provenance

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear publishes no versioned, offline schema artifact this gate can deterministically pin; the exact production operation documents are shared with the e2e responder through the graphql module, and every response fixture is parsed through the real HTTP plugin tests. -->

`schema.graphql` pins the read-only subset from Linear's published GraphQL schema and
Relay connection shape as documented in Linear's API documentation and schema explorer
on 2026-08-24. One mutation carries a later date: `projectDelete(id: String!):
ProjectArchivePayload!` was re-observed in that same published schema on 2026-08-29 and
pinned then, which is what let `delete_project` stop refusing — a copy into Linear can
now take back a project it created, the way `issueDelete` already let it take back an
issue. The contract test parses every production operation against the pinned
field and argument types and recursively validates each selected response-fixture shape.
`issues.json` covers `Issue`, `WorkflowState`, `IssueLabel`, and `PageInfo`;
`projects.json` covers `Project`, its status, and `ProjectLabel`; `labels.json` covers the
`issueLabels` connection; the two relation fixtures cover documented forward and inverse
issue/project relation connections. They are documentation-derived, not captured from a
user workspace, and contain only invented identifiers and content.
