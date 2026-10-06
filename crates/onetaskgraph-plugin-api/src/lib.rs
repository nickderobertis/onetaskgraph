//! The contract every onetaskgraph source is written against.
//!
//! This crate holds exactly what a plugin author needs in order to implement a
//! source, and nothing else: the two traits, the work types, the query and paging
//! types, the capability declaration, and the error enum. The engine that drives
//! sources lives in `onetaskgraph-core`, and **no plugin crate may depend on it**:
//! `deny.toml` permits that crate exactly one wrapper, the binary, failing the
//! required `deny` job, and `scripts/check-plugin-isolation.sh` reads the real
//! `cargo metadata` graph inside `just check`.
//!
//! Keeping this crate still is the point. Every change here rebuilds and re-tests
//! every plugin, which is exactly the cost the split exists to avoid paying on an
//! ordinary engine change. When a new type could plausibly sit on either side, it
//! belongs in `onetaskgraph-core` unless a trait signature names it.
#![deny(missing_docs)]

mod asset;
mod capability;
mod clock;
mod comment;
mod error;
mod id;
mod metadata;
mod metering;
mod query;
mod source;
mod status_mapping;
mod update;
mod work;
mod write;

pub use asset::{
    Asset, AssetContentType, AssetName, AssetPayload, AssetUpload, AssetUploads, AssetWrite,
    AssetsWritten, asset_references, asset_sha256, assetless, body_digest, is_sha256,
    rewrite_asset_references, serve_asset_references,
};
pub use capability::{Capabilities, DependencySupport, Support};
pub use clock::{Clock, SharedClock, system_clock};
pub use comment::{Comment, CommentBody, NewComment, TaskDetailRead, commentless};
pub use error::SourceError;
pub use id::{NativeId, SOURCE_NAME_PATTERN, SourceName};
pub use metadata::{MetadataKey, MetadataRecord, unwritable_metadata};
pub use metering::{Metered, Metering};
pub use query::{
    Cursor, DocumentQuery, LabelFilter, MetadataMatch, Page, PageRequest, ProjectFilter,
    ProjectQuery, TaskQuery, TextFields, TextQuery,
};
pub use source::{Health, SecretResolver, SourcePlugin, TaskSource};
pub use status_mapping::{StatusMapping, StatusName, StatusNames, UnmappedStatus};
pub use update::{TaskUpdate, TaskUpdateOutcome, UpdatedField};
pub use work::{
    DependencyEdge, DependencyEndpoint, DependencyKind, Direction, Document, ItemKind, Label,
    Location, Priority, Project, Repository, Status, StatusCategory, Task, TaskRef,
};
pub use write::{ItemWrite, WriteSupport, documentless, unwritable, unwritable_field};
