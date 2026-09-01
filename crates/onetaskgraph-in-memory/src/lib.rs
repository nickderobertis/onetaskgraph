//! A complete [`TaskSource`] held entirely in memory.
//!
//! It exists so the engine has a source whose capability declaration a test can
//! *choose*. Configure two of them over the same dependency graph — one
//! [`DependencySupport::BothDirections`](onetaskgraph_plugin_api::DependencySupport::BothDirections),
//! one
//! [`ForwardOnly`](onetaskgraph_plugin_api::DependencySupport::ForwardOnly) —
//! and the engine's compensation, including its emulated reverse-dependency
//! scan, is exercised against real difference rather than a mock.
//!
//! # What this source declares, field by field
//!
//! **This source's capabilities are configured per source, not fixed by the plugin —
//! `documents` excepted.** Every other field below is a [`CapabilityConfig`] key a document
//! sets, so the verdict for this plugin is about what it *can* apply rather than about one
//! number it always reports. `documents` is fixed at `Unsupported` because this source
//! holds none: a key that could declare it native would let a configuration claim
//! something no code here can serve, which is the one thing the capability rules forbid. Where a configuration declares a field `Native`, this source applies it — it
//! holds every item in memory, so there is no predicate here it could not apply — and
//! where a configuration declares it `Unsupported`, this source ignores it and returns the
//! wider set, which is capability rule 2 and is the whole reason this plugin exists.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven,** and configurable. Note that in the contract this field means *the source has projects at all*: a source declaring it unsupported contributes no project rows and the engine reports the predicate unreachable rather than compensating. |
//! | `documents` | **Unsupported, and unimplemented,** and — alone among these fields — *not* configurable. This source holds no documents at all, so a key that could declare it native would let a configuration claim something no code here can serve. docs/follow-ups.md tracks it. |
//! | `orphan_tasks` | **Supported and proven,** and configurable. |
//! | `filter_by_label` | **Supported and proven,** and configurable. |
//! | `filter_by_status` | **Supported and proven,** and configurable. |
//! | `search_title` | **Supported and proven,** and configurable. |
//! | `search_content` | **Supported and proven,** and configurable. |
//! | `task_dependencies` | **Supported and proven** in both directions, and configurable down to `ForwardOnly`. |
//! | `project_dependencies` | **Supported and proven** in both directions, and configurable down to `ForwardOnly`. |
//! | `max_page_size` | **Supported and proven,** and configurable; a zero is refused where the configuration is read, naming the setting. |
//!
//! ## Ruling: the row that declares nothing native is the fixture, not the defect
//!
//! The shared journey table in `crates/onetaskgraph/tests/e2e/fixtures.rs` carries two
//! rows of this plugin. The second — *"in-memory (compensated: nothing native but its
//! project table, forward-only)"* — declares every field it may as `Unsupported` and both
//! dependency fields as `ForwardOnly`, over exactly the same dataset as the first.
//!
//! **That is deliberate and must stay.** It is the only coverage the engine's compensation
//! path has: it is what proves that a source ignoring a predicate still returns the correct
//! rows, that the plan says the engine applied it, and that the emulated reverse scan
//! answers `DependedOnBy` in the same order a native source does. A worker who reads it as
//! a plugin somebody forgot to finish and helpfully declares it native deletes that path's
//! only coverage while every test still passes.
//!
//! Its ceiling of two rows is part of the same fixture: compensation has to walk more than
//! one source page to find the rows a filter keeps, and a ceiling of two is what makes a
//! journey notice when it stops doing so.
#![deny(missing_docs)]

mod config;
mod filter;
mod source;

pub use config::{CapabilityConfig, InMemoryConfig};
pub use source::{InMemorySource, Plugin};
