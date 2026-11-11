# The recorded Linear normalizations

What real Linear stored for each Markdown spike `spike-expandable-blocks` wrote through its
GraphQL API to team `TES2` on 2026-11-10, taken unchanged from the spike's branch
(`nick/design-doc-collapsible/spike-expandable-blocks`, commit `f24f1a44`,
`scripts/spikes/expandable-blocks/normalizations.json` and `fixtures/`). The spike's report is the
document `design-doc-collapsible-spike-expandable-blocks-report`.

- `fixtures/<issue|document>-<form>.json`: one record each — the Markdown sent, the Markdown
  Linear stored, the editor state it stored (`stored_state`, base64 Yjs) with its decoded node
  types and tree, and the record it was captured from.
- `normalizations.json`: every line those fixtures show Linear changing, each entry the lines
  sent, the lines stored and the fixture.

`src/linear_normalizations.rs` is the one reader. The loopback Linear's `linear_markdown::stored`
is held to every fixture there; the journeys account for a Linear hop's changes and read the
shapes the authored expandable form does not recognise from here. Their JSON values are the
spike's exactly; only their whitespace is the repository formatter's, which `workspace:format-check`
holds every JSON file to. Do not edit their values: a new observation of Linear is a new capture.
