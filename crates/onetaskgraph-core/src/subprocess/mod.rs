//! Sources that are other processes, on both sides of the pipe.
//!
//! `docs/plugin-protocol.md` is the normative text; this module is its Rust
//! implementation. [`SubprocessSource`] is the engine's half — a [`TaskSource`] that is a
//! spawned program — and [`serve`] is the plugin's half, hosting any plugin of this build
//! behind the same protocol so the two can be exercised against each other.
//!
//! Nothing here compensates for a capability. A subprocess-hosted source declares what it
//! can do in the handshake exactly as a compiled-in one does, and the engine's one
//! compensation layer reads that declaration and does the rest — which is what keeps a
//! source's answers the same whichever side of a pipe it is on.
//!
//! [`TaskSource`]: onetaskgraph_plugin_api::TaskSource

mod connection;
mod plugin;
mod serve;
mod source;
mod wire;

pub use connection::MAX_LINE;
pub use plugin::{Plugin as SubprocessPlugin, Program, SubprocessConfig};
pub use serve::serve;
pub use source::SubprocessSource;
