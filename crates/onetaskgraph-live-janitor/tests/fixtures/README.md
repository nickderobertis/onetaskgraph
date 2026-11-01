# GitHub's REST contract, as the janitor uses it

`rest-operations.json` is GitHub's published OpenAPI description reduced to the five REST
operations the janitor sends — the allowance read, the issue and label listings, the label
delete, the single-run read. It records each operation's
method, path, query parameters (with their enumerations where GitHub gives one) and the
fields of its success body, and the file's own `_` names the
`github/rest-api-description` commit and date it was reduced from. It is
documentation-derived; nothing in it was captured from a live response.

`tests/journey.py` holds both halves of the offline journeys to it: every request the real
janitor sends must be an operation it names with parameters it admits, and every body the
loopback stand-in serves on success may carry only fields it names. One journey also
asserts the janitor sends every operation the pin names, so a stale entry fails as well.

The upstream source is immutable commit `734bc9c1030b774eb3fc909cce477aceea21cf77` of
`github/rest-api-description`, file `descriptions/api.github.com/api.github.com.json`, read
on 2026-10-07. Moving the pin is the moment to regenerate the reduction from that source:
retain the five operations above, their query parameters and enumerations, their success
status and body fields. Descend into the allowance resources and listing arrays; a
null leaf names a field without constraining its value. The journeys then say what moved.
GraphQL is held separately, by `tests/schema.rs` against the GitHub Projects plugin's
pinned `schema.graphql`.

## Linear document cleanup

`linear-upstream.graphql.gz` preserves the complete, unmodified official
[`linear/linear` SDK schema](https://github.com/linear/linear/blob/8397645bc0a496f68896b18ec6453dddd5e51927/packages/sdk/src/schema.graphql),
read 2026-10-28, compressed with `gzip -n`. Regenerate by fetching that immutable URL
and piping its bytes through `gzip -n`; do not hand-edit a subset. `tests/schema.rs`
decompresses it offline, validates every operation the janitor sends, and validates every
replayed success field and its nullability against that same upstream schema.
`linear-documents.json` and `linear-projects.json` record a response for offline replay,
with invented ids, team keys, titles and `updatedAt` times covering the removal boundary —
the projects the superseded pre-pool parent beside a pool member's, a local set's, another
team's and a shared one; `linear-document-delete.json` and `linear-project-delete.json` the
two delete answers. They are schema-derived, not captured account data: no Linear credential
or account inventory was available.
`linear_journey.py` splits it into two pages and drives the real scheduled executable
against those responses, records its delete requests and proves protected ids receive none.
Additional near-cutoff timestamps test the 24-hour expiry. Listing, transport, malformed
response and deletion refusals are served over that same HTTP boundary.
