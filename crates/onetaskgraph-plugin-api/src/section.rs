//! What a source says about the headings of a record it stores, and the markup a body is
//! written and read with where its destination needs some to address a heading.
//!
//! Both exist for a copy's section links — a link to one heading of another record, or of
//! the same one. How each destination spells such a link is decided once, in the engine,
//! and never here: a plugin only reports what its backend stores ([`StoredHeading`]) and
//! applies the markup it is handed ([`BodyMarkup`]), so it holds no copy of any form.

use std::sync::Arc;

/// One heading of a record, as its backend stores it: its text, and the id the backend gave
/// it.
///
/// Reported only by a backend that assigns its headings ids of its own — Linear's editor
/// state gives every heading node an `id` attribute — and in the order the headings appear
/// in the record, so the n-th heading with one text is the n-th entry with that text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredHeading {
    /// The heading's text, without its leading `#` marks or any inline formatting.
    pub text: String,
    /// The id its backend stores the heading under.
    pub id: String,
}

/// Markup a destination's bodies carry for a reader to address their headings, which the
/// engine hands a source when it builds it.
///
/// A source given one applies [`written`](Self::written) to every body it sends its backend
/// and [`read`](Self::read) to every body it reads back, so what the engine reads through
/// the source is the body as it was authored. Defaulted away entirely: a source given none
/// stores what it is given.
pub trait BodyMarkup: Send + Sync {
    /// `content` as the backend is sent it.
    fn written(&self, content: &str) -> String;
    /// `content` read from the backend, with exactly the markup [`written`](Self::written)
    /// adds removed.
    fn read(&self, content: &str) -> String;
}

/// A shared [`BodyMarkup`], as a source holds the one it was handed.
pub type SharedBodyMarkup = Arc<dyn BodyMarkup>;
