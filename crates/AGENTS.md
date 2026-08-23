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
