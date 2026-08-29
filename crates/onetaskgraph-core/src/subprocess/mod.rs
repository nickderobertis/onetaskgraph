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
//! # What this source declares, field by field
//!
//! One verdict per field of `Capabilities`. Every one of them is **supported and proven,
//! and is the hosted source's own**: [`SubprocessSource`] reports the value the program
//! behind the pipe sent in its handshake, unchanged, and holds no opinion of its own about
//! any of them. Nothing here could be unsupported, because nothing here decides.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `orphan_tasks` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `filter_by_label` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `filter_by_status` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `search_title` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `search_content` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `task_dependencies` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `project_dependencies` | **Supported and proven** — the hosted source's own, forwarded. |
//! | `max_page_size` | **Supported and proven** — the hosted source's own, forwarded. |
//!
//! That is proven rather than asserted: the journey table's `subprocess` row hosts the
//! in-memory source over a real pipe and answers every shared journey — every capability
//! field included — with the same rows and the same plan the in-process row does.
//!
//! [`TaskSource`]: onetaskgraph_plugin_api::TaskSource

mod connection;
mod plugin;
mod serve;
mod source;
mod wire;

pub use connection::MAX_LINE;
pub use plugin::{Plugin as SubprocessPlugin, Program, SubprocessConfig};
pub use serve::{serve, serve_plugin};
pub use source::{RequestDeadline, SubprocessSource};
