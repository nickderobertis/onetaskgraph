//! What a source says its own requests to its backend have cost.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Everything one source has sent to its backend since it was built, and what that spent
/// against each budget the backend meters it by.
///
/// **A running total, never a figure per call.** What one piece of work cost is the
/// difference between a reading taken before it and one taken after, which is how the
/// engine reports what a copy spent; a source that reset its figures between readings
/// would make that difference meaningless.
///
/// **Source-owned, in an open vocabulary.** Only the source knows what it sent and how its
/// backend meters it, so the budget and unit names are the source's own. The engine adds
/// figures up by name and interprets none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Metering {
    /// How many requests this source has sent to its backend.
    pub requests: u64,
    /// What those requests spent, one entry per budget.
    #[serde(default)]
    pub budgets: Vec<Metered>,
}

/// What one source's requests have spent against one budget, split by where each figure
/// came from.
///
/// Two amounts rather than one beside a flag: a figure the backend reported or a request
/// counted is a measurement, and a figure the source modelled is an estimate, and a caller
/// adding readings up has to keep the two apart to say which a total is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Metered {
    /// The budget, as the backend names it — `graphql`, `rest`.
    pub budget: String,
    /// What the budget is metered in — `points`, `requests`.
    pub unit: String,
    /// How much was spent against it that the backend reported, or that is a count of
    /// requests against a budget metered in requests.
    #[serde(default)]
    pub measured: u64,
    /// How much was spent against it that is this source's own model rather than a
    /// measurement — a lower bound, when the model is a minimum charge per call.
    #[serde(default)]
    pub modelled: u64,
}
