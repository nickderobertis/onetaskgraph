// llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
//! The credential-gated live lane for this crate.
//!
//! `just test-live` runs this target for every project, uniformly. An in-memory
//! source reaches no service, so it has no live tests and the lane passes with
//! nothing to run.
