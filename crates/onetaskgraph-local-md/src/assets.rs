//! Where a record's image assets are kept: a directory of their own beside the record's file.
//!
//! A task at `tasks/alpha/fix-login.md` keeps its assets in `tasks/alpha/fix-login.assets/`,
//! each under the name its content references it by — `![before](./before.png)` is
//! `tasks/alpha/fix-login.assets/before.png`. The directory is named after the record's own
//! file, and no two records share a file, so two records' assets never collide however their
//! assets are named. Nothing in it is a record: this source reads only `.md` files, and an
//! asset name ends in an image extension.
//!
//! The directory holds exactly the record's asset set. A write carrying assets replaces it —
//! writing each asset through a staging file and a rename, then removing every asset the
//! write does not name — and removing the record removes it. Only files whose names are asset
//! names are ever written or removed here, so anything else a person put in the directory is
//! left where it is.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use onetaskgraph_plugin_api::{Asset, AssetName, AssetWrite, SourceError, asset_sha256};

use crate::STAGING_SUFFIX;

/// The extension a record's asset directory carries in place of the record's `.md`.
pub(crate) const DIRECTORY_EXTENSION: &str = "assets";

/// Staging files written by this process so far, so two never share a name.
static STAGED: AtomicU64 = AtomicU64::new(0);

/// The directory holding the assets of the record whose file is `record`.
pub(crate) fn directory_of(record: &Path) -> PathBuf {
    record.with_extension(DIRECTORY_EXTENSION)
}

/// Every asset the record whose file is `record` holds, by name.
pub(crate) fn listed(record: &Path) -> Result<Vec<Asset>, SourceError> {
    let directory = directory_of(record);
    let mut assets = Vec::new();
    for (name, path) in held(&directory)? {
        let bytes = fs::read(&path).map_err(|e| SourceError::Unavailable {
            message: format!("cannot read {}: {e}", path.display()),
        })?;
        assets.push(Asset {
            sha256: asset_sha256(&bytes),
            content_type: name.content_type(),
            name,
            path: Some(path.to_string_lossy().into_owned()),
        });
    }
    Ok(assets)
}

/// The bytes of the asset `name` the record whose file is `record` holds, when it holds it.
pub(crate) fn bytes(record: &Path, name: &AssetName) -> Result<Option<Vec<u8>>, SourceError> {
    let path = directory_of(record).join(name.as_str());
    if !path.is_file() {
        return Ok(None);
    }
    fs::read(&path)
        .map(Some)
        .map_err(|e| SourceError::Unavailable {
            message: format!("cannot read {}: {e}", path.display()),
        })
}

/// Every asset of `write` with its bytes, checked before anything is written.
///
/// A payload carries its bytes, which must hash to its `sha256`; one without bytes is
/// accepted only when the record whose file is `existing` already holds that asset with that
/// digest, because this source records no upload a write could stand on otherwise.
pub(crate) fn resolved(
    existing: Option<&Path>,
    write: &AssetWrite,
) -> Result<Vec<(AssetName, Vec<u8>)>, SourceError> {
    let mut files = Vec::with_capacity(write.assets.len());
    for payload in &write.assets {
        if files.iter().any(|(name, _)| name == &payload.name) {
            return Err(SourceError::Refused {
                message: format!(
                    "the asset {} is given twice; next: give each asset once",
                    payload.name
                ),
            });
        }
        let bytes = match (&payload.bytes, existing) {
            (Some(bytes), _) => bytes.clone(),
            (None, Some(record)) => bytes(record, &payload.name)?
                .filter(|held| asset_sha256(held) == payload.sha256)
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "the asset {} carries no bytes and this record holds no asset of that \
                         name with sha256 {}; next: send its bytes",
                        payload.name, payload.sha256
                    ),
                })?,
            (None, None) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "the asset {} carries no bytes, and a record being created holds \
                         none to reuse; next: send its bytes",
                        payload.name
                    ),
                });
            }
        };
        if asset_sha256(&bytes) != payload.sha256 {
            return Err(SourceError::Refused {
                message: format!(
                    "the asset {}'s bytes do not hash to the sha256 {} it carries; next: send \
                     the bytes that digest names",
                    payload.name, payload.sha256
                ),
            });
        }
        files.push((payload.name.clone(), bytes));
    }
    Ok(files)
}

/// Make the record whose file is `record` hold exactly `files` as its assets.
pub(crate) fn replace(record: &Path, files: &[(AssetName, Vec<u8>)]) -> Result<(), SourceError> {
    let directory = directory_of(record);
    if !files.is_empty() {
        fs::create_dir_all(&directory).map_err(|e| SourceError::Unavailable {
            message: format!("cannot create {}: {e}", directory.display()),
        })?;
    }
    for (name, bytes) in files {
        let path = directory.join(name.as_str());
        if fs::read(&path).is_ok_and(|held| held == *bytes) {
            continue;
        }
        staged(&path, bytes)?;
    }
    for (name, path) in held(&directory)? {
        if !files.iter().any(|(kept, _)| kept == &name) {
            fs::remove_file(&path).map_err(|e| SourceError::Unavailable {
                message: format!("cannot remove {}: {e}", path.display()),
            })?;
        }
    }
    remove_if_empty(&directory)
}

/// Remove every asset of the record whose file was `record`, and the directory once empty.
pub(crate) fn remove(record: &Path) -> Result<(), SourceError> {
    replace(record, &[])
}

/// The assets in `directory`, by name: every file there whose name is an asset name.
fn held(directory: &Path) -> Result<Vec<(AssetName, PathBuf)>, SourceError> {
    let entries = match fs::read_dir(directory) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries.map_err(|e| SourceError::Unavailable {
            message: format!("cannot read {}: {e}", directory.display()),
        })?,
    };
    let mut held = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| SourceError::Unavailable {
            message: format!("cannot read an entry of {}: {e}", directory.display()),
        })?;
        let path = entry.path();
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| AssetName::new(name).ok())
        else {
            continue;
        };
        if path.is_file() {
            held.push((name, path));
        }
    }
    held.sort();
    Ok(held)
}

/// Remove `directory` when nothing is left in it.
fn remove_if_empty(directory: &Path) -> Result<(), SourceError> {
    let empty = match fs::read_dir(directory) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        entries => entries
            .map_err(|e| SourceError::Unavailable {
                message: format!("cannot read {}: {e}", directory.display()),
            })?
            .next()
            .is_none(),
    };
    if empty {
        fs::remove_dir(directory).map_err(|e| SourceError::Unavailable {
            message: format!("cannot remove {}: {e}", directory.display()),
        })?;
    }
    Ok(())
}

/// Write `bytes` at `path` through a staging file beside it and a rename; a staging file
/// whose write or rename fails is removed.
fn staged(path: &Path, bytes: &[u8]) -> Result<(), SourceError> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staging = path.with_file_name(format!(
        ".{name}.{}-{}{STAGING_SUFFIX}",
        std::process::id(),
        STAGED.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, path)
    })();
    written.map_err(|e| {
        let _ = fs::remove_file(&staging);
        SourceError::Unavailable {
            message: format!("cannot write {}: {e}", path.display()),
        }
    })
}
