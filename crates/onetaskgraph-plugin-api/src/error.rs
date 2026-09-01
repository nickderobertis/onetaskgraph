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
    ///
    /// A rate limit is the one refusal whose *reason* an operator cannot guess from the
    /// kind alone. A hosted service typically has more than one limiter, only some of
    /// them are reported by the endpoint an operator would go and check, and the right
    /// next step differs between them — so a source that knows which one refused it, and
    /// what it was doing when it did, says so in [`message`](Self::RateLimited::message)
    /// rather than leaving the operator to infer it and infer it wrong.
    #[error("the source rate-limited the request{}", rate_limit_detail(.message))]
    RateLimited {
        /// How long the source asked us to wait, when it said.
        retry_after_seconds: Option<u64>,
        /// What the source can add about *which* limit refused it and what it was doing.
        ///
        /// Absent means the source had nothing to add beyond the kind, which is what
        /// every source said before this member existed; it is omitted from the wire
        /// entirely when absent, so a reader written against the shape without it sees
        /// exactly the shape it was written for. Never contains a credential.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
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

/// The trailing detail a rate limit renders, or nothing when it carried none.
///
/// Split out so that a `RateLimited` with no message renders exactly the sentence it
/// rendered before the member existed, which is what keeps the addition invisible to
/// everything that was reading it.
fn rate_limit_detail(message: &Option<String>) -> String {
    message
        .as_ref()
        .map(|said| format!(": {said}"))
        .unwrap_or_default()
}
