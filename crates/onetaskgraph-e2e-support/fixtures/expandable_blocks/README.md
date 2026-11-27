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

## The nested recording

`nested/observations.json` holds what real Linear stored for each record spike
`spike-linear-nested-blocks` wrote to team `TES2` on 2026-11-27 (stamp
`U-17UN402ICR95C-4150265-1795791873`; its branch `nick/design-doc-show-over-tell/spike-linear-nested-blocks`,
`scripts/live-nested-blocks.py`; its report the document
`design-doc-show-over-tell-spike-linear-nested-blocks-report`): a collapsible section `Outer`
holding a paragraph, an image and a section `Before` holding an image, sent in each mix of `+++`
and `>>>`, a control of one section, and the `+++`-in-`+++` form as a team document. The spike left
those records in `TES2`; they were read back from there by their titles, in the shape the spike's
`observations.json` writes — the Markdown sent, the Markdown stored, the encoded editor state and
its decoded tree and outline — the URL of each record left out. `sent` is recomposed by the
spike's own `FORMS` from the image URLs the stored Markdown names, which is how the spike composed
it. `src/linear_normalizations.rs`'s `nested` is its one reader, and the loopback Linear is held to
storing each record's sent Markdown as Linear stored it and to nesting each section and image as
its editor state does.
