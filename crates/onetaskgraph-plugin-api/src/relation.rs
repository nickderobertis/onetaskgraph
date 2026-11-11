//! Relations between tasks: a task's parent, and typed links between two items.
//!
//! Both are opt-in by what a caller writes, and neither touches an item that holds none: a
//! source answers [`TaskSource::read_relations`](crate::TaskSource::read_relations) and
//! [`TaskSource::set_parents`](crate::TaskSource::set_parents) only where it can hold a parent,
//! and a link lives in the store's own metadata on both of its endpoints, under
//! [`TaskLink::METADATA_KEY`], beside a `## Links` section the engine renders into the item's
//! content and every source strips on read with [`strip_links_section`].
//!
//! A link may also be mirrored as the destination's own relation, so it shows in that
//! tracker's relationship panel — opt-in per type, through a source's `link_types`
//! configuration, which [`LinkTypes::checked`] reads against the words a plugin can mirror as.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{NativeId, SourceError, Task};

/// One typed link, as one endpoint holds it under [`TaskLink::METADATA_KEY`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskLink {
    /// The link's type name — the caller's word, which no destination stores natively.
    // llmlint: ignore[invalid_states_unrepresentable] This is the stored shape the task fixes literally, `{"type": "<name>", …}`, read back from an endpoint a person can edit: a read answers what an endpoint holds rather than failing every link of it on one bad name, and the engine refuses an empty name both where it writes one (`LinkTypeName` in `engine/relations.rs`) and where it reads one (`stored_links`). A newtype here would change the contract every plugin builds against, which this crate keeps still.
    #[serde(rename = "type")]
    pub link_type: String,
    /// Which end of the directed link this item is.
    pub direction: LinkDirection,
    /// The other item's qualified id, `<source>:<native-id>`.
    // llmlint: ignore[invalid_states_unrepresentable] A qualified id is the engine's `GlobalId`, which this crate may not name (AGENTS.md: a plugin never sees a qualified id); a plugin only stores and returns this string, and the engine parses it on every read.
    pub other: String,
}

/// Which end of a directed link an item is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LinkDirection {
    /// This item is the link's `from`.
    Out,
    /// This item is the link's `to`.
    In,
}

impl TaskLink {
    /// The reserved metadata key every endpoint of a link holds its links under: a JSON list
    /// of `{"type", "direction", "other"}`.
    ///
    /// Written only by the engine's link verbs, through a targeted update, and never carried by
    /// a copy.
    pub const METADATA_KEY: &'static str = "onetaskgraph.links";

    /// The links `metadata` holds — none when it holds no [`Self::METADATA_KEY`].
    ///
    /// # Errors
    ///
    /// When the key holds something other than a list of links, naming what it holds.
    pub fn held(metadata: &BTreeMap<String, Value>) -> Result<Vec<Self>, String> {
        match metadata.get(Self::METADATA_KEY) {
            None => Ok(Vec::new()),
            Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
                format!(
                    "its `{}` is not a list of {{\"type\", \"direction\", \"other\"}}: {error}",
                    Self::METADATA_KEY
                )
            }),
        }
    }
}

/// What a source reads of one task's relations.
#[derive(Debug, Clone, PartialEq)]
pub struct Relations {
    /// The task as the source reads it — its links under [`TaskLink::METADATA_KEY`], its
    /// version, and its content with the `## Links` section already stripped.
    pub task: Task,
    /// Its parent, a task of the same source, or `None`.
    pub parent: Option<NativeId>,
    /// Its children, each a task of the same source.
    pub children: Vec<NativeId>,
    /// Whether `children` is every child: `false` when the source read fewer than it holds.
    pub children_complete: bool,
}

/// One parent write: `child`'s parent set to `parent`, or removed for `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentWrite {
    /// The task whose parent is written.
    pub child: NativeId,
    /// Its new parent, a task of the same source, or `None` to remove the one it has.
    pub parent: Option<NativeId>,
    /// The version the child must be at for anything to be written.
    // llmlint: ignore[invalid_states_unrepresentable] As `BatchUpdate::if_version`: a source's own opaque token, `None` the ordinary unheld write.
    pub if_version: Option<String>,
}

/// What became of one [`ParentWrite`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentAnswer {
    /// The parent was written; `version` is the child's after it, when the write's answer says.
    Written {
        /// The child's version after the write.
        version: Option<String>,
    },
    /// The child already had that parent, so nothing was written.
    Unchanged {
        /// The child's version, as read.
        version: Option<String>,
    },
    /// The child was at another version than the one it was held to; nothing was written.
    VersionMoved {
        /// The version the source found it at.
        found: Option<String>,
    },
    /// The source, or its destination, refused this edge alone — a cycle, or a limit on
    /// children or depth — and says why.
    Refused {
        /// The refusal, in the destination's own words where it gave some.
        reason: String,
    },
    /// The source holds no such child, or no such parent.
    Missing {
        /// Which of the two is missing.
        what: NativeId,
    },
}

/// One native relation to add or remove between two tasks of one source, mirroring a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLink {
    /// The link's `from`.
    pub from: NativeId,
    /// The link's `to`.
    pub to: NativeId,
    /// The destination's own word for the relation, as [`LinkTypes`] names it.
    pub native: String,
}

/// A source's `link_types` configuration: each link type a source mirrors natively, with the
/// destination's word for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkTypes(BTreeMap<String, String>);

/// One entry of a source's `link_types` configuration, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkTypeConfig {
    /// The destination's own relation this type is mirrored as.
    #[serde(default)]
    pub native: Option<String>,
}

impl LinkTypes {
    /// `configured` checked against `words`, the native relations the `kind` plugin can mirror
    /// a link as — refusing any other word, naming the type and the word, and why for the words
    /// the store already gives a meaning or that change an item's state.
    ///
    /// # Errors
    ///
    /// [`SourceError::Config`] naming the first type whose `native` word the plugin cannot
    /// mirror as.
    pub fn checked(
        kind: &str,
        configured: &BTreeMap<String, LinkTypeConfig>,
        words: &[&str],
    ) -> Result<Self, SourceError> {
        let mut mirrored = BTreeMap::new();
        for (name, entry) in configured {
            if name.trim().is_empty() {
                return Err(SourceError::Config {
                    message: "link_types names a type with an empty name; name each link type"
                        .to_owned(),
                });
            }
            let Some(word) = &entry.native else {
                continue;
            };
            if !words.contains(&word.as_str()) {
                let why = match word.as_str() {
                    "blocks" | "blocked-by" | "blocked_by" | "blocking" => {
                        " — blocking is what a task's `depends_on` already means, so no link type \
                         is mirrored as it"
                            .to_owned()
                    }
                    "duplicate" | "similar" => {
                        " — a native duplicate closes the issue on GitHub and moves it to \
                         `Duplicate` on Linear, so a duplicate link stays in the store's metadata \
                         alone"
                            .to_owned()
                    }
                    _ => String::new(),
                };
                let allowed = if words.is_empty() {
                    format!("the {kind} plugin mirrors no link natively; remove `native`")
                } else {
                    format!(
                        "the {kind} plugin mirrors a link only as {}",
                        words
                            .iter()
                            .map(|word| format!("`{word}`"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    )
                };
                return Err(SourceError::Config {
                    message: format!(
                        "link_types.{name}.native is `{word}`, which this source cannot mirror a \
                         link as{why}; {allowed}"
                    ),
                });
            }
            mirrored.insert(name.clone(), word.clone());
        }
        Ok(Self(mirrored))
    }

    /// The native word `link_type` is mirrored as, or `None` for a type kept in metadata alone.
    #[must_use]
    pub fn native(&self, link_type: &str) -> Option<&str> {
        self.0.get(link_type).map(String::as_str)
    }

    /// Every mirrored type and its word.
    #[must_use]
    pub fn mirrored(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

/// The refusal a source holding no relations answers a relation read or write with.
#[must_use]
pub fn relationless(kind: &str) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin holds no relations between tasks"),
    }
}

/// The heading of the section the engine renders an item's links under, last in its content.
pub const LINKS_HEADING: &str = "## Links";

/// `content` with the trailing `## Links` section the engine rendered removed — exactly the
/// content it was rendered onto, on a source that stores what it is sent.
///
/// The section is recognised only as the last thing in `content`: a line holding exactly
/// [`LINKS_HEADING`], followed by nothing but blank lines and bullets — `- ` or `* `, because
/// Linear stores a `- ` bullet as `* ` and changes blank lines. Anything else after the heading
/// means a person wrote it, and `content` is answered unchanged.
#[must_use]
pub fn strip_links_section(content: &str) -> &str {
    let mut at = None;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if line.trim_end() == LINKS_HEADING {
            at = Some(offset);
        }
        offset += line.len();
    }
    let Some(start) = at else {
        return content;
    };
    let after = &content[start..];
    let only_bullets = after
        .split_inclusive('\n')
        .skip(1)
        .map(str::trim)
        .all(|line| line.is_empty() || line.starts_with("- ") || line.starts_with("* "));
    if !only_bullets {
        return content;
    }
    let before = &content[..start];
    before
        .strip_suffix("\n\n")
        .unwrap_or_else(|| before.trim_end_matches('\n'))
}

/// `content` with `bullets` rendered as a `## Links` section after it — `content` itself when
/// there are none. [`strip_links_section`] gives back exactly `content`.
#[must_use]
pub fn with_links_section(content: &str, bullets: &[String]) -> String {
    if bullets.is_empty() {
        return content.to_owned();
    }
    let mut out = String::with_capacity(content.len() + 64);
    if !content.is_empty() {
        out.push_str(content);
        out.push_str("\n\n");
    }
    out.push_str(LINKS_HEADING);
    out.push_str("\n\n");
    for bullet in bullets {
        out.push_str("- ");
        out.push_str(bullet);
        out.push('\n');
    }
    out
}

/// `content` as a read answers it for an item whose `metadata` holds links: the `## Links`
/// section stripped, and `None` where nothing but the section was there. Content of an item
/// holding no links is answered unchanged, so a section a person wrote is never touched.
#[must_use]
pub fn read_content(content: Option<String>, metadata: &BTreeMap<String, Value>) -> Option<String> {
    if !metadata.contains_key(TaskLink::METADATA_KEY) {
        return content;
    }
    let content = content?;
    let stripped = strip_links_section(&content);
    if stripped.len() == content.len() {
        return Some(content);
    }
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_owned())
    }
}

/// The `## Links` line of `link` when nothing better is known of its other end: its type, its
/// direction, and the other end's qualified id.
#[must_use]
pub fn link_line(link: &TaskLink) -> String {
    format!(
        "**{}** {} `{}`",
        link.link_type,
        arrow(link.direction),
        link.other
    )
}

/// The arrow a `## Links` line shows a direction with.
#[must_use]
pub const fn arrow(direction: LinkDirection) -> &'static str {
    match direction {
        LinkDirection::Out => "→",
        LinkDirection::In => "←",
    }
}

/// `content` as a write of an item holding `metadata` stores it: with its `## Links` section
/// re-rendered from the links `metadata` holds, so a content write never drops what a person
/// sees of them.
///
/// Each link keeps the line `stored` — the content as the source stores it now, its section
/// included — already shows it with, so its other end's title and address survive; a link the
/// stored section does not show gets [`link_line`]. `content` already ending in a section — a
/// link write's own — is answered unchanged, and so is any content of an item holding no
/// links, so a write of an item that never had one is exactly what it was.
#[must_use]
pub fn relinked(content: &str, stored: Option<&str>, metadata: &BTreeMap<String, Value>) -> String {
    let Ok(links) = TaskLink::held(metadata) else {
        return content.to_owned();
    };
    if links.is_empty() || strip_links_section(content).len() != content.len() {
        return content.to_owned();
    }
    let shown: Vec<&str> = stored
        .map(|stored| {
            let kept = strip_links_section(stored).len();
            stored[kept..]
                .lines()
                .map(str::trim)
                .filter_map(|line| line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")))
                .collect()
        })
        .unwrap_or_default();
    let bullets: Vec<String> = links
        .iter()
        .map(|link| {
            let typed = format!("**{}** {} ", link.link_type, arrow(link.direction));
            let named = format!("`{}`", link.other);
            shown
                .iter()
                .find(|line| line.starts_with(&typed) && line.ends_with(&named))
                .map_or_else(|| link_line(link), |line| (*line).to_owned())
        })
        .collect();
    with_links_section(content, &bullets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bullets() -> Vec<String> {
        vec!["**caused-by** → [B](https://example.test/b) `s:b`".to_owned()]
    }

    #[test]
    fn a_rendered_section_strips_back_to_exactly_the_content_it_was_rendered_onto() {
        for content in ["", "Body", "Body\n", "Body\n\n", "# Title\n\ntext\n"] {
            let rendered = with_links_section(content, &bullets());
            assert_ne!(rendered, content);
            assert_eq!(strip_links_section(&rendered), content, "{content:?}");
        }
    }

    #[test]
    fn linears_stars_and_extra_blank_lines_are_still_the_section() {
        let stored = "Body\n\n## Links\n\n\n* **caused-by** → [B](u)\n\n\n* **x** ← [C](v)\n";
        assert_eq!(strip_links_section(stored), "Body");
    }

    #[test]
    fn a_links_heading_a_person_wrote_prose_under_is_left_alone() {
        let written = "Body\n\n## Links\n\nSee the wiki.\n";
        assert_eq!(strip_links_section(written), written);
        assert_eq!(strip_links_section("no section"), "no section");
    }

    #[test]
    fn content_of_an_item_holding_no_links_is_never_stripped() {
        let rendered = with_links_section("Body", &bullets());
        assert_eq!(
            read_content(Some(rendered.clone()), &BTreeMap::new()),
            Some(rendered.clone())
        );
        let mut metadata = BTreeMap::new();
        metadata.insert(TaskLink::METADATA_KEY.to_owned(), Value::Array(Vec::new()));
        assert_eq!(
            read_content(Some(rendered), &metadata),
            Some("Body".to_owned())
        );
        assert_eq!(
            read_content(Some(with_links_section("", &bullets())), &metadata),
            None
        );
    }

    #[test]
    fn a_native_word_the_plugin_cannot_mirror_as_is_refused_naming_the_type_and_the_word() {
        let configured = |word: &str| {
            BTreeMap::from([(
                "caused-by".to_owned(),
                LinkTypeConfig {
                    native: Some(word.to_owned()),
                },
            )])
        };
        assert!(
            LinkTypes::checked(
                "github-projects",
                &configured("relates-to"),
                &["relates-to"]
            )
            .is_ok()
        );
        for word in ["blocks", "blocked-by", "duplicate", "similar", "frobs"] {
            let Err(SourceError::Config { message }) =
                LinkTypes::checked("github-projects", &configured(word), &["relates-to"])
            else {
                panic!("{word} was accepted");
            };
            assert!(message.contains("caused-by"), "{message}");
            assert!(message.contains(&format!("`{word}`")), "{message}");
        }
        let Err(SourceError::Config { message }) =
            LinkTypes::checked("local-md", &configured("related"), &[])
        else {
            panic!("a native word was accepted on a plugin with none");
        };
        assert!(message.contains("mirrors no link natively"), "{message}");
    }

    #[test]
    fn links_are_read_from_their_key_and_a_malformed_value_is_refused() {
        let mut metadata = BTreeMap::new();
        assert!(TaskLink::held(&metadata).unwrap().is_empty());
        metadata.insert(
            TaskLink::METADATA_KEY.to_owned(),
            serde_json::json!([{"type": "t", "direction": "out", "other": "s:b"}]),
        );
        assert_eq!(
            TaskLink::held(&metadata).unwrap(),
            vec![TaskLink {
                link_type: "t".to_owned(),
                direction: LinkDirection::Out,
                other: "s:b".to_owned()
            }]
        );
        metadata.insert(TaskLink::METADATA_KEY.to_owned(), serde_json::json!("x"));
        assert!(TaskLink::held(&metadata).is_err());
    }

    fn holding(links: serde_json::Value) -> BTreeMap<String, Value> {
        BTreeMap::from([(TaskLink::METADATA_KEY.to_owned(), links)])
    }

    #[test]
    fn a_content_write_re_renders_the_section_keeping_each_stored_line() {
        let metadata = holding(serde_json::json!([
            {"type": "caused-by", "direction": "out", "other": "s:b"},
            {"type": "relates", "direction": "in", "other": "t:c"},
        ]));
        let stored = with_links_section("Old", &bullets());
        let written = relinked("New", Some(&stored), &metadata);
        assert_eq!(
            written,
            with_links_section(
                "New",
                &[
                    "**caused-by** → [B](https://example.test/b) `s:b`".to_owned(),
                    "**relates** ← `t:c`".to_owned(),
                ]
            )
        );
        assert_eq!(strip_links_section(&written), "New");
        // Linear's `*` bullets are the same lines.
        let starred = stored.replace("\n- ", "\n* ");
        assert_eq!(relinked("New", Some(&starred), &metadata), written);
    }

    #[test]
    fn content_carrying_a_section_or_of_an_item_with_no_links_is_written_as_given() {
        let metadata = holding(serde_json::json!([
            {"type": "caused-by", "direction": "out", "other": "s:b"}
        ]));
        let composed = with_links_section("Body", &bullets());
        assert_eq!(relinked(&composed, None, &metadata), composed);
        assert_eq!(relinked("Body", Some(&composed), &BTreeMap::new()), "Body");
        assert_eq!(
            relinked("Body", None, &metadata),
            with_links_section("Body", &["**caused-by** → `s:b`".to_owned()])
        );
    }
}
