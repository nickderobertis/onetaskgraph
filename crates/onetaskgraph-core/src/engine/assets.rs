//! The image assets a create, a render and a copy carry, and the order a read lists them in.
//!
//! What is an asset reference is `onetaskgraph-plugin-api`'s one reader of the convention,
//! [`asset_references`]; everything here is the engine's half: which assets a write carries,
//! which refusals it owes before anything is written, and which bytes a destination that
//! already serves an asset is not sent again.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    Asset, AssetName, AssetPayload, AssetUploads, AssetWrite, SourceError, asset_references,
};
use serde_json::Value;

use super::EngineError;
use crate::resolve::ResolvedSource;

/// Which kind of record an asset belongs to: the two that hold assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// A task.
    Task,
    /// A document.
    Document,
}

/// Whether a record holds any asset: one its source lists, or — for a source that lists none,
/// as one reached over the stdio protocol does — one its own record of uploads still names.
pub(super) fn holds_any(listed: &[Asset], recorded: Option<&AssetUploads>) -> bool {
    !listed.is_empty() || recorded.is_some_and(|recorded| !recorded.0.is_empty())
}

/// The assets a write of `content` carries: one payload per asset it references, in the order
/// it first references them, each taken from `given` or else from `kept`.
///
/// # Errors
///
/// Refused by name, before anything is written: [`EngineError::AssetGivenTwice`] for two of
/// `given` under one name, [`EngineError::AssetNotReferenced`] for one of `given` the content
/// does not reference, and [`EngineError::AssetNotGiven`] for a reference neither `given` nor
/// `kept` holds.
pub(super) fn settled(
    record: &str,
    content: &str,
    given: &[AssetPayload],
    kept: &[AssetPayload],
) -> Result<Vec<AssetPayload>, EngineError> {
    for (index, payload) in given.iter().enumerate() {
        if given[..index]
            .iter()
            .any(|earlier| earlier.name == payload.name)
        {
            return Err(EngineError::AssetGivenTwice {
                record: record.to_owned(),
                asset: payload.name.to_string(),
            });
        }
    }
    let referenced = asset_references(content);
    if let Some(unreferenced) = given
        .iter()
        .find(|payload| !referenced.contains(&payload.name))
    {
        return Err(EngineError::AssetNotReferenced {
            record: record.to_owned(),
            asset: unreferenced.name.to_string(),
        });
    }
    referenced
        .iter()
        .map(|name| {
            given
                .iter()
                .chain(kept)
                .find(|payload| &payload.name == name)
                .cloned()
                .ok_or_else(|| EngineError::AssetNotGiven {
                    record: record.to_owned(),
                    asset: name.to_string(),
                })
        })
        .collect()
}

/// What a write of `assets` sends a destination whose record already records `recorded`: every
/// payload, with the bytes of each whose SHA-256 the record already records for that name
/// left out, because the destination already serves exactly those.
pub(super) fn write_of(assets: Vec<AssetPayload>, recorded: Option<AssetUploads>) -> AssetWrite {
    let assets = assets
        .into_iter()
        .map(|payload| {
            let served = recorded.as_ref().is_some_and(|recorded| {
                recorded.reusable(&payload.name, &payload.sha256).is_some()
            });
            AssetPayload {
                bytes: if served { None } else { payload.bytes },
                ..payload
            }
        })
        .collect();
    AssetWrite {
        assets,
        recorded_assets: recorded,
    }
}

/// What a record's `onetaskgraph.assets` says its source serves, read and checked.
///
/// # Errors
///
/// [`SourceError::Malformed`] naming the key when the record holds something that is not an
/// upload record: a source that wrote one it cannot vouch for is refused rather than read as
/// holding none, which would send every asset it serves again.
pub(super) fn recorded(
    metadata: &BTreeMap<String, Value>,
) -> Result<Option<AssetUploads>, SourceError> {
    AssetUploads::read(metadata).map_err(|message| SourceError::Malformed { message })
}

/// Refuse writing `record`, which carries `asset`, to a source whose plugin stores no assets.
pub(super) fn stores(
    source: &ResolvedSource,
    record: &str,
    asset: &AssetName,
) -> Result<(), EngineError> {
    if source.source().capabilities().assets.is_native() {
        return Ok(());
    }
    Err(EngineError::AssetsUnsupported {
        name: source.name().to_string(),
        kind: source.kind().to_owned(),
        record: record.to_owned(),
        asset: asset.to_string(),
    })
}

/// `listed` in the order `content` first references each asset, any it does not reference
/// after those by name.
pub(super) fn ordered(mut listed: Vec<Asset>, content: Option<&str>) -> Vec<Asset> {
    let referenced = asset_references(content.unwrap_or_default());
    listed.sort_by(|left, right| {
        let at = |asset: &Asset| {
            referenced
                .iter()
                .position(|name| name == &asset.name)
                .unwrap_or(usize::MAX)
        };
        at(left)
            .cmp(&at(right))
            .then_with(|| left.name.cmp(&right.name))
    });
    listed
}

/// Whether `held` — what a record holds — is exactly `carried`, name for name and digest for
/// digest.
pub(super) fn same_set(held: &[Asset], carried: &[AssetPayload]) -> bool {
    held.len() == carried.len()
        && carried.iter().all(|payload| {
            held.iter()
                .any(|asset| asset.name == payload.name && asset.sha256 == payload.sha256)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(name: &str, bytes: &[u8]) -> AssetPayload {
        AssetPayload::of(AssetName::new(name).expect("a name"), bytes.to_vec())
    }

    #[test]
    fn a_write_carries_what_the_content_references_in_order_and_refuses_the_rest() {
        let content = "![b](./b.png) ![a](./a.gif) ![b](./b.png)";
        let carried = settled(
            "task",
            content,
            &[payload("a.gif", b"a")],
            &[payload("b.png", b"b")],
        )
        .expect("settles");
        assert_eq!(
            carried.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["b.png", "a.gif"]
        );
        assert!(matches!(
            settled("task", content, &[payload("a.gif", b"a")], &[]),
            Err(EngineError::AssetNotGiven { asset, .. }) if asset == "b.png"
        ));
        assert!(matches!(
            settled("task", "none", &[payload("a.gif", b"a")], &[]),
            Err(EngineError::AssetNotReferenced { asset, .. }) if asset == "a.gif"
        ));
        assert!(matches!(
            settled("task", content, &[payload("a.gif", b"1"), payload("a.gif", b"2")], &[]),
            Err(EngineError::AssetGivenTwice { asset, .. }) if asset == "a.gif"
        ));
    }
}
