//! This folder's exclusive, cross-process hold on one record's file, and the byte-for-byte
//! snapshot a copy that takes everything back restores it from — what
//! [`TaskSource::hold_item`](onetaskgraph_plugin_api::TaskSource::hold_item) and
//! [`TaskSource::restore_item`](onetaskgraph_plugin_api::TaskSource::restore_item) answer here.
//!
//! The hold is a lock file beside the record, `<file>.lock`, created exclusively — which every
//! platform's filesystem refuses a second process — and removed on release. One older than
//! [`STALE`] is a dead process's and is taken over; a live holder is waited for, up to twice
//! that; and one written at a time ahead of this host's clock, whose age cannot be told, is
//! refused at once. It excludes nothing across hosts, or on a network filesystem that does not
//! honour an exclusive create.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::SourceError;

/// How long a lock file may stand before it is taken to be a dead process's: far longer than
/// the read and the one metadata write it guards.
pub(crate) const STALE: Duration = Duration::from_secs(30);

/// The lock files this source holds, by the record file each guards — shared by every clone of
/// the source, so a clone releases what another took.
#[derive(Debug, Clone, Default)]
pub(crate) struct Holds(Arc<Mutex<HashMap<PathBuf, PathBuf>>>);

fn lock_of(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_owned();
    path.push(".lock");
    PathBuf::from(path)
}

impl Holds {
    /// Wait for and take the lock beside `file`.
    pub(crate) fn take(&self, file: &Path) -> Result<(), SourceError> {
        let lock = lock_of(file);
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
            {
                Ok(_) => {
                    self.0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(file.to_path_buf(), lock);
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let modified = std::fs::metadata(&lock).and_then(|held| held.modified());
                    // A lock written at a time ahead of this host's clock — another host's, on a
                    // shared folder — can never be judged stale here, so it is refused at once
                    // rather than waited on.
                    if let Ok(modified) = &modified
                        && modified.elapsed().is_err()
                        && modified
                            .duration_since(std::time::SystemTime::now())
                            .is_ok_and(|ahead| ahead > Duration::from_secs(1))
                    {
                        return Err(SourceError::Unavailable {
                            message: format!(
                                "{} was written at a time ahead of this host's clock, so whether \
                                 its holder is still running cannot be told; next: remove it \
                                 once no copy is running",
                                lock.display()
                            ),
                        });
                    }
                    let stale = modified
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > STALE);
                    if stale {
                        let _ = std::fs::remove_file(&lock);
                        continue;
                    }
                    if started.elapsed() > STALE * 2 {
                        return Err(SourceError::Unavailable {
                            message: format!(
                                "{} is held by another process that has not let go of it; next: \
                                 let it finish, or remove the lock file once no copy is running",
                                lock.display()
                            ),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    return Err(SourceError::Unavailable {
                        message: format!("could not lock {}: {error}", lock.display()),
                    });
                }
            }
        }
    }

    /// Release the lock beside `file`, when this source holds it.
    pub(crate) fn release(&self, file: &Path) {
        if let Some(lock) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(file)
        {
            let _ = std::fs::remove_file(lock);
        }
    }
}
