//! The credential-gated live lane for this crate.
//!
//! `just test-live` runs this target for every project, uniformly. The binary's
//! live journeys reach a service only through a hosted plugin, so they land with
//! those plugins; until then the lane passes with nothing to run.
