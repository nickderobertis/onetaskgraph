//! The contract every onetaskgraph source is written against.
//!
//! This crate holds exactly what a plugin author needs in order to implement a
//! source, and nothing else: the two traits, the work types, the query and paging
//! types, the capability declaration, and the error enum. The engine that drives
//! sources lives in `onetaskgraph-core`, and **no plugin crate may depend on it**
//! — see `AGENTS.md` for the two mechanisms that enforce that rather than
//! stating it.
//!
//! Keeping this crate still is the point. Every change here rebuilds and re-tests
//! every plugin, which is exactly the cost the split exists to avoid paying on an
//! ordinary engine change. When a new type could plausibly sit on either side, it
//! belongs in `onetaskgraph-core` unless a trait signature names it.
#![deny(missing_docs)]

mod capability;
mod error;
mod id;
mod query;
mod source;
mod work;

pub use capability::{Capabilities, DependencySupport, Support};
pub use error::SourceError;
pub use id::{NativeId, SOURCE_NAME_PATTERN, SourceName};
pub use query::{
    Cursor, LabelFilter, Page, PageRequest, ProjectFilter, ProjectQuery, TaskQuery, TextFields,
    TextQuery,
};
pub use source::{Health, SecretResolver, SourcePlugin, TaskSource};
pub use work::{
    DependencyEdge, DependencyKind, Direction, Label, Project, Status, StatusCategory, Task,
};
