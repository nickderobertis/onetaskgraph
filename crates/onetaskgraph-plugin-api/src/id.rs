//! Identifiers a plugin deals in.
//!
//! A plugin only ever sees its own source's opaque [`NativeId`]. Qualifying one
//! into a `<source>:<native>` global id is the engine's job, in
//! `onetaskgraph-core`, so nothing here knows about it.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::SourceError;

/// A source's own opaque identifier for one item.
///
/// Deliberately unvalidated: a native id is whatever the upstream system says it
/// is, colons included. The engine parses a qualified id by splitting on the
/// *first* colon precisely so this stays true.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct NativeId(pub String);

impl NativeId {
    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NativeId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for NativeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for NativeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The pattern every [`SourceName`] matches.
///
/// Underscores are excluded on purpose: `ONETASKGRAPH_SOURCES__<NAME>__...`
/// joins path segments with a double underscore, so a name containing one would
/// make that mapping ambiguous.
pub const SOURCE_NAME_PATTERN: &str = "^[a-z0-9][a-z0-9-]*$";

/// The name a configuration document gives one configured source.
///
/// A plugin learns its own name from
/// [`SourcePlugin::build`](crate::SourcePlugin::build) and nowhere else. It quotes it
/// in an error message, and it compares it against the source segment of a qualified
/// [`DependencyEndpoint`](crate::DependencyEndpoint) — which is how a plugin tells a far
/// end its own backend could have related from one in a system it knows nothing about.
/// Nothing else about a plugin's behaviour may depend on it: a source answers the same
/// way whatever a document chose to call it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceName(String);

impl SourceName {
    /// Validate and wrap a source name.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Config`] when `value` does not match
    /// [`SOURCE_NAME_PATTERN`].
    pub fn new(value: impl Into<String>) -> Result<Self, SourceError> {
        let value = value.into();
        if Self::is_valid(&value) {
            Ok(Self(value))
        } else {
            Err(SourceError::Config {
                message: format!(
                    "source name {value:?} is not usable; names must match {SOURCE_NAME_PATTERN} \
                     (lower-case letters, digits and hyphens, starting with a letter or digit)"
                ),
            })
        }
    }

    /// The same language [`SOURCE_NAME_PATTERN`] describes, hand-rolled so building a
    /// name costs no regex. The two are one rule in two places, so
    /// `source_name_validation_agrees_with_the_pattern_it_publishes` in
    /// `tests/contract.rs` derives a matcher from the constant and fails if they ever
    /// describe different languages. Change both together.
    fn is_valid(value: &str) -> bool {
        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return false;
        }
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SourceName {
    type Error = SourceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SourceName> for String {
    fn from(value: SourceName) -> Self {
        value.0
    }
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for SourceName {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SourceName".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": SOURCE_NAME_PATTERN,
            "description": "The name a configuration document gives one configured source.",
        })
    }
}
