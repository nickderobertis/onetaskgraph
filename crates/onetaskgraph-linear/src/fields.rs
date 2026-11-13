//! A source's `metadata_fields`, shown as one collapsed `Fields` section at the end of an item's
//! visible description.
//!
//! Linear has no custom fields, and the metadata slot (see `METADATA_OPEN_SPAN`) is an HTML
//! comment Linear never renders. So a source configured with `metadata_fields` writes, between
//! the caller's content and the slot of every task's, project's and document's long form, one
//! store-owned expandable block in the authored form — `<details>`, the summary `Fields`, and
//! one bullet per configured entry holding a value — which the engine's markup then writes as
//! Linear's own `+++ Fields` … `+++` section, as it writes any authored block. Which values an
//! entry shows is the shared rule, [`ProjectedValue`]; `docs/metadata.md` states the section's
//! shape for a caller.
//!
//! **Display only.** The slot stays the metadata's one home. The section is added where every
//! request is sent, to each long form a mutation carries that ends in a slot, regenerated from
//! that slot's metadata, and taken off every long form an answer carries: so nothing above the
//! transport — no read, no comparison deciding whether a write is needed, no stored-content
//! check — ever sees it. A person's edit to the section is not read, and the next write that
//! sends the long form writes it afresh; an unchanged write is still not sent, whatever Linear
//! did to the section's text. A comment's `body` carries no slot and gets no section.
//!
//! **How the section is told from a caller's block.** It is set off by a marker of its own, an
//! HTML comment holding a code span — the one spelling Linear keeps byte for byte, as the slot
//! is — directly before it: [`MARKER`], a blank line, then the block. A read takes off everything
//! from a marker standing on its own line and followed by an expandable block in either form to
//! the slot, and nothing else; so a caller's own block of any summary, `Fields` included, reads
//! back as it always did.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{ProjectedValue, SourceError, SourceName};
use serde_json::Value;

/// The marker the section is set off by.
pub(crate) const MARKER: &str = "<!-- `onetaskgraph.fields` -->";

/// The section's summary.
const SUMMARY: &str = "Fields";

/// One `metadata_fields` entry of a `linear` source's configuration: the same shape a
/// `github-projects` source takes.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[schemars(rename = "LinearMetadataField")]
#[serde(deny_unknown_fields)]
pub struct MetadataFieldConfig {
    /// The name the value is shown under, in bold, in the item's `Fields` section.
    pub field: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `MetadataField::resolve` refuses a blank or repeated name before the private validated entry is built.
    /// The top-level metadata key the value is read from. A key under the reserved
    /// `onetaskgraph.` prefix is refused.
    pub key: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `MetadataField::resolve` refuses a blank or reserved key before the private validated entry is built.
    /// Object keys walked inside that key's value, outermost first. Absent or empty, the
    /// key's own value is the one shown. Left out of a serialized entry when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    // Kept in the schema as `"default": []` although a serialized entry leaves an empty list
    // out, so both SDKs model an absent list as an empty one rather than as `null`.
    #[schemars(!skip_serializing_if)]
    pub path: Vec<String>,
}

/// One validated [`MetadataFieldConfig`] entry.
#[derive(Debug, Clone)]
pub(crate) struct MetadataField {
    field: ShownField,
    key: ShownKey,
    path: Vec<String>,
}

/// The name a value is shown under: never blank, and never one another entry of the same
/// source names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShownField(String);

/// The top-level metadata key a shown value is read from: never blank, and never under the
/// namespace this product reserves.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShownKey(String);

impl MetadataField {
    /// Validate every entry of one instance's `metadata_fields`, refusing the first that cannot
    /// stand, naming the source and the entry, by the rule a `github-projects` source reads its
    /// entries by.
    pub(crate) fn resolve(
        configured: Vec<MetadataFieldConfig>,
        instance: &SourceName,
    ) -> Result<Vec<Self>, SourceError> {
        let mut resolved: Vec<Self> = Vec::with_capacity(configured.len());
        for (index, entry) in configured.into_iter().enumerate() {
            if let Some(why) = ProjectedValue::entry_refusal(
                &entry.field,
                &entry.key,
                resolved.iter().map(|other| other.field.0.as_str()),
                "field",
            )
            .or_else(|| {
                entry.field.contains(char::is_control).then(|| {
                    "names a field holding a line break or another control character, which \
                     cannot stand on one line of the Fields section; next: name it on one line"
                        .to_owned()
                })
            }) {
                return Err(SourceError::Config {
                    message: format!(
                        "metadata_fields[{index}] of source {instance} (field {:?}, key {:?}) \
                         {why}",
                        entry.field, entry.key
                    ),
                });
            }
            resolved.push(Self {
                field: ShownField(entry.field),
                key: ShownKey(entry.key),
                path: entry.path,
            });
        }
        Ok(resolved)
    }

    /// The field's name, as `sources fields` reports it.
    pub(crate) fn field(&self) -> &str {
        &self.field.0
    }

    /// The key, as `sources fields` reports it.
    pub(crate) fn key(&self) -> &str {
        &self.key.0
    }

    /// The path, as `sources fields` reports it.
    pub(crate) fn path(&self) -> &[String] {
        &self.path
    }

    /// The bullet this entry shows for `metadata`, `None` for nothing to show, or the refusal
    /// for a value no section can show.
    fn bullet(
        &self,
        metadata: &BTreeMap<String, Value>,
        instance: &SourceName,
    ) -> Result<Option<String>, SourceError> {
        let value = match ProjectedValue::read(metadata, &self.key.0, &self.path) {
            Ok(Some(value)) if !value.is_empty() => value,
            Ok(_) => return Ok(None),
            Err(found) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "source {instance} shows metadata key {:?} at path {} as the field {:?} \
                         of an item's Fields section, and this item holds {found} there; only a \
                         string, an array of strings, an object of string values, null or \
                         nothing can be shown; next: store one of those there, or remove the \
                         value",
                        self.key.0,
                        serde_json::to_string(&self.path)
                            .unwrap_or_else(|_| format!("{:?}", self.path)),
                        self.field.0
                    ),
                });
            }
        };
        let head = format!("- **{}**:", markdown_escaped(&self.field.0));
        let nested = |lines: Vec<String>| {
            lines
                .iter()
                .map(|line| format!("\n  - {}", code_span(line)))
                .collect::<String>()
        };
        Ok(Some(match value {
            ProjectedValue::Text(text) if !text.contains('\n') => {
                format!("{head} {}", code_span(&text))
            }
            ProjectedValue::Text(text) => {
                format!(
                    "{head}{}",
                    nested(text.split('\n').map(str::to_owned).collect())
                )
            }
            ProjectedValue::List(list) => format!("{head}{}", nested(list)),
            ProjectedValue::Map(map) => format!(
                "{head}{}",
                nested(
                    map.iter()
                        .map(|(key, value)| format!("{key}: {value}"))
                        .collect()
                )
            ),
        }))
    }
}

/// `text` with every character Markdown could read as markup inside a bold label
/// backslash-escaped, as CommonMark escapes ASCII punctuation, so a field's name is shown as
/// written and never changes the section's structure.
fn markdown_escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if "\\`*_[]<>#!|&~".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// `text` as one inline code span, as CommonMark spells one: fenced by one backtick more than
/// the longest run it holds, padded by a space where it begins or ends with a backtick, and
/// with a line break as the space CommonMark reads it as — so it never spans two lines. The
/// empty string is a span of one space, since an empty span is no span.
fn code_span(text: &str) -> String {
    let text = text.replace("\r\n", " ").replace(['\n', '\r'], " ");
    let text = if text.is_empty() { " " } else { text.as_str() };
    let mut longest = 0;
    let mut run = 0;
    for character in text.chars() {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let padded = text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty());
    if padded {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// The section `fields` show for `metadata`, in the authored form, or `None` when no entry has
/// a value to show — or the refusal for the first value none can show.
pub(crate) fn section(
    fields: &[MetadataField],
    metadata: &BTreeMap<String, Value>,
    instance: &SourceName,
) -> Result<Option<String>, SourceError> {
    let mut bullets = Vec::new();
    for field in fields {
        if let Some(bullet) = field.bullet(metadata, instance)? {
            bullets.push(bullet);
        }
    }
    Ok((!bullets.is_empty()).then(|| {
        format!(
            "<details>\n<summary>{SUMMARY}</summary>\n\n{}\n\n</details>",
            bullets.join("\n")
        )
    }))
}

/// Where a section of this source's sits in `above` — the text of a long form above its
/// metadata slot: the byte its marker starts at, or `None` when it holds none.
///
/// A marker counts only on a line of its own, followed after blank lines by an expandable
/// block in either form, so prose quoting the marker is never taken for one.
pub(crate) fn marker_at(above: &str) -> Option<usize> {
    let at = above.rfind(MARKER)?;
    let own_line = at == 0 || above[..at].ends_with('\n');
    let after = above[at + MARKER.len()..].trim_start_matches(['\n', ' ']);
    let block = after.starts_with("<details>\n") || after.starts_with("+++ ");
    (own_line && block).then_some(at)
}
