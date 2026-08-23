# Working in `crates/`

## Adding a plugin

1. A new crate here depends on `onetaskgraph-plugin-api` and on nothing else of this
   workspace. `just check` fails in seconds if it reaches further.
2. Register it in `onetaskgraph-core`'s registry so a configuration can name it. Shipping
   the factory before the source is deliberate: a config naming an unimplemented plugin
   should get that plugin's own message, not "unknown plugin".
3. Mirror the crate's Cargo dependencies in its `project.json` `implicitDependencies`.
   Nx cannot read a Cargo manifest, and an unmirrored edge makes the gate under-run.

## Test layout

- `tests/live.rs` exists in **every** crate, empty ones included: `test-live` is a uniform
  target and an empty lane passes with nothing to run. A crate with a real credential puts
  its live journeys there, and they skip — never fail — when the credential is absent, so
  a contributor without keys and a fork pull request behave the same way.
- Unit tests go in `tests/`, not in a `#[cfg(test)]` module under `src/`. Coverage counts
  a test module's own unreached lines against the crate, so an in-`src` suite quietly
  costs the crate the very number it is there to earn.
- A fixture shared by one crate's tests goes in `tests/common/mod.rs`, which cargo does
  not build as a test target of its own.

## Where a new type goes

In `onetaskgraph-core`, unless a trait signature in `onetaskgraph-plugin-api` names it.
Every addition to the api crate re-tests all six crates that depend on it.
