//! What every instance of one Linear tracker in a process shares: the team's vocabulary — its
//! id, its workflow states and the workspace's project statuses — the ids of the labels its
//! writes named, and the ids of the people its assignee writes named.
//!
//! A long-lived caller builds a source instance per command, and several sources over one team
//! are each an instance; each would otherwise read the same vocabulary afresh. Nothing of a
//! user's work is held here — names and the ids Linear gives them — and a write that fails
//! carrying one of them drops what this tracker holds, as an instance's own forget does, so the
//! next write reads afresh rather than sending a stale id again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use onetaskgraph_plugin_api::NativeId;

use super::{Vocabulary, WriteKind};

/// Every tracker's vocabulary, by its tracker — endpoint and API key's workspace — and team.
type Vocabularies = HashMap<(String, String), Arc<Vocabulary>>;
/// Every label id read, by its tracker — endpoint and API key's workspace — which label set it
/// is of, and its name.
type Labels = HashMap<(String, WriteKind, String), NativeId>;

/// Every tracker's vocabulary, by its tracker — endpoint and API key's workspace — and team.
pub(crate) fn vocabularies() -> MutexGuard<'static, Vocabularies> {
    static HELD: OnceLock<Mutex<Vocabularies>> = OnceLock::new();
    HELD.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Every label id read, by its tracker — endpoint and API key's workspace — which label set it
/// is of, and its name.
pub(crate) fn labels() -> MutexGuard<'static, Labels> {
    static HELD: OnceLock<Mutex<Labels>> = OnceLock::new();
    HELD.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Every user id an assignee write resolved, by its tracker — endpoint and API key's workspace —
/// and the display name it was resolved from.
type People = HashMap<(String, String), String>;

/// Every user id an assignee write resolved, by its tracker and the display name it was
/// resolved from.
pub(crate) fn people() -> MutexGuard<'static, People> {
    static HELD: OnceLock<Mutex<People>> = OnceLock::new();
    HELD.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
