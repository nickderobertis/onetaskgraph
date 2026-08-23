// llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
//! The credential-gated live lane for this crate.
//!
//! `just test-live` runs this target for every project, uniformly. This is where
//! the `github-projects` plugin's live journeys land when the source does; until then the
//! lane passes with nothing to run.
