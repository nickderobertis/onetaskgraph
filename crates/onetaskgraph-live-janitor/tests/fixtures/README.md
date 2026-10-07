# GitHub's REST contract, as the janitor uses it

`rest-operations.json` is GitHub's published OpenAPI description reduced to the six REST
operations the janitor sends — the allowance read, the issue and label listings, the label
delete, the `ci.yml` run listing and the single-run read. It records each operation's
method, path, query parameters (with their enumerations where GitHub gives one) and the
fields of its success body, and the file's own `_` names the
`github/rest-api-description` commit and date it was reduced from. It is
documentation-derived; nothing in it was captured from a live response.

`tests/journey.py` holds both halves of the offline journeys to it: every request the real
janitor sends must be an operation it names with parameters it admits, and every body the
loopback stand-in serves on success may carry only fields it names. One journey also
asserts the janitor sends every operation the pin names, so a stale entry fails as well.

To re-observe, download that description at a newer commit and run
`python3 -I reduce_rest_description.py <api.github.com.json> <commit> <date> >
rest-operations.json`, then `just format`; the journeys then say what moved. GraphQL is held separately, by
`tests/schema.rs` against the GitHub Projects plugin's pinned `schema.graphql`.
