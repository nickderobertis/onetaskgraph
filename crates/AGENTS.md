<!-- llmlint: ignore-file[instruction_layer_localized] this file *is* the localized layer
     the rule asks for — the crate-subtree rules live here and the repo-wide ones stay in
     the root `AGENTS.md`. Its other half, `CODEOWNERS` review routing, this repository has
     decided against: it requires zero approvals to merge because nobody reviews these pull
     requests, and GitHub does not request review from a pull request's own author, so a
     `CODEOWNERS` naming its one owner would route nothing to nobody. Adding one would also
     fork the merge-path arrangement away from its source of truth, the create-repo skill's
     `setup_github_governance.py`. If review ever becomes something that happens here,
     delete this directive and add the file. -->
# Working in `crates/`

## Adding a plugin

A plugin's factory is registered in `onetaskgraph-core`'s registry even while its source
still refuses. A configuration naming a plugin nobody has implemented yet must get that
plugin's own message, never "unknown plugin".

## Test layout

- Tests live in `tests/`, never in a `#[cfg(test)]` module under `src/`: coverage counts a
  test module's own unreached lines against the crate it is measuring.
- A fixture shared across one crate's tests goes in `tests/common/mod.rs` — the one path
  cargo does not build as a test target of its own.
- An empty `tests/live.rs` is load-bearing and is never deleted, because `cargo test --test
  live` fails on a missing target rather than passing vacuously.

## The command surface the binary owes

Exit codes are part of the contract, because a caller scripts around them: **`0`** means
success and nothing else; **`1`** the command failed while running; **`2`** the invocation
itself was wrong (clap's own code, so a bad `--set` and an unknown flag agree); **`4`** the
query ran, some sources answered and at least one did not. `--allow-partial` is how a
caller says a partial answer is acceptable, and it turns `4` into `0`. A run that lost a
source never exits `0` unless it was asked to.

**Ordering across sources is round-robin** — one row from each selected source in
configured-name order, then the next from each — and within a source the source's own
order, exactly. Source-major order would be simpler to explain and would cost a call to
every other source on every page filled by the first, which against a hosted source is a
request spent on rows nobody will see. Each stream is walked strictly forwards, so a walk
to exhaustion returns every row exactly once whatever the page size.

The engine mints the page token and never interprets a source's cursor. It carries one of
those cursors per source stream plus a count of rows the engine itself already handed
back — the count is what lets compensation resume inside a source page it narrowed,
without holding the remainder of that page between calls. It is rendered as hex, because a
plugin cursor may contain anything and a token a person pastes has to survive a shell.
