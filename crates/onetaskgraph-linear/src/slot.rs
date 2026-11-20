//! Where a Linear item's caller metadata lives: one collapsed block at the very end of its long
//! form — an issue's `description`, a project's and a document's `content` — and every spelling
//! this source wrote before it, which reads still accept.
//!
//! Linear has no caller-defined fields, so the metadata travels in the long form itself. This
//! source writes it as one of Linear's collapsible sections, closed for every reader until opened:
//!
//! ```text
//! +++ onetaskgraph metadata
//!
//! `<slot_json>`
//!
//! +++
//! ```
//!
//! set off from the content by one blank line, with nothing after the final `+++`. When the
//! source's `metadata_fields` show a `Fields` section, that section comes first and the block
//! opens with the paragraph `` `onetaskgraph.fields` `` — the record that the section directly
//! before it is the store's — so the field ends `+++ Fields` … `+++`, a blank line, then the block.
//! Spike `spike-linear-collapsed-slot` measured exactly these bytes on an issue, a project and a
//! document of team `TES2`: each stored as sent, read back unchanged, and re-written unchanged.
//!
//! **What this replaced, and why reads still take it.** The metadata was an HTML comment,
//! ``<!-- onetaskgraph.metadata `{…}` -->``, and before that the same comment over several lines.
//! Linear renders an HTML comment as text — a visible paragraph of its literal characters, the
//! JSON among them — so every item showed its machine data under its own text. A `Fields` section
//! was set off by the comment ``<!-- `onetaskgraph.fields` -->`` on the line before it, which Linear
//! shows the same way. Items written then keep every key they hold: [`tail`] recognises all three
//! spellings, a write of such an item writes the block, and `sources migrate` rewrites every one a
//! source still holds ([`StoredForm`] names each).
//!
//! **How the block is told from a caller's own.** [`tail`] takes a block off only when all three
//! hold: it is the last top-level block of the field — set off by a blank line, and outside every
//! fenced code block, closed or left open ([`leaves_fence_open`]); its first line is exactly
//! `+++ onetaskgraph metadata` and its last exactly `+++`; and its body is one line holding a single
//! code span whose text is a JSON object — or the line `` `onetaskgraph.fields` ``, a blank line,
//! and such a line, when the one `+++ Fields` section directly before it is taken off too. So a
//! caller's own expandable block of any summary, this one and `Fields` included, reads back as
//! itself anywhere but last, and a write whose field would end in what a read takes for this block
//! is refused before it is sent — and so is a write of an item with metadata whose content leaves a
//! fence open, which would store the block as that code.
//!
//! **In which form.** The block is composed and recognised in Linear's own `+++` form, never in the
//! authored `<details>` form the engine's markup translates to and from: the transport takes it off
//! an answer before the markup reads the rest, and sets it after the markup writes the rest, so it
//! is never remembered as a caller's literal block and never reaches a caller as content — whether
//! or not a markup is attached.

use std::collections::BTreeMap;
use std::ops::Range;

use onetaskgraph_plugin_api::SourceError;
use serde_json::Value;

/// The block's first line: the summary it is collapsed under, with or without a `Fields` section.
const BLOCK_OPEN: &str = "+++ onetaskgraph metadata";

/// A collapsible section's closing line.
const SECTION_CLOSE: &str = "+++";

/// The paragraph opening a block that a `Fields` section stands directly before: a code span,
/// which Linear stores byte for byte where it would autolink the bare `onetaskgraph.fields`. It
/// is never a key of the metadata, so the JSON stays exactly the caller's.
const FIELDS_SENTINEL: &str = "`onetaskgraph.fields`";

/// The first line of the `Fields` section a block with the sentinel takes off with it.
const FIELDS_OPEN: &str = "+++ Fields";

/// The opening every HTML-comment slot this source wrote shares.
const METADATA_PREFIX: &str = "<!-- onetaskgraph.metadata";
/// The opening of the one-line, code-span comment slot this source wrote before the block.
const METADATA_OPEN_SPAN: &str = "<!-- onetaskgraph.metadata `";
/// The close of that comment slot.
const METADATA_CLOSE_SPAN: &str = "` -->";
/// The opening of the multi-line comment slot this source wrote before the code span. Linear
/// normalized its JSON, so a key or a value Linear rewrote reads back as Linear left it — or, for
/// an array Linear escaped, as a malformed slot naming itself.
const METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const METADATA_CLOSE: &str = "\n-->";
/// The same close as Linear hands that multi-line slot back: it escapes a line opening `-->`, so
/// the slot reads back with a backslash before its close (observed from the real API on
/// 2026-09-14 for a document and on 2026-10-02 for an issue's description too).
const METADATA_CLOSE_ESCAPED: &str = "\n\\-->";

/// The legacy marker a `Fields` section was set off by: an HTML comment holding a code span,
/// alone on the line before the section. Still read; never written.
const FIELDS_MARKER: &str = "<!-- `onetaskgraph.fields` -->";

/// Which spelling a long form's metadata is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredForm {
    /// The collapsed block this source writes now.
    Block,
    /// The one-line comment ``<!-- onetaskgraph.metadata `{…}` -->``.
    HtmlCommentSlot,
    /// The multi-line comment, closed by `-->` or by Linear's escaped `\-->`.
    MultiLineCommentSlot,
    /// A comment slot with a `Fields` section set off by the legacy comment marker before it.
    FieldsCommentMarker,
}

impl StoredForm {
    /// Whether this is the form this source writes now.
    pub(crate) const fn is_current(self) -> bool {
        matches!(self, Self::Block)
    }

    /// The superseded form's name, as `sources migrate` reports the form an item left.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Block => "collapsed-block",
            Self::HtmlCommentSlot => "html-comment-slot",
            Self::MultiLineCommentSlot => "multi-line-comment-slot",
            Self::FieldsCommentMarker => "fields-comment-marker",
        }
    }
}

/// Where the store's tail sits in one long form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tail {
    /// The byte the tail starts at — its `Fields` section's first, when it takes one off, and
    /// otherwise the slot's own. Everything before it is content and its separator.
    pub(crate) start: usize,
    /// The span of the encoded JSON object.
    pub(crate) json: Range<usize>,
    /// The spelling the tail is stored in.
    pub(crate) form: StoredForm,
}

impl Tail {
    /// The metadata the tail holds.
    pub(crate) fn metadata(&self, long_form: &str) -> Result<BTreeMap<String, Value>, SourceError> {
        serde_json::from_str(&long_form[self.json.clone()]).map_err(|error| {
            SourceError::Malformed {
                message: format!(
                    "invalid canonical JSON in Linear onetaskgraph metadata slot: {error}"
                ),
            }
        })
    }

    /// The tail as the read side above the transport holds it: a block exactly as stored with no
    /// `Fields` section and no sentinel, or a comment slot verbatim with no `Fields` section —
    /// so which spelling the item is stored in stays legible to every write that compares.
    pub(crate) fn canonical(&self, long_form: &str) -> String {
        match self.form {
            StoredForm::Block => block_around(&long_form[self.json.start - 1..self.json.end + 1]),
            StoredForm::HtmlCommentSlot
            | StoredForm::MultiLineCommentSlot
            | StoredForm::FieldsCommentMarker => long_form[self.slot_start(long_form)..].to_owned(),
        }
    }

    /// Where a comment slot itself starts, past any legacy `Fields` marker and section.
    fn slot_start(&self, long_form: &str) -> usize {
        long_form[..self.json.start]
            .rfind(METADATA_PREFIX)
            .unwrap_or(self.start)
    }
}

/// One JSON value in the encoding the slot holds it in: compact canonical JSON with `<`, `>` and
/// `` ` `` escaped as `\u003c`, `\u003e` and `\u0060`.
///
/// All three only ever occur inside a JSON string, where the escape means the same character, so
/// the value parses back exactly; and with them escaped no value can close the code span it
/// travels in. A search phrase is not built with this: `slot_phrase` sends only a value none of
/// whose characters any encoder escapes, which this leaves as written.
pub(crate) fn slot_json(value: &impl serde::Serialize) -> Result<String, SourceError> {
    let encoded = serde_json::to_string(value).map_err(|error| SourceError::Malformed {
        message: error.to_string(),
    })?;
    Ok(encoded
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('`', "\\u0060"))
}

/// The block holding exactly `metadata`, with no `Fields` section, as this source writes it.
pub(crate) fn slot_text(metadata: &BTreeMap<String, Value>) -> Result<String, SourceError> {
    Ok(block_around(&format!("`{}`", slot_json(metadata)?)))
}

/// The block holding the code-span paragraph `span`.
fn block_around(span: &str) -> String {
    format!("{BLOCK_OPEN}\n\n{span}\n\n{SECTION_CLOSE}")
}

/// The tail of a long form whose metadata `fields` — the `Fields` section, in Linear's form —
/// shows: the section, a blank line, and the block opening with the sentinel.
pub(crate) fn tail_with_fields(
    fields: &str,
    metadata: &BTreeMap<String, Value>,
) -> Result<String, SourceError> {
    Ok(format!(
        "{fields}\n\n{BLOCK_OPEN}\n\n{FIELDS_SENTINEL}\n\n`{}`\n\n{SECTION_CLOSE}",
        slot_json(metadata)?
    ))
}

/// Where the store's tail is in `long_form`, in whichever spelling it is stored — or `None` when
/// the field ends in none.
///
/// The one place a tail is recognised, so what a read takes off, what a write keeps and what the
/// transport rewrites are the same bytes.
///
/// # Errors
///
/// [`SourceError::Malformed`] for a comment slot that opens and is never closed.
pub(crate) fn tail(long_form: &str) -> Result<Option<Tail>, SourceError> {
    if let Some(tail) = block_tail(long_form) {
        return Ok(Some(tail));
    }
    comment_tail(long_form)
}

/// The block tail of `long_form`, by the three conditions the module states.
fn block_tail(long_form: &str) -> Option<Tail> {
    // Nothing follows the final `+++` but the line breaks a backend may append, which are no
    // block of their own.
    let field = long_form.trim_end_matches(['\n', '\r']);
    let body = field.strip_suffix(SECTION_CLOSE)?;
    let body = body.strip_suffix("\n\n")?;
    // The payload: one line, one code span, a JSON object inside it.
    let line_start = body.rfind('\n').map_or(0, |at| at + 1);
    let line = &body[line_start..];
    let inner = line.strip_prefix('`')?.strip_suffix('`')?;
    if inner.contains('`') || !matches!(serde_json::from_str::<Value>(inner), Ok(Value::Object(_)))
    {
        return None;
    }
    let json = line_start + 1..line_start + 1 + inner.len();
    let before = body[..line_start].strip_suffix("\n\n")?;
    let (opening, sentinel) = match before.strip_suffix(FIELDS_SENTINEL) {
        Some(rest) => (rest.strip_suffix("\n\n")?, true),
        None => (before, false),
    };
    let summary_start = opening.rfind('\n').map_or(0, |at| at + 1);
    if &opening[summary_start..] != BLOCK_OPEN {
        return None;
    }
    // A top-level block: the field's first line, or one a blank line sets apart — and outside
    // every fenced code block, where the lines would be the caller's code rather than a block.
    if summary_start != 0 && !opening[..summary_start].ends_with("\n\n") {
        return None;
    }
    if in_code(long_form, summary_start) {
        return None;
    }
    let start = if sentinel {
        fields_section_before(opening, summary_start).unwrap_or(summary_start)
    } else {
        summary_start
    };
    Some(Tail {
        start,
        json,
        form: StoredForm::Block,
    })
}

/// The byte the one `+++ Fields` section directly before the block at `block_start` starts at —
/// a blank line between them, nested sections counted — or `None` when none stands there.
fn fields_section_before(text: &str, block_start: usize) -> Option<usize> {
    let above = text[..block_start].strip_suffix("\n\n")?;
    let lines: Vec<(usize, &str)> = {
        let mut at = 0;
        above
            .split('\n')
            .map(|line| {
                let held = (at, line);
                at += line.len() + 1;
                held
            })
            .collect()
    };
    if lines.last()?.1 != SECTION_CLOSE {
        return None;
    }
    // A line inside a code block is the caller's code, never a section's opening or close.
    let blocks = code_blocks(above);
    let code: Vec<bool> = lines
        .iter()
        .map(|&(at, _)| blocks.iter().any(|block| block.contains(&at)))
        .collect();
    let mut depth = 0_usize;
    for (&(at, line), code) in lines.iter().zip(code).rev() {
        if code {
            continue;
        }
        if line == SECTION_CLOSE {
            depth += 1;
        } else if line.starts_with("+++ ") {
            depth -= 1;
            if depth == 0 {
                let set_apart = at == 0 || above[..at].ends_with("\n\n");
                return (line == FIELDS_OPEN && set_apart).then_some(at);
            }
        }
    }
    None
}

/// Whether a line written after `text` and a blank line, at the field's left margin — where the
/// store's block is written — would stand inside a code block: a fence `text` leaves open.
///
/// Decided by a CommonMark block parse rather than by lines, so a fence is read in its container:
/// one a list item or a block quote opens and closes is closed, and one such a container leaves
/// open ends with it, since a line at the margin after a blank line ends the container. A fence at
/// the top level left open holds every line after it.
pub(crate) fn leaves_fence_open(text: &str) -> bool {
    let probe = format!("{text}\n\n{BLOCK_OPEN}\n");
    in_code(&probe, text.len() + 2)
}

/// Whether byte `at` of `text` lies inside a code block of its CommonMark block structure.
fn in_code(text: &str, at: usize) -> bool {
    code_blocks(text).iter().any(|block| block.contains(&at))
}

/// The byte ranges of every code block of `text`, fences included, as a CommonMark parse reads
/// them. Linear's editor shows an HTML block as a paragraph of its text, which the parse does not
/// model; a fence is the construct that decides where the store's block stands.
fn code_blocks(text: &str) -> Vec<Range<usize>> {
    use pulldown_cmark::{Event, Parser, Tag};
    Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::CodeBlock(_))).then_some(range)
        })
        .collect()
}

/// The comment-slot tail of `long_form`, in either spelling, with a legacy `Fields` marker and
/// section before it taken too.
fn comment_tail(description: &str) -> Result<Option<Tail>, SourceError> {
    let Some(start) = description.rfind(METADATA_PREFIX) else {
        return Ok(None);
    };
    let rest = &description[start..];
    let (encoded_start, close, form) = if rest.starts_with(METADATA_OPEN_SPAN) {
        let encoded_start = start + METADATA_OPEN_SPAN.len();
        (
            encoded_start,
            description[encoded_start..]
                .rfind(METADATA_CLOSE_SPAN)
                .map(|at| (at, METADATA_CLOSE_SPAN.len())),
            StoredForm::HtmlCommentSlot,
        )
    } else if rest.starts_with(METADATA_OPEN) {
        let encoded_start = start + METADATA_OPEN.len();
        (
            encoded_start,
            [METADATA_CLOSE, METADATA_CLOSE_ESCAPED]
                .into_iter()
                .filter_map(|close| {
                    description[encoded_start..]
                        .find(close)
                        .map(|at| (at, close.len()))
                })
                .min(),
            StoredForm::MultiLineCommentSlot,
        )
    } else {
        return Ok(None);
    };
    let Some((relative_end, close_len)) = close else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in Linear description".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    if !description[encoded_end + close_len..].trim().is_empty() {
        return Ok(None);
    }
    let (start, form) = match fields_marker_at(&description[..start]) {
        Some(at) => (at, StoredForm::FieldsCommentMarker),
        None => (start, form),
    };
    Ok(Some(Tail {
        start,
        json: encoded_start..encoded_end,
        form,
    }))
}

/// Where a legacy `Fields` marker sits in `above` — the text of a long form above its comment
/// slot: the byte it starts at, or `None` when it holds none.
///
/// A marker counts only alone on a line of its own, followed after blank lines by an expandable
/// block in either form, so prose quoting the marker is never taken for one.
fn fields_marker_at(above: &str) -> Option<usize> {
    let at = above.rfind(FIELDS_MARKER)?;
    let own_line = at == 0 || above[..at].ends_with('\n');
    let after = above[at + FIELDS_MARKER.len()..].strip_prefix('\n')?;
    let after = after.trim_start_matches('\n');
    let block = after.starts_with("<details>\n") || after.starts_with("+++ ");
    (own_line && block).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::{StoredForm, Tail, tail};

    const JSON: &str = r#"{"a":1,"b":"x<y"}"#;

    fn block(json: &str) -> String {
        format!("+++ onetaskgraph metadata\n\n`{json}`\n\n+++")
    }

    fn read(long_form: &str) -> (String, Option<StoredForm>) {
        match tail(long_form).expect("recognised") {
            Some(Tail { start, form, .. }) => (long_form[..start].to_owned(), Some(form)),
            None => (long_form.to_owned(), None),
        }
    }

    #[test]
    fn the_block_is_taken_off_only_when_it_is_last_and_holds_one_json_object() {
        let field = format!("Body.\n\n{}", block(JSON));
        assert_eq!(
            read(&field),
            ("Body.\n\n".to_owned(), Some(StoredForm::Block))
        );
        assert_eq!(read(&block(JSON)).0, "");
        // Not last: a paragraph after it.
        let later = format!("{field}\n\nAfter.");
        assert_eq!(read(&later), (later.clone(), None));
        // Another body: two lines, no code span, an array, a second span.
        for body in [
            "`{}`\n`{}`",
            "{\"a\":1}",
            "`[1]`",
            "`{}` and `{}`",
            "text\n\n`{}`",
        ] {
            let other = format!("Body.\n\n+++ onetaskgraph metadata\n\n{body}\n\n+++");
            assert_eq!(read(&other), (other.clone(), None), "{body}");
        }
        // Another summary.
        let other = "Body.\n\n+++ onetaskgraph metadata!\n\n`{}`\n\n+++";
        assert_eq!(read(other), (other.to_owned(), None));
    }

    #[test]
    fn a_block_inside_a_fenced_code_block_is_the_callers_code_and_never_the_slot() {
        // Left open: every line after the fence is code, the block among them.
        for fence in ["```", "```rust", "~~~", "  ````", "~~~ a`b"] {
            let field = format!("Body.\n\n{fence}\nlet x = 1;\n\n{}", block(JSON));
            assert_eq!(read(&field), (field.clone(), None), "{fence}");
            assert!(super::leaves_fence_open(&field), "{fence}");
        }
        // A shorter run, another character, or text after it closes nothing.
        for close in ["``", "~~~", "``` x", "    ```"] {
            let field = format!("```\ncode\n{close}\n\n{}", block(JSON));
            assert_eq!(read(&field), (field.clone(), None), "{close}");
        }
        // Closed, the block after it is the store's again.
        for (open, close) in [
            ("```", "```"),
            ("````", "`````"),
            ("~~~", "~~~  "),
            ("```", "```\r"),
        ] {
            let field = format!("{open}\ncode\n{close}\n\n{}", block(JSON));
            assert_eq!(read(&field).1, Some(StoredForm::Block), "{open} {close}");
            assert!(!super::leaves_fence_open(&field), "{open} {close}");
        }
        // A fence read in its container: one a list item or a block quote closes is closed, and
        // one it leaves open ends with it at the margin; the old line scan refused all of these.
        for content in [
            "- ```\n  code\n  ```\n\nAfter",
            "1. Step:\n\n   ```sh\n   run it\n   ```",
            "> ```\n> quoted\n> ```",
            "- ```\n  never closed",
        ] {
            assert!(!super::leaves_fence_open(content), "{content:?}");
            let field = format!("{content}\n\n{}", block(JSON));
            assert_eq!(read(&field).1, Some(StoredForm::Block), "{content:?}");
        }
        // A backtick run with a backtick after it opens no fence.
        let field = format!("``` a`b\n\n{}", block(JSON));
        assert_eq!(read(&field).1, Some(StoredForm::Block));
        // A `+++` line inside a fence neither opens nor closes the `Fields` section.
        let fields = "+++ Fields\n\n* **Owner**: `nick`\n\n```\n+++\n```\n\n+++";
        let sentinel =
            format!("+++ onetaskgraph metadata\n\n`onetaskgraph.fields`\n\n`{JSON}`\n\n+++");
        let field = format!("Body.\n\n{fields}\n\n{sentinel}");
        assert_eq!(
            read(&field),
            ("Body.\n\n".to_owned(), Some(StoredForm::Block))
        );
    }

    #[test]
    fn a_fields_section_goes_with_the_sentinel_block_and_never_without_it() {
        let fields = "+++ Fields\n\n* **Owner**: `nick`\n\n+++";
        let sentinel =
            format!("+++ onetaskgraph metadata\n\n`onetaskgraph.fields`\n\n`{JSON}`\n\n+++");
        let field = format!("Body.\n\n{fields}\n\n{sentinel}");
        assert_eq!(read(&field).0, "Body.\n\n");
        // A caller's `Fields` section before a block without the sentinel is content.
        let caller = format!("Body.\n\n{fields}\n\n{}", block(JSON));
        assert_eq!(read(&caller).0, format!("Body.\n\n{fields}\n\n"));
        // Only the one section directly before: a caller's earlier one stays.
        let two = format!("Body.\n\n{fields}\n\n{fields}\n\n{sentinel}");
        assert_eq!(read(&two).0, format!("Body.\n\n{fields}\n\n"));
    }

    #[test]
    fn every_legacy_spelling_is_named_by_its_form() {
        for (field, form, above) in [
            (
                "Body.\n\n<!-- onetaskgraph.metadata `{\"a\":1}` -->".to_owned(),
                StoredForm::HtmlCommentSlot,
                "Body.\n\n",
            ),
            (
                "Body.\n\n<!-- onetaskgraph.metadata\n{\"a\":1}\n-->".to_owned(),
                StoredForm::MultiLineCommentSlot,
                "Body.\n\n",
            ),
            (
                "Body.\n\n<!-- onetaskgraph.metadata\n{\"a\":1}\n\\-->".to_owned(),
                StoredForm::MultiLineCommentSlot,
                "Body.\n\n",
            ),
            (
                "Body.\n\n<!-- `onetaskgraph.fields` -->\n\n+++ Fields\n\n- **A**: `1`\n\n+++\n\n\
                 <!-- onetaskgraph.metadata `{\"a\":1}` -->"
                    .to_owned(),
                StoredForm::FieldsCommentMarker,
                "Body.\n\n",
            ),
        ] {
            assert_eq!(read(&field), (above.to_owned(), Some(form)));
            let held = tail(&field).unwrap().unwrap();
            assert_eq!(held.metadata(&field).unwrap()["a"], 1);
            assert!(!held.canonical(&field).contains("Fields"));
        }
    }
}
