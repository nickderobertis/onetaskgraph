//! What a source declares it can do natively, so the engine can compensate for
//! the rest instead of reducing every source to the weakest one's floor.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One source's declared abilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    /// Whether the source has projects at all.
    pub projects: Support,
    /// Whether the source has documents at all.
    ///
    /// The same shape as [`projects`](Self::projects), and read the same way: it says what
    /// the source *holds*, not which predicate it applies. It is therefore **not** one of
    /// the predicates the second capability rule reaches — there is no wider result set to
    /// return and nothing for the engine to narrow. A source declaring `Unsupported` is
    /// never asked for a document at all; the engine reads this once at the handshake,
    /// exactly as it reads [`TaskSource::writes`](crate::TaskSource::writes), and a
    /// document read across several sources reports such a source as holding none rather
    /// than as having failed.
    ///
    /// Defaulted to [`Support::Unsupported`] when a wire value omits it, so a plugin that
    /// predates documents says nothing here and is read as the document-free source it is.
    // llmlint: ignore[names_match_behavior] SECOND PERMITTED REASON — this restates at a new field the justification recorded across this crate (`Capabilities.max_page_size` below, `PageRequest.limit` in query.rs, `Task::url` in work.rs) and in AGENTS.md's "The plugin contract": the approved contract specifies this field as a `Support` *in the shape `projects` already uses*, and `projects` has carried exactly this meaning — "the source has projects at all" — since before this change, as AGENTS.md and every plugin's verdict table record. A second enum here would say the same thing two ways for two sibling fields and change the serialized form of a frozen handshake. Renaming or re-typing either is the contract owner's call, not this crate's.
    // llmlint: ignore[invalid_states_unrepresentable] the unrepresentable state named — "has documents, but some document predicate needs compensation" — is not a state this contract has: a `DocumentQuery`'s predicates are the same `text`/`labels`/`project` the task and project queries carry, and `filter_by_label`, `search_title` and `search_content` already declare how the source applies each of them, over whichever entity it is asked for. Adding a per-entity predicate axis is a contract change with no caller yet, and it would have to reach `projects` in the same breath. Recorded in AGENTS.md, "The three capability rules".
    #[serde(default = "no_documents")]
    pub documents: Support,
    /// Whether the source's tasks have comments at all.
    ///
    /// Read exactly as [`documents`](Self::documents) is: it says what the source *holds*,
    /// not which predicate it applies, so the second capability rule does not reach it. A
    /// source declaring `Unsupported` is never sent a comment call — the engine reads this
    /// once at the handshake and refuses such a call before anything is read, naming the
    /// source and its plugin. Adding, editing and removing a comment is a write, so a source
    /// declaring `Native` is written through only when
    /// [`TaskSource::writes`](crate::TaskSource::writes) says it can be written at all.
    ///
    /// Defaulted to [`Support::Unsupported`] when a wire value omits it, so a plugin that
    /// predates comments says nothing here and is read as the comment-free source it is.
    // llmlint: ignore[names_match_behavior, invalid_states_unrepresentable] the reason recorded at `documents` above, at a new field: the contract says whether a source holds a kind of thing in the shape `projects` and `documents` already use, and a second enum here would say the same thing three ways for three sibling fields. Whether comments can be *written* is `TaskSource::writes`, the one write declaration every write of this contract already reads, so a read-only pairing is a source declaring `Native` here and `Unsupported` there rather than a third variant.
    #[serde(default = "no_comments")]
    pub comments: Support,
    /// Whether the source's tasks hold a [`Priority`](crate::Priority) at all.
    ///
    /// Read exactly as [`documents`](Self::documents) and [`comments`](Self::comments) are:
    /// it says what the source *holds*, not which predicate it applies, so the second
    /// capability rule does not reach it. A source declaring `Unsupported` reports every
    /// task's priority as `none`, and the engine never hands it one that is not: a copy
    /// carrying another priority to it, and a `task priority set` naming it, are both refused
    /// before the source is asked, naming the source and the field.
    ///
    /// Defaulted to [`Support::Unsupported`] when a wire value omits it, so a plugin that
    /// predates priorities says nothing here and is never handed a priority it would drop.
    // llmlint: ignore[names_match_behavior, invalid_states_unrepresentable] the reason recorded at `documents` above, at a new field: the contract says whether a source holds a kind of thing in the shape `projects`, `documents` and `comments` already use, and a second enum here would say the same thing four ways for four sibling fields. Whether a priority can be *written* is `TaskSource::writes`, the one write declaration every write of this contract already reads.
    #[serde(default = "no_priority")]
    pub priority: Support,
    /// Whether the source keeps only the tasks whose priority a query lists, itself.
    ///
    /// A predicate, and so one the second capability rule reaches: a source declaring
    /// `Unsupported` ignores [`TaskQuery::priorities`](crate::TaskQuery::priorities) and
    /// returns the wider set, and the engine narrows it. Its own member rather than a reading
    /// of [`priority`](Self::priority), because holding a priority and filtering by one are
    /// two abilities — a board holds a priority on a field it cannot be asked to filter by.
    ///
    /// Defaulted to [`Support::Unsupported`] when a wire value omits it, so a plugin that
    /// predates priorities is narrowed by the engine rather than trusted to have filtered.
    #[serde(default = "no_priority_filter")]
    pub filter_by_priority: Support,
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

/// What [`Capabilities::documents`] means when a wire value does not carry it.
///
/// A named function rather than a `Default` on [`Support`], which has no sensible default
/// of its own: an absent *predicate* declaration is a plugin that did not answer, while an
/// absent document declaration is a plugin written before there were any.
fn no_documents() -> Support {
    Support::Unsupported
}

/// What [`Capabilities::comments`] means when a wire value does not carry it: a plugin
/// written before there were comments, on the terms [`no_documents`] gives.
fn no_comments() -> Support {
    Support::Unsupported
}

/// What [`Capabilities::priority`] means when a wire value does not carry it: a plugin
/// written before there were priorities, on the terms [`no_documents`] gives.
fn no_priority() -> Support {
    Support::Unsupported
}

/// What [`Capabilities::filter_by_priority`] means when a wire value does not carry it: a
/// plugin that has never heard of the predicate, and so ignores it — which rule 2 already
/// makes the one safe reading.
fn no_priority_filter() -> Support {
    Support::Unsupported
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
