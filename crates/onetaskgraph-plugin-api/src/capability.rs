//! What a source declares it can do natively, so the engine can compensate for
//! the rest instead of reducing every source to the weakest one's floor.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One source's declared abilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    /// Whether the source has projects at all.
    pub projects: Support,
    /// Whether the source can select tasks belonging to no project.
    pub orphan_tasks: Support,
    /// Whether the source filters by label itself.
    pub filter_by_label: Support,
    /// Whether the source filters by status itself.
    pub filter_by_status: Support,
    /// Whether the source searches titles itself.
    pub search_title: Support,
    /// Whether the source searches bodies itself.
    pub search_content: Support,
    /// How far the source can walk task dependencies.
    pub task_dependencies: DependencySupport,
    /// How far the source can walk project dependencies.
    pub project_dependencies: DependencySupport,
    /// The largest page the source will serve. At least 1 — a source that serves no rows
    /// cannot be paged, and every implementation rejects zero where its config is read.
    // llmlint: ignore[invalid_states_unrepresentable] this field's wire shape is frozen by the plugin contract every source is written against; only the contract's owner may change it, and tightening it is post-build follow-up.
    // llmlint: ignore[boundary_inputs_validated] the boundary that reads a user's configuration does reject zero — `CapabilityConfig::max_page_size` (onetaskgraph-in-memory/src/config.rs) is a `NonZeroU32` and names the setting when it refuses. What stays a plain `u32` is this frozen contract field, which only the contract's owner may narrow — AGENTS.md, "The plugin contract".
    pub max_page_size: u32,
}

/// Whether a source applies one predicate itself.
///
/// Keeps an `Unsupported` variant because in-memory compensation for a filter or
/// a search is sound: the engine over-fetches and narrows. Do not conflate this
/// with [`DependencySupport`], which has no such variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Support {
    /// The source applies this predicate itself.
    Native,
    /// The source ignores this predicate; the engine narrows the wider result.
    Unsupported,
}

impl Support {
    /// Whether the source applies the predicate itself.
    #[must_use]
    pub fn is_native(self) -> bool {
        matches!(self, Self::Native)
    }
}

/// How far a source can walk its own dependency edges.
///
/// There is deliberately **no** unsupported variant: dependency traversal is a
/// guaranteed capability of this product, not one a source may opt out of. A
/// source that cannot report an item's forward edges cannot implement
/// [`TaskSource`](crate::TaskSource). The weakest declaration is
/// [`ForwardOnly`](Self::ForwardOnly), which the engine answers in reverse by a
/// bounded scan and reports as emulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DependencySupport {
    /// The source answers both directions itself.
    BothDirections,
    /// The source answers forward edges; the engine emulates the reverse.
    ForwardOnly,
}

impl DependencySupport {
    /// Whether the source answers the reverse direction itself.
    #[must_use]
    pub fn answers_reverse(self) -> bool {
        matches!(self, Self::BothDirections)
    }
}
