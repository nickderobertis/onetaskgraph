//! A complete [`TaskSource`] held entirely in memory.
//!
//! It exists so the engine has a source whose capability declaration a test can
//! *choose*. Configure two of them over the same dependency graph — one
//! [`DependencySupport::BothDirections`](onetaskgraph_plugin_api::DependencySupport::BothDirections),
//! one
//! [`ForwardOnly`](onetaskgraph_plugin_api::DependencySupport::ForwardOnly) —
//! and the engine's compensation, including its emulated reverse-dependency
//! scan, is exercised against real difference rather than a mock.
#![deny(missing_docs)]

mod config;
mod filter;
mod source;

pub use config::{CapabilityConfig, InMemoryConfig};
pub use source::{InMemorySource, Plugin};
