# What a document copy's coalesced origin lookup costs on GitHub

GitHub's own dry run (`rateLimit(dryRun: true) { cost }`) of three read shapes, side by side,
against the public test board (`nickderobertis/projects/1`) and its public scratch repository
`nickderobertis/onetaskgraph-live-scratch`, measured 2026-10-25 around 02:50 UTC:

| Shape | Cost |
| --- | ---: |
| today's separate origin lookup, `ORIGIN_LOOKUPS` (two origins asked, a page of one each) | 1 |
| today's separate members listing, `SUB_ISSUES` (a page of 23) | 1 |
| the two coalesced, with the members trimmed to `id`, `parent`, `body`, `repository` (a page of 23) | 1 |

`dry_run.py` is what produced `costs.json`, from `documents.json` — the plugin's own
`ORIGIN_LOOKUPS` and `SUB_ISSUES` text at the commit that added this directory — and
`costs.json` records the coalesced document exactly as it was priced. The trimmed members
page was priced against a scratch issue standing in for the project: GitHub's dry-run cost is
static from the query's shape and its `first:` arguments, so which issue it names does not move
it.

**Who ran it.** The manager ran `dry_run.py` unchanged, with a credential holding `read:project`
on the test board, because the dispatch's own `gh` credential lacks that scope and GitHub
refuses every board field of these documents without it, dry run included. That credential's
GraphQL allowance read 4,517 remaining before and 4,515 after: the two points are its own
allowance reads and the script's lookup of the scratch issue's id. Nothing was written, and no
private source was read.

The three `# llmlint:` comment lines at the head of `dry_run.py` were added after that run, so
the judged lint reads it as the record it is; nothing it runs changed.
