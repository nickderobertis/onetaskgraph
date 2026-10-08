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
status and body fields. Descend into the allowance resources; a
null leaf names a field without constraining its value. The journeys then say what moved.
GraphQL is held separately, by `tests/schema.rs` against the GitHub Projects plugin's
pinned `schema.graphql`.
