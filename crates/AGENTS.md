<!-- llmlint: ignore-file[instruction_layer_localized] this file is the crate-subtree layer,
     and judged whole the rule would split it once more: the binary's exit codes and the
     engine's paging rules into per-crate files under `crates/onetaskgraph/` and
     `crates/onetaskgraph-core/`. That move is the relocation follow-up the root `AGENTS.md`
     records for its own project-scoped sections; until it lands they stay here. -->
# Working in `crates/`

## Adding a plugin

A plugin's factory is registered in `onetaskgraph-core`'s registry even while its source
still refuses. A configuration naming a plugin nobody has implemented yet must get that
plugin's own message, never "unknown plugin".

A plugin that reaches a network is an optional dependency of the engine behind a feature of
its own that no default enables, and the registry names it among the omitted kinds so a
build without it refuses it with that feature's name. Every consumer in this workspace
enables exactly the features it uses — the binary crate enables them all — and a test suite
that configures one declares it in `required-features`.

## Test layout

- Tests live in `tests/`, never in a `#[cfg(test)]` module under `src/`: coverage counts a
  test module's own unreached lines against the crate it is measuring.
- A fixture shared across one crate's tests goes in `tests/common/mod.rs` — the one path
  cargo does not build as a test target of its own.
- An empty `tests/live.rs` is load-bearing and is never deleted, because `cargo test --test
  live` fails on a missing target rather than passing vacuously.

## The command surface the binary owes

The codes themselves are in `cli.rs` and the README, reconciled against `--help` by
`surface.rs`; what does not survive in any of the three is the rule behind them. **`0`
means success and nothing else.** A run that reached no source, or lost one, exits
non-zero unless `--allow-partial` says a partial answer was wanted. Widening `0` to cover
"mostly worked" would make every scripted caller wrong at once and none of them would
notice.

**Ordering across sources is round-robin**, not source-major, and the reason is cost
rather than taste: with source-major order a page filled entirely by the first source
still costs a call to each of the others, which against a hosted source is a request spent
on rows nobody will see.

The engine mints the page token and never interprets a source's cursor. Two things in it
are easy to remove without noticing what they were for. The **count** beside each cursor
is how compensation resumes inside a source page it narrowed, and the alternative is
holding that page's remainder between calls, which is the caching this product does not
do. The **query fingerprint** is what stops a token being answered by a query it was not
minted for — those cursors are offsets into one result set, and against another they
return real rows at exit `0` with nothing to say the answer is arbitrary.
