//! What a source's backend really is: who can read what is written there.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{NativeId, Repository};

/// Who can read a destination, as its backend reports it on a live read.
///
/// Only [`Public`](Self::Public) is public. [`Unknown`](Self::Unknown) is the answer of a
/// source that cannot say — a stdio plugin, whose protocol has no such read — and a
/// destination answering it is never treated as private: an item that may only be written
/// somewhere private is refused there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    /// Anybody can read it.
    Public,
    /// Only those its backend admits can read it.
    Private,
    /// The source cannot say.
    Unknown,
}

impl Visibility {
    /// The value as the wire spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The item a write is about to reach, so a source can say who can read where it lands.
///
/// A backend may keep different items in places of different visibility — a GitHub board
/// creates each issue in a repository decided per item, and keeps an existing issue where
/// it is — so [`TaskSource::visibility`](crate::TaskSource::visibility) is asked about the
/// item rather than about the source as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteTarget<'a> {
    /// An item the write creates.
    New {
        /// The repositories it names.
        repositories: &'a [Repository],
        /// The project it is filed under in this source, when it is filed under one.
        project: Option<&'a NativeId>,
    },
    /// An item this source already holds.
    Existing(&'a NativeId),
}
