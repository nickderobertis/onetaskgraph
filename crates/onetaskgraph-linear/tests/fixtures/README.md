# Linear GraphQL fixture provenance

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear publishes no versioned, offline schema artifact this gate can deterministically pin; the exact production operation documents are shared with the e2e responder through the graphql module, and every response fixture is parsed through the real HTTP plugin tests. -->

`schema.graphql` pins the read-only subset from Linear's published GraphQL schema and
Relay connection shape as documented in Linear's API documentation and schema explorer
on 2026-08-24. The contract test parses every production operation against the pinned
field and argument types and recursively validates each selected response-fixture shape.
`issues.json` covers `Issue`, `WorkflowState`, `IssueLabel`, and `PageInfo`;
`projects.json` covers `Project`, its status, and `ProjectLabel`; `labels.json` covers the
`issueLabels` connection; the two relation fixtures cover documented forward and inverse
issue/project relation connections. They are documentation-derived, not captured from a
user workspace, and contain only invented identifiers and content.
