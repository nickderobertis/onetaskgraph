//! What a source's backend says is left of its allowance, asked through the engine.

use onetaskgraph_plugin_api::{QuotaReading, SourceName};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::delivery::source_failed;
use super::{Engine, EngineError};

/// What `sources quota` answers: every rate-limit window one source's backend reported, each
/// as it reported it.
///
/// A source with nothing to report — a folder of Markdown, an in-memory source, a stdio
/// plugin whose handshake does not declare `quota` — answers no resources at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceQuota {
    /// The source asked.
    pub source: SourceName,
    /// One reading per window, never merged with another; choosing the least is the caller's.
    pub resources: Vec<QuotaReading>,
}

impl Engine {
    /// What the backend of the source called `source` says is left of each rate-limit window
    /// it meters that source's credential by, asked afresh — a source behind the store's own
    /// cache asks its backend every time.
    ///
    /// The store reports the reading; deciding what it means is the caller's.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::SourceUnavailable`] for one that did not build, and
    /// [`EngineError::SourceFailed`] when the reading itself could not be taken.
    pub async fn quota(&self, source: &SourceName) -> Result<Vec<QuotaReading>, EngineError> {
        let built = self.built(source)?;
        built
            .source()
            .quota()
            .await
            .map_err(|error| source_failed(built, error))
    }
}
