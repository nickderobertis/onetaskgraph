//! Image assets: the one Markdown convention that references one, the shapes a source reads and
//! writes them in, and the record a hosted source keeps of what it uploaded.
//!
//! # The convention
//!
//! In a task's or a document's content, an asset is referenced by a Markdown image whose
//! target is `./<name>` — `![<alt>](./<name>)` — where `<name>` is an [`AssetName`]: a bare
//! file name with no `/`, no `\` and no `..`, ending, case-insensitively, in `.png`, `.jpg`,
//! `.jpeg`, `.gif` or `.webp`. Nothing else is an asset reference: every other link, absolute
//! or relative, an image whose target has a directory or no `./`, and a plain link to
//! `./<name>.png`, is left exactly as written. [`asset_references`] is the one reader of the
//! convention and [`rewrite_asset_references`] the one writer, so the engine and every plugin
//! agree about which text is a reference.
//!
//! # What crosses the seam
//!
//! A record's assets are read as [`Asset`]s — name, SHA-256, content type and, for a source
//! that keeps them on this machine, the path holding the bytes — and their bytes one asset at
//! a time. They are written as an [`AssetWrite`] beside the record's own write: one
//! [`AssetPayload`] per asset the content references, and the destination record's existing
//! [`AssetUploads`] when it has one. A payload whose SHA-256 equals the one recorded there
//! carries no bytes, because the destination already serves exactly those.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{MetadataKey, NativeId, SourceError};

/// The extensions an asset may end in, lower-cased, each with the content type it is stored
/// and served as.
const EXTENSIONS: [(&str, AssetContentType); 5] = [
    ("png", AssetContentType::Png),
    ("jpg", AssetContentType::Jpeg),
    ("jpeg", AssetContentType::Jpeg),
    ("gif", AssetContentType::Gif),
    ("webp", AssetContentType::Webp),
];

/// The prefix an asset reference's target carries before the name.
const REFERENCE_PREFIX: &str = "./";

/// One asset's name: a bare file name ending in an accepted image extension.
///
/// Validated wherever one is built, deserialized included: non-empty, no `/`, no `\`, no
/// `..`, no whitespace, control character or parenthesis — none of which a bare Markdown link
/// target can hold — and ending, case-insensitively, in `.png`, `.jpg`, `.jpeg`, `.gif` or
/// `.webp` after a non-empty stem. The name is kept exactly as written, case included: it is
/// the text a reference names and the file name a source stores the bytes under.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct AssetName(String);

impl AssetName {
    /// One name, once it is established that it is one.
    ///
    /// # Errors
    ///
    /// A message naming the name and what is wrong with it, and what to write instead.
    pub fn new(name: impl Into<String>) -> Result<Self, String> {
        let name = name.into();
        let refuse = |why: &str| {
            Err(format!(
                "the asset name {name:?} {why}; an asset is a bare file name ending in .png, \
                 .jpg, .jpeg, .gif or .webp"
            ))
        };
        if name.is_empty() {
            return refuse("is empty");
        }
        if name.contains('/') || name.contains('\\') {
            return refuse("has a directory in it");
        }
        if name.contains("..") {
            return refuse("contains `..`");
        }
        if name
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        {
            return refuse("contains whitespace or a control character");
        }
        if name.contains(['(', ')', '<', '>']) {
            return refuse("contains a parenthesis or an angle bracket");
        }
        if content_type_of(&name).is_none() {
            return refuse("does not end in an accepted image extension after a non-empty stem");
        }
        Ok(Self(name))
    }

    /// The name, as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The content type its extension stores and serves it as.
    #[must_use]
    pub fn content_type(&self) -> AssetContentType {
        content_type_of(&self.0).expect("an `AssetName` ends in an accepted extension")
    }
}

impl fmt::Display for AssetName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for AssetName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<AssetName> for String {
    fn from(value: AssetName) -> Self {
        value.0
    }
}

/// The content type of a name's extension, when it has an accepted one after a non-empty
/// stem.
fn content_type_of(name: &str) -> Option<AssetContentType> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    EXTENSIONS
        .iter()
        .find(|(accepted, _)| extension.eq_ignore_ascii_case(accepted))
        .map(|(_, content_type)| *content_type)
}

/// The content type an asset is stored and served as, decided by its name's extension alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum AssetContentType {
    /// `.png`.
    #[serde(rename = "image/png")]
    Png,
    /// `.jpg` and `.jpeg`.
    #[serde(rename = "image/jpeg")]
    Jpeg,
    /// `.gif`.
    #[serde(rename = "image/gif")]
    Gif,
    /// `.webp`.
    #[serde(rename = "image/webp")]
    Webp,
}

impl AssetContentType {
    /// The media type, as it is serialized.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

/// Whether `digest` is a SHA-256 as this contract spells one: 64 lowercase hex digits.
#[must_use]
pub fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The lowercase hex SHA-256 of `bytes`: what [`Asset::sha256`], [`AssetPayload::sha256`] and
/// [`AssetUpload::sha256`] hold.
#[must_use]
pub fn asset_sha256(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        // Writing to a `String` never fails.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// One asset a record holds, as a read reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    /// Its name, which the record's content references as `./<name>`.
    pub name: AssetName,
    /// The lowercase hex SHA-256 of its bytes.
    // llmlint: ignore[invalid_states_unrepresentable] This member's JSON shape — a lowercase hex string — is the asset contract `docs/plugin-protocol.md` §4.9a states, and plugins in other crates and other languages read and write it as exactly that. The digest is computed by `asset_sha256` wherever it is made, and every source that receives bytes refuses them unless they hash to it before storing anything; a recorded one is checked as a digest where `AssetUploads::read` reads it.
    pub sha256: String,
    /// The content type its extension gives it.
    pub content_type: AssetContentType,
    /// The absolute path holding its bytes on this machine, for a source that keeps them
    /// here; `null` for a hosted source.
    // llmlint: ignore[invalid_states_unrepresentable] This member is an absolute path string or null in what `show --json` prints, as the README states. Only a source reports it, from the path it wrote the bytes to — local-md's canonicalized record path joined with the asset's validated name — and nothing reads it back to decide anything.
    pub path: Option<String>,
}

/// One asset a create or an update carries to a destination.
///
/// `bytes` is base64 over the out-of-process protocol and the raw bytes in process. It is
/// absent exactly when the destination's existing [`AssetUploads`] records this name with
/// this `sha256`: the destination already serves those bytes, so nothing is sent again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AssetPayload {
    /// Its name, which the record's content references as `./<name>`.
    pub name: AssetName,
    /// The lowercase hex SHA-256 of its bytes.
    // llmlint: ignore[invalid_states_unrepresentable] This member's JSON shape — a lowercase hex string — is the asset contract `docs/plugin-protocol.md` §4.9a states, and plugins in other crates and other languages read and write it as exactly that. The digest is computed by `asset_sha256` wherever it is made, and every source that receives bytes refuses them unless they hash to it before storing anything; a recorded one is checked as a digest where `AssetUploads::read` reads it.
    pub sha256: String,
    /// The content type its extension gives it.
    pub content_type: AssetContentType,
    /// Its bytes, or absent when the destination already records them by `sha256`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "base64_out",
        deserialize_with = "base64_in"
    )]
    #[schemars(with = "Option<String>", extend("contentEncoding" = "base64"))]
    pub bytes: Option<Vec<u8>>,
}

impl AssetPayload {
    /// A payload carrying `bytes`, its digest and content type taken from them and from the
    /// name.
    #[must_use]
    pub fn of(name: AssetName, bytes: Vec<u8>) -> Self {
        Self {
            sha256: asset_sha256(&bytes),
            content_type: name.content_type(),
            name,
            bytes: Some(bytes),
        }
    }
}

// llmlint: ignore[suppressions_justified] serde's `serialize_with` hands the field by
// reference to the field's own type, so the signature is fixed by serde rather than chosen.
#[allow(clippy::ref_option)]
fn base64_out<S: Serializer>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
    match bytes {
        Some(bytes) => {
            serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        None => serializer.serialize_none(),
    }
}

fn base64_in<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error> {
    let encoded = Option::<String>::deserialize(deserializer)?;
    encoded
        .map(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded.as_bytes())
                .map_err(|error| {
                    serde::de::Error::custom(format!("asset bytes are not base64: {error}"))
                })
        })
        .transpose()
}

/// What a hosted source uploaded for one asset of one record: the bytes' SHA-256 and the URL
/// it serves them at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AssetUpload {
    /// The lowercase hex SHA-256 of the bytes uploaded.
    // llmlint: ignore[invalid_states_unrepresentable] This member's JSON shape — a lowercase hex string — is the asset contract `docs/plugin-protocol.md` §4.9a states, and plugins in other crates and other languages read and write it as exactly that. The digest is computed by `asset_sha256` wherever it is made, and every source that receives bytes refuses them unless they hash to it before storing anything; a recorded one is checked as a digest where `AssetUploads::read` reads it.
    pub sha256: String,
    /// Where the destination serves them.
    // llmlint: ignore[invalid_states_unrepresentable] `onetaskgraph.assets` is `{"sha256": <hex>, "url": <string>}` in the contract `docs/plugin-protocol.md` §4.9a states. A URL is whatever the destination serves the bytes at — its own scheme, as `in-memory://` shows — so no narrower type says more than a string; nothing here dereferences one.
    pub url: String,
}

/// The value of [`MetadataKey::ASSETS_KEY`] on a destination record: what its hosted source
/// uploaded, by asset name.
///
/// Written by the hosted plugin in the same write that lands the record, and read back by the
/// next copy onto it: an asset whose SHA-256 equals the one recorded here reuses the recorded
/// URL and is not uploaded again.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AssetUploads(pub BTreeMap<AssetName, AssetUpload>);

impl AssetUploads {
    /// What `metadata` records under [`MetadataKey::ASSETS_KEY`], or `None` when it records
    /// nothing there.
    ///
    /// # Errors
    ///
    /// A message naming the key when what it holds is not this shape.
    pub fn read(metadata: &BTreeMap<String, Value>) -> Result<Option<Self>, String> {
        let Some(value) = metadata.get(MetadataKey::ASSETS_KEY) else {
            return Ok(None);
        };
        let uploads: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "{} holds {value}, which is not an object of asset names to \
                 {{\"sha256\", \"url\"}}: {error}",
                MetadataKey::ASSETS_KEY
            )
        })?;
        if let Some((name, upload)) = uploads
            .0
            .iter()
            .find(|(_, upload)| !is_sha256(&upload.sha256) || upload.url.is_empty())
        {
            return Err(format!(
                "{} records {name} with sha256 {:?} and url {:?}; a record is a lowercase hex \
                 SHA-256 and a non-empty url",
                MetadataKey::ASSETS_KEY,
                upload.sha256,
                upload.url
            ));
        }
        Ok(Some(uploads))
    }

    /// The value this record is written under [`MetadataKey::ASSETS_KEY`] as.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("an asset record is plain JSON")
    }

    /// The URL recorded for `name`, when it was recorded with exactly `sha256`.
    #[must_use]
    pub fn reusable(&self, name: &AssetName, sha256: &str) -> Option<&str> {
        self.0
            .get(name)
            .filter(|upload| upload.sha256 == sha256)
            .map(|upload| upload.url.as_str())
    }
}

/// The assets a create or an update of one task or document carries.
///
/// Over the out-of-process protocol its two members sit in the write's `params` beside
/// `write` — `docs/plugin-protocol.md` §4.9 — and in process it is the argument the asset
/// writes of [`TaskSource`](crate::TaskSource) take beside the record's own write. One type for
/// both, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct AssetWrite {
    /// One payload per asset the record's content references, in the order it first
    /// references them — the record's whole asset set once the write lands, so an asset the
    /// destination holds and this does not name is removed.
    pub assets: Vec<AssetPayload>,
    /// The destination record's existing [`MetadataKey::ASSETS_KEY`], when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_assets: Option<AssetUploads>,
}

/// What an asset-carrying write answers with: where the record now is, and the content the
/// destination stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AssetsWritten {
    /// The id the destination holds the record under.
    pub id: NativeId,
    /// The content as stored — each `./<name>` reference pointed at where the destination
    /// serves that asset, for a source that serves them at a URL; unchanged for one that keeps
    /// them beside the record.
    #[serde(default)]
    pub content: Option<String>,
}

/// One asset reference in a content: where its target sits and the name it names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reference {
    /// The byte range of the target — `./<name>` — inside the content.
    target: std::ops::Range<usize>,
    /// The name it references.
    name: AssetName,
}

/// Every asset reference in `content`, in order, duplicates included.
fn references(content: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = content[from..].find("![") {
        let start = from + at;
        from = start + 2;
        let Some(alt_end) = content[from..].find(']').map(|end| from + end) else {
            break;
        };
        if content[from..alt_end].contains('\n') {
            continue;
        }
        let open = alt_end + 1;
        if content.as_bytes().get(open) != Some(&b'(') {
            continue;
        }
        let target_start = open + 1;
        let target_end = content[target_start..]
            .find(|character: char| character == ')' || character.is_whitespace())
            .map_or(content.len(), |end| target_start + end);
        if target_end >= content.len() {
            continue;
        }
        let closes = content[target_end..]
            .split_once(')')
            .is_some_and(|(between, _)| !between.contains('\n'));
        let target = &content[target_start..target_end];
        if let (true, Some(name)) = (closes, target.strip_prefix(REFERENCE_PREFIX))
            && let Ok(name) = AssetName::new(name)
        {
            found.push(Reference {
                target: target_start..target_end,
                name,
            });
            from = target_end;
        }
    }
    found
}

/// The assets `content` references, each once, in the order it first references them.
#[must_use]
pub fn asset_references(content: &str) -> Vec<AssetName> {
    let mut names: Vec<AssetName> = Vec::new();
    for reference in references(content) {
        if !names.contains(&reference.name) {
            names.push(reference.name);
        }
    }
    names
}

/// `content` with the target of each reference to an asset `served` names replaced by the URL
/// it maps to, and every other byte as it was.
#[must_use]
pub fn rewrite_asset_references(content: &str, served: &BTreeMap<AssetName, String>) -> String {
    let mut rewritten = String::with_capacity(content.len());
    let mut copied = 0;
    for reference in references(content) {
        if let Some(url) = served.get(&reference.name) {
            rewritten.push_str(&content[copied..reference.target.start]);
            rewritten.push_str(url);
            copied = reference.target.end;
        }
    }
    rewritten.push_str(&content[copied..]);
    rewritten
}

/// Point a record's asset references at where a hosted source serves them, and record what it
/// uploaded: the one way a plugin that declares assets `native` and serves them at a URL
/// finishes the record it is about to write.
///
/// `content` is rewritten by [`rewrite_asset_references`] over `uploads`, and `metadata` gains
/// `uploads` under [`MetadataKey::ASSETS_KEY`] — or loses that key, when `uploads` is empty,
/// so a record that holds no asset records nothing about assets. When `metadata` records a template rendering
/// under [`MetadataKey::TEMPLATE_KEY`] whose `body_digest` is still the digest of `content`,
/// that entry's `body_digest` is re-recorded as the digest of the rewritten content and its
/// `template`, `digest` and `answers_digest` are carried as they were — the rule a copy that
/// rewrites references follows (README, "Creating and regenerating from a template"). Any other
/// entry, a hand edit the rendering no longer vouches for included, is carried verbatim.
#[must_use]
pub fn serve_asset_references(
    content: &str,
    metadata: &mut BTreeMap<String, Value>,
    uploads: &AssetUploads,
) -> String {
    let served = uploads
        .0
        .iter()
        .map(|(name, upload)| (name.clone(), upload.url.clone()))
        .collect();
    let rewritten = rewrite_asset_references(content, &served);
    if rewritten != content
        && let Some(Value::Object(entry)) = metadata.get_mut(MetadataKey::TEMPLATE_KEY)
        && entry.get("body_digest").and_then(Value::as_str) == Some(&body_digest(content))
    {
        entry.insert(
            "body_digest".to_owned(),
            Value::String(body_digest(&rewritten)),
        );
    }
    if uploads.0.is_empty() {
        metadata.remove(MetadataKey::ASSETS_KEY);
    } else {
        metadata.insert(MetadataKey::ASSETS_KEY.to_owned(), uploads.to_value());
    }
    rewritten
}

/// `sha256:` and the lowercase hex SHA-256 of `content`: what a rendering's `body_digest`
/// records.
#[must_use]
pub fn body_digest(content: &str) -> String {
    format!("sha256:{}", asset_sha256(content.as_bytes()))
}

/// The refusal a source that keeps no assets answers an asset write with.
///
/// Spelled once beside [`unwritable`](crate::unwritable), for that function's reason: every
/// source without assets refuses in the same words, and the engine's own refusal of a copy
/// into one names the source, the record and the asset before any source is asked.
#[must_use]
pub fn assetless(kind: &str) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin cannot store image assets"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(name: &str) -> AssetName {
        AssetName::new(name).expect("a valid name")
    }

    #[test]
    fn only_a_dot_slash_image_of_an_accepted_bare_name_is_a_reference() {
        let content = "![a](./one.png) ![b](https://example.invalid/x.png) \
                       ![c](./img/two.png) ![d](../three.png) ![e](four.png) \
                       [f](./five.png) [g](./notes.txt) ![h](./six.JpEg \"title\") \
                       ![i](./one.png) ![j](./seven.txt) ![k](./eight.gif";
        assert_eq!(
            asset_references(content),
            vec![name("one.png"), name("six.JpEg")]
        );
    }

    #[test]
    fn a_rewrite_touches_the_targets_it_maps_and_nothing_else() {
        let content = "see ![a](./one.png) and ![b](./two.webp \"t\") and [c](./one.png)";
        let served = BTreeMap::from([(name("one.png"), "https://h/1".to_owned())]);
        assert_eq!(
            rewrite_asset_references(content, &served),
            "see ![a](https://h/1) and ![b](./two.webp \"t\") and [c](./one.png)"
        );
    }

    #[test]
    fn names_are_refused_by_what_is_wrong_with_them() {
        for bad in [
            "", "a/b.png", "a\\b.png", "..png", "a..b.png", "a b.png", ".png", "a.txt",
        ] {
            assert!(AssetName::new(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(name("X.PNG").content_type(), AssetContentType::Png);
        assert_eq!(name("x.JpEg").content_type(), AssetContentType::Jpeg);
        assert_eq!(name("x.jpg").content_type(), AssetContentType::Jpeg);
        assert_eq!(name("x.gif").content_type(), AssetContentType::Gif);
        assert_eq!(name("x.webp").content_type(), AssetContentType::Webp);
    }

    #[test]
    fn bytes_cross_as_base64_and_are_omitted_when_absent() {
        let payload = AssetPayload::of(name("a.png"), vec![0, 1, 2, 255]);
        let value = serde_json::to_value(&payload).expect("serializes");
        assert_eq!(value["bytes"], "AAEC/w==");
        let back: AssetPayload = serde_json::from_value(value).expect("deserializes");
        assert_eq!(back, payload);
        let reused = AssetPayload {
            bytes: None,
            ..payload
        };
        let value = serde_json::to_value(&reused).expect("serializes");
        assert!(value.get("bytes").is_none());
    }

    #[test]
    fn a_record_whose_digest_is_not_one_is_refused_where_it_is_read() {
        let metadata = BTreeMap::from([(
            MetadataKey::ASSETS_KEY.to_owned(),
            serde_json::json!({"a.png": {"sha256": "not hex", "url": "https://h/a"}}),
        )]);
        let refused = AssetUploads::read(&metadata).expect_err("refused");
        assert!(
            refused.contains("a.png") && refused.contains("not hex"),
            "{refused}"
        );
        assert!(is_sha256(&asset_sha256(b"x")));
        assert!(!is_sha256(&asset_sha256(b"x").to_uppercase()));
    }

    #[test]
    fn serving_restamps_a_rendering_that_still_matches_and_leaves_a_hand_edit_alone() {
        let content = "![a](./one.png)";
        let uploads = AssetUploads(BTreeMap::from([(
            name("one.png"),
            AssetUpload {
                sha256: "00".to_owned(),
                url: "https://h/1".to_owned(),
            },
        )]));
        let entry = |digest: String| {
            serde_json::json!({
                "template": "t", "digest": "sha256:aa", "body_digest": digest,
                "answers_digest": "sha256:bb"
            })
        };
        let mut metadata = BTreeMap::from([(
            MetadataKey::TEMPLATE_KEY.to_owned(),
            entry(body_digest(content)),
        )]);
        let rewritten = serve_asset_references(content, &mut metadata, &uploads);
        assert_eq!(rewritten, "![a](https://h/1)");
        assert_eq!(
            metadata[MetadataKey::TEMPLATE_KEY],
            entry(body_digest(&rewritten))
        );
        assert_eq!(metadata[MetadataKey::ASSETS_KEY], uploads.to_value());

        let mut edited = BTreeMap::from([(
            MetadataKey::TEMPLATE_KEY.to_owned(),
            entry("sha256:00".to_owned()),
        )]);
        let _ = serve_asset_references(content, &mut edited, &uploads);
        assert_eq!(
            edited[MetadataKey::TEMPLATE_KEY],
            entry("sha256:00".to_owned())
        );
    }
}
