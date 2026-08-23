//! The one error type every trait method returns.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why a source could not answer.
///
/// Every variant carries owned data only, so an error survives the JSON-over-stdio
/// boundary a subprocess-hosted plugin crosses without losing anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceError {
    /// The source's configuration block is wrong, or names something absent.
    #[error("configuration for this source is invalid: {message}")]
    Config {
        /// What is wrong, in a form a user can act on.
        message: String,
    },
    /// The credential was missing, malformed, or rejected.
    #[error("authentication for this source failed: {message}")]
    Auth {
        /// What failed. Never contains the credential itself.
        message: String,
    },
    /// The source understood the request and declined it.
    #[error("the source refused the request: {message}")]
    Refused {
        /// The source's own reason.
        message: String,
    },
    /// The source asked the caller to slow down.
    #[error("the source rate-limited the request")]
    RateLimited {
        /// How long the source asked us to wait, when it said.
        retry_after_seconds: Option<u64>,
    },
    /// The source could not be reached at all.
    #[error("the source could not be reached: {message}")]
    Unavailable {
        /// What went wrong reaching it.
        message: String,
    },
    /// The source answered with something this interface cannot represent.
    #[error("the source returned data this interface cannot represent: {message}")]
    Malformed {
        /// What could not be represented.
        message: String,
    },
}
