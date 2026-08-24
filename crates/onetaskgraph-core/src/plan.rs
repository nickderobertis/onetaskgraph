//! What the engine did, carried back with every response.
//!
//! The point of a capability declaration is that two sources answer the same
//! query differently and both answers are correct. These types make that visible
//! instead of leaving a user to guess why one source was fast and another was not:
//! `--explain` renders a [`QueryPlan`] and `--json` carries it as a field.

use onetaskgraph_plugin_api::{SourceError, SourceName};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::engine::StreamState;

/// One page of engine output, with the plan that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QueryResponse<T> {
    /// This page's items, already qualified and merged across sources.
    pub items: Vec<T>,
    /// Where to resume, or `None` when every source is exhausted.
    pub next: Option<PageToken>,
    /// What each source was asked to do, and what the engine did instead.
    pub plan: QueryPlan,
    /// Sources that failed. One failure never fails the whole query.
    pub errors: Vec<SourceFailure>,
}

/// What the engine did, per source.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct QueryPlan {
    /// One entry per source the query reached.
    pub per_source: Vec<SourcePlan>,
}

/// What one source was asked for, and what happened to each predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourcePlan {
    /// The configured source this describes.
    pub source: SourceName,
    /// The plugin kind behind it.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this restates at a new site the justification already recorded at `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs): `kind: String` is approved contract text. It is also the one field that could not be narrowed even if it were free — a plan must carry the kind a subprocess-hosted plugin reports, an open vocabulary no compile-time type can enumerate.
    pub kind: String,
    /// Predicates the source applied itself.
    ///
    /// The four predicate vectors below partition one set of outcomes, and nothing in
    /// the type says so: a `Predicate` could appear in two of them at once, or in none.
    /// One `Vec<(Predicate, Outcome)>` — or a map keyed by predicate — would make that
    /// unrepresentable. See the directive below for why it stays as it is.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this
    // restates at a new site the justification already recorded at
    // `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs):
    // `SourcePlan`'s four-vector shape is approved contract text, reproduced field for
    // field, and `--json` publishes it as the wire format both SDKs are generated from.
    // Collapsing the four vectors into one outcome-tagged collection is a change to that
    // contract, which is the contract owner's call and is expressly forbidden to any node
    // of this plan while other nodes are being written against this text. The finding is
    // correct and is surfaced as a contract defect rather than dismissed: the contract can
    // represent a plan its own rules forbid.
    pub pushed_down: Vec<Predicate>,
    /// Predicates the engine applied in memory over a wider result set.
    pub applied_locally: Vec<Predicate>,
    /// Predicates the engine answered by a bounded scan of the source.
    pub emulated: Vec<Predicate>,
    /// Predicates neither side could answer, so the result is unconstrained.
    ///
    /// Never [`Predicate::ReverseDependencies`]: `DependencySupport` has no
    /// unsupported variant, so a reverse-dependency read is answered natively or
    /// emulated by the engine's bounded scan, never abandoned. The type cannot say
    /// so — see the directive below.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this
    // restates at a new site the justification already recorded at
    // `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs):
    // `unavailable: Vec<Predicate>` and the `Predicate` enum are both approved contract
    // text, so a narrower element type here is the contract owner's call, not this
    // crate's. The finding is correct and is being surfaced as a contract defect rather
    // than dismissed: the contract can express a state its own rules forbid.
    pub unavailable: Vec<Predicate>,
    /// How many pages the engine pulled from this source to answer.
    pub pages_fetched: u32,
}

/// One thing a query can ask of a source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Predicate {
    /// Filter by label name.
    Label,
    /// Filter by status category.
    Status,
    /// Search titles.
    SearchTitle,
    /// Search bodies.
    SearchContent,
    /// Filter by owning project.
    Project,
    /// Walk dependency edges backwards.
    ReverseDependencies,
}

/// One source's failure, kept beside the results the other sources returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceFailure {
    /// The source that failed.
    pub source: SourceName,
    /// Why.
    pub error: SourceError,
}

/// The engine's own resume token: one plugin cursor per source stream, opaque to the
/// caller exactly as a plugin's cursor is opaque to the engine.
///
/// Rendered as lower-case hex, which is not obfuscation — the inside is not a secret —
/// but the one property a token a person copies off a terminal has to have: it survives
/// a shell. The document underneath holds a plugin's own cursor, and a cursor may hold
/// anything at all, so a token spelled as the raw JSON would carry quotes, braces and
/// spaces straight into the next command line. Hex has no character a shell reads.
///
/// The wire shape is a bare JSON string, unchanged. What is not representable is a
/// token this engine never issued: the inner string is private, and both ways in
/// validate — [`encode`](Self::encode) builds one from cursors, and
/// [`parse`](Self::parse), which is also what deserialising one goes through,
/// refuses anything that does not decode. A hand-edited token therefore fails at
/// the boundary it entered rather than wherever it is first decoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct PageToken(String);

impl PageToken {
    /// Encode where every stream still walking picks up.
    ///
    /// Crate-private on purpose: what a token *means* is the engine's, and a caller able
    /// to build one from parts could name a stream no query addressed. A caller with a
    /// token in hand reaches it through [`parse`](Self::parse), which refuses one this
    /// engine did not issue.
    ///
    /// Infallible: a stream state is a source name, a stream kind, an optional cursor
    /// and a count, and none of those can fail to serialise.
    pub(crate) fn encode(streams: &[StreamState]) -> Self {
        let document = serde_json::to_string(streams)
            .expect("a stream state is plain data and always serialises");
        Self(to_hex(&document))
    }

    /// Accept a token from a caller — a `--page-token` argument, or a deserialised
    /// response — refusing one this engine could not have issued.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Malformed`] when `raw` does not decode.
    pub fn parse(raw: impl Into<String>) -> Result<Self, SourceError> {
        let token = Self(raw.into());
        token.streams()?;
        Ok(token)
    }

    /// Borrow the underlying token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where each stream picks up.
    ///
    /// Infallible, and that is a property of the type rather than an assumption: the only
    /// two ways to obtain a `PageToken` are [`encode`](Self::encode), which built this
    /// document, and [`parse`](Self::parse), which refuses anything that does not decode
    /// — and deserialising one goes through `parse`. So a token that does not decode
    /// never exists to be read here.
    pub(crate) fn decode(&self) -> Vec<StreamState> {
        self.streams()
            .expect("every way to build a PageToken validates it")
    }

    /// The states inside, or why this is not one of this engine's tokens.
    fn streams(&self) -> Result<Vec<StreamState>, SourceError> {
        let document = from_hex(&self.0).ok_or_else(|| SourceError::Malformed {
            message: "page token was not issued by this engine: it is not one of this \
                      engine's tokens at all"
                .to_owned(),
        })?;
        serde_json::from_str(&document).map_err(|error| SourceError::Malformed {
            message: format!("page token was not issued by this engine: {error}"),
        })
    }
}

/// Deserialising a token goes through [`PageToken::parse`], so a response carrying
/// a token this engine never issued is refused where it is read.
impl TryFrom<String> for PageToken {
    type Error = SourceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// Serialising is the plain string it has always been; only the way back in changed.
impl From<PageToken> for String {
    fn from(value: PageToken) -> Self {
        value.0
    }
}

impl std::fmt::Display for PageToken {
    /// The opaque string a caller passes back as `--page`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Render `document` as lower-case hex.
fn to_hex(document: &str) -> String {
    let mut rendered = String::with_capacity(document.len() * 2);
    for byte in document.as_bytes() {
        rendered.push(nibble(byte >> 4));
        rendered.push(nibble(byte & 0x0f));
    }
    rendered
}

/// One hex digit.
fn nibble(value: u8) -> char {
    char::from_digit(u32::from(value), 16).expect("a nibble is a hex digit")
}

/// Read hex back, or `None` when `raw` is not hex of valid UTF-8.
fn from_hex(raw: &str) -> Option<String> {
    if !raw.len().is_multiple_of(2) {
        return None;
    }
    let digits: Vec<u8> = raw
        .chars()
        .map(|digit| digit.to_digit(16))
        .collect::<Option<Vec<u32>>>()?
        .into_iter()
        .map(|digit| u8::try_from(digit).expect("a hex digit fits in a byte"))
        .collect();
    let bytes: Vec<u8> = digits
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect();
    String::from_utf8(bytes).ok()
}
