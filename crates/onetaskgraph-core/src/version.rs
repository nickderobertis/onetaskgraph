//! How two of a task's change tokens stand to each other.
//!
//! A [`Task::version`](onetaskgraph_plugin_api::Task::version) is opaque to every caller, and
//! this is the one reader that looks inside one: it dispatches on the prefix each plugin
//! writes, to the ordering that plugin states beside the token it spells, so a token's format
//! and its order have one source each.
//!
//! The answer is about **two versions of one item from one source**: versions of different
//! items, or of different sources, are never compared, and a value no plugin of this build
//! spells is not either.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::GlobalId;

/// How one version of an item stands to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VersionOrder {
    /// Nothing the token covers changed between them.
    Equal,
    /// The first is the later of the two.
    Later,
    /// The first is the earlier of the two.
    Earlier,
    /// The two cannot be put in order: different items, different sources, a value the
    /// plugin cannot parse, or two versions its token says differ without saying which came
    /// first.
    Incomparable,
}

/// One version of one item: the item's qualified id and the token its source gave.
#[derive(Debug, Clone, Copy)]
pub struct Versioned<'a> {
    /// The item.
    pub item: &'a GlobalId,
    /// Its source's token for it.
    pub version: &'a str,
}

/// How `a` stands to `b`.
#[must_use]
pub fn compare(a: Versioned<'_>, b: Versioned<'_>) -> VersionOrder {
    if a.item != b.item {
        return VersionOrder::Incomparable;
    }
    match ordering(a.version, b.version) {
        Some(Ordering::Equal) => VersionOrder::Equal,
        Some(Ordering::Greater) => VersionOrder::Later,
        Some(Ordering::Less) => VersionOrder::Earlier,
        None => VersionOrder::Incomparable,
    }
}

/// The ordering of the plugin whose prefix both tokens carry, or `None`.
fn ordering(a: &str, b: &str) -> Option<Ordering> {
    if a.starts_with(onetaskgraph_local_md::change_token::PREFIX) {
        return onetaskgraph_local_md::change_token::order(a, b);
    }
    #[cfg(feature = "github-projects")]
    if a.starts_with(onetaskgraph_github_projects::change_token::PREFIX) {
        return onetaskgraph_github_projects::change_token::order(a, b);
    }
    None
}
