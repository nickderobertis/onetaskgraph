//! The credential-gated live lane for this crate.
//!
//! `just test-live` runs this target for every project, uniformly. The contract
//! crate reaches no service, so it has no live tests and the lane passes with
//! nothing to run — which is exactly the shape the two hosted plugin crates fill
//! in without touching a workflow file.
