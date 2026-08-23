# Working in `crates/`

## Adding a plugin

Register it in `onetaskgraph-core`'s registry once the crate exists. Shipping the factory
before the source is deliberate: a configuration naming a plugin that has no implementation
yet should get that plugin's own message, not "unknown plugin".

## Test layout

- Unit tests go in `tests/`, not in a `#[cfg(test)]` module under `src/`. Coverage counts a
  test module's own unreached lines against the crate, so an in-`src` suite quietly costs
  the crate the number it is there to earn.
- A fixture shared by one crate's tests goes in `tests/common/mod.rs`, which cargo does not
  build as a test target of its own.
- `tests/live.rs` exists in every crate, empty ones included, because `test-live` is a
  uniform target.
