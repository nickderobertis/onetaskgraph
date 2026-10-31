//! This folder's exclusive, cross-process hold on one record's file, and the byte-for-byte
//! snapshot a copy that takes everything back restores it from — what
//! [`TaskSource::hold_item`](onetaskgraph_plugin_api::TaskSource::hold_item) and
//! [`TaskSource::restore_item`](onetaskgraph_plugin_api::TaskSource::restore_item) answer here.
//!
//! The hold is a lock file beside the record, `<file>.lock`, and what holds it is the kernel:
//! - **Unix**: an exclusive `flock` on that file, taken without blocking and asked again until it
//!   is free. The lock goes with the process however it ends, killed included, so a dead holder's
//!   file is taken at once and nothing is judged by its age. Once taken, the file is confirmed to
//!   be the one the path still names — a holder unlinks it as it lets go, while another process may
//!   already have opened it — and it is unlinked before the lock is released.
//! - **Windows**: the file opened with no sharing at all and deleted when its handle closes, which
//!   the kernel does for a killed process too; a second opener is refused with a sharing violation
//!   until then.
//! - **Neither** — a filesystem that refuses `flock`: the file created exclusively (`O_EXCL`),
//!   removed on release, and one older than [`STALE`] taken to be a dead process's and taken over.
//!   That fallback is weaker: a holder that outlives [`STALE`] can be taken over, and a lock dated
//!   ahead of this host's clock, whose age cannot be told, is refused at once.
//!
//! A live holder is waited for, up to twice [`STALE`]. None of these excludes anything across
//! hosts, or on a network filesystem that does not honour the lock.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::SourceError;

/// How long a fallback lock file may stand before it is taken to be a dead process's: far longer
/// than the read and the one metadata write it guards. The kernel's locks are never judged by it.
pub(crate) const STALE: Duration = Duration::from_secs(30);

/// One lock this source holds.
#[derive(Debug)]
enum Held {
    /// The kernel's lock on the open file at the path, released as the file closes.
    Locked(File, PathBuf),
    /// A fallback lock file this source created exclusively, released by removing it.
    Created(PathBuf),
}

/// The locks this source holds, by the record each guards — not by its file, so a record
/// deleted or moved while held is still released — shared by every clone of the source, so a
/// clone releases what another took.
#[derive(Debug, Clone, Default)]
pub(crate) struct Holds(Arc<Mutex<HashMap<String, Held>>>);

fn lock_of(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_owned();
    path.push(".lock");
    PathBuf::from(path)
}

fn could_not_lock(lock: &Path, error: &std::io::Error) -> SourceError {
    SourceError::Unavailable {
        message: format!("could not lock {}: {error}", lock.display()),
    }
}

fn still_held(lock: &Path) -> SourceError {
    SourceError::Unavailable {
        message: format!(
            "{} is held by another process that has not let go of it; next: let it finish, or \
             remove the lock file once no copy is running",
            lock.display()
        ),
    }
}

/// What one attempt at the kernel's lock found.
enum Attempt {
    Taken(File),
    Busy,
    /// This filesystem offers no such lock: the exclusive-create fallback decides instead, told
    /// whether this attempt created the file itself.
    #[cfg_attr(
        windows,
        expect(
            dead_code,
            reason = "Windows always has a share-none open, so it never falls back"
        )
    )]
    Unsupported {
        created: bool,
    },
}

/// Open the lock file, with the flags the kernel's lock on this platform needs.
fn open(lock: &Path, create_new: bool) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // GENERIC_READ | GENERIC_WRITE | DELETE, shared with nobody, deleted as it closes.
        const ACCESS: u32 = 0x8000_0000 | 0x4000_0000 | 0x0001_0000;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        options
            .access_mode(ACCESS)
            .share_mode(0)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
    }
    options.open(lock)
}

#[cfg(windows)]
fn attempt(lock: &Path) -> Result<Attempt, SourceError> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match open(lock, false) {
        Ok(file) => Ok(Attempt::Taken(file)),
        Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Ok(Attempt::Busy),
        Err(error) => Err(could_not_lock(lock, &error)),
    }
}

#[cfg(not(windows))]
fn attempt(lock: &Path) -> Result<Attempt, SourceError> {
    use std::os::unix::fs::MetadataExt;
    let (file, created) = match open(lock, true) {
        Ok(file) => (file, true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match open(lock, false) {
                Ok(file) => (file, false),
                // Unlinked by its holder between the two opens: ask again.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Attempt::Busy);
                }
                Err(error) => return Err(could_not_lock(lock, &error)),
            }
        }
        Err(error) => return Err(could_not_lock(lock, &error)),
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(Attempt::Busy),
        Err(std::fs::TryLockError::Error(_)) => return Ok(Attempt::Unsupported { created }),
    }
    // The file locked is the lock only while the path still names it: a holder unlinks it before
    // it lets go, so one opened just before that is a file nobody else will ever ask for.
    let opened = file
        .metadata()
        .map_err(|error| could_not_lock(lock, &error))?;
    match std::fs::metadata(lock) {
        Ok(named) if named.dev() == opened.dev() && named.ino() == opened.ino() => {
            Ok(Attempt::Taken(file))
        }
        Ok(_) => Ok(Attempt::Busy),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Attempt::Busy),
        Err(error) => Err(could_not_lock(lock, &error)),
    }
}

impl Holds {
    /// Wait for and take the lock beside `file`, held as the record `record` names.
    pub(crate) fn take(&self, record: &str, file: &Path) -> Result<(), SourceError> {
        self.take_within(record, file, STALE * 2)
    }

    /// [`Self::take`], waiting for a live holder no longer than `patience`.
    fn take_within(
        &self,
        record: &str,
        file: &Path,
        patience: Duration,
    ) -> Result<(), SourceError> {
        let lock = lock_of(file);
        let started = Instant::now();
        let held = loop {
            match attempt(&lock)? {
                Attempt::Taken(file) => break Held::Locked(file, lock),
                Attempt::Unsupported { created: true } => break Held::Created(lock),
                Attempt::Unsupported { created: false } => {
                    take_created(&lock, started, patience)?;
                    break Held::Created(lock);
                }
                Attempt::Busy if started.elapsed() > patience => return Err(still_held(&lock)),
                Attempt::Busy => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(record.to_owned(), held);
        Ok(())
    }

    /// Release the lock this source holds for `record`, wherever its file now is.
    pub(crate) fn release(&self, record: &str) {
        let held = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(record);
        match held {
            // Unlinked while still locked, so nobody can take a file the path no longer names;
            // on Windows closing it deletes it.
            Some(Held::Locked(file, lock)) => {
                #[cfg(not(windows))]
                let _ = std::fs::remove_file(&lock);
                #[cfg(windows)]
                let _ = lock;
                drop(file);
            }
            Some(Held::Created(lock)) => {
                let _ = std::fs::remove_file(lock);
            }
            None => {}
        }
    }
}

/// The exclusive-create fallback, on a filesystem with no kernel lock: create `lock` exclusively,
/// taking over one older than [`STALE`], refusing one dated ahead of this host's clock, and waiting
/// for a live holder no longer than `patience` from `started`.
fn take_created(lock: &Path, started: Instant, patience: Duration) -> Result<(), SourceError> {
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock)
        {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let modified = std::fs::metadata(lock).and_then(|held| held.modified());
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
                            "{} was written at a time ahead of this host's clock, so whether its \
                             holder is still running cannot be told; next: remove it once no \
                             copy is running",
                            lock.display()
                        ),
                    });
                }
                let stale = modified
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > STALE);
                if stale {
                    // Taken over by removing it; one this process may not remove is refused
                    // rather than tried again for ever.
                    match std::fs::remove_file(lock) {
                        Ok(()) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(SourceError::Unavailable {
                                message: format!(
                                    "{} is a dead process's lock that could not be removed: \
                                     {error}; next: remove it once no copy is running",
                                    lock.display()
                                ),
                            });
                        }
                    }
                }
                if started.elapsed() > patience {
                    return Err(still_held(lock));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(could_not_lock(lock, &error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> (tempfile::TempDir, PathBuf) {
        let folder = tempfile::tempdir().expect("a folder");
        let file = folder.path().join("T-1.md");
        std::fs::write(&file, "").expect("a record");
        (folder, file)
    }

    /// The wait's bound, with a patience short enough to reach: in use it is a minute. A second
    /// open of the lock file is excluded by the kernel exactly as another process's is. Everything
    /// else about the hold is driven through the plugin, in `tests/item_hold.rs`, and with real
    /// processes in the end-to-end suite.
    #[test]
    fn a_live_holder_waited_on_past_the_patience_is_refused() {
        let (_folder, file) = record();
        let holder = Holds::default();
        holder.take("task T-1", &file).expect("held");
        let refused = Holds::default()
            .take_within("task T-1", &file, Duration::from_millis(50))
            .expect_err("still held");
        assert!(
            matches!(&refused, SourceError::Unavailable { message }
                if message.contains("has not let go")),
            "{refused:?}"
        );
        assert!(lock_of(&file).exists(), "the holder's lock is left");
        holder.release("task T-1");
        assert!(!lock_of(&file).exists(), "released, the lock file goes");
        Holds::default()
            .take_within("task T-1", &file, Duration::from_millis(50))
            .expect("free once released");
    }

    /// A lock file nobody holds is taken at once whatever its date: its age is never asked.
    #[test]
    fn a_lock_file_nobody_holds_is_taken_at_once_whatever_its_date() {
        let (_folder, file) = record();
        for offset in [
            std::time::SystemTime::now() + Duration::from_secs(3600),
            std::time::SystemTime::now(),
        ] {
            let left = File::create(lock_of(&file)).expect("a dead holder's file");
            left.set_modified(offset).expect("dated");
            drop(left);
            let holds = Holds::default();
            holds
                .take_within("task T-1", &file, Duration::from_millis(50))
                .expect("taken at once");
            holds.release("task T-1");
            assert!(!lock_of(&file).exists());
        }
    }

    fn dated(lock: &Path, at: std::time::SystemTime) {
        let file = File::create(lock).expect("a lock file");
        file.set_modified(at).expect("dated");
    }

    /// The fallback a filesystem with no kernel lock gets: one older than [`STALE`] is taken over,
    /// one dated ahead of this clock is refused at once, and a live one is waited on.
    #[test]
    fn the_exclusive_create_fallback_judges_a_lock_by_its_age() {
        let (_folder, file) = record();
        let lock = lock_of(&file);
        dated(&lock, std::time::SystemTime::now() - STALE * 2);
        take_created(&lock, Instant::now(), Duration::from_millis(50)).expect("taken over");
        assert!(lock.exists(), "taken over, it is this one's now");

        dated(
            &lock,
            std::time::SystemTime::now() + Duration::from_secs(3600),
        );
        let refused = take_created(&lock, Instant::now(), Duration::from_secs(60))
            .expect_err("its age cannot be told");
        assert!(
            matches!(&refused, SourceError::Unavailable { message }
                if message.contains("ahead of this host's clock")),
            "{refused:?}"
        );
        assert!(lock.exists(), "another host's lock is left where it is");

        dated(&lock, std::time::SystemTime::now());
        let refused = take_created(&lock, Instant::now(), Duration::from_millis(50))
            .expect_err("a live holder");
        assert!(
            matches!(&refused, SourceError::Unavailable { message }
                if message.contains("has not let go")),
            "{refused:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_fallback_refuses_a_dead_lock_it_cannot_remove_rather_than_trying_for_ever() {
        use std::os::unix::fs::PermissionsExt;
        let (folder, file) = record();
        let lock = lock_of(&file);
        dated(&lock, std::time::SystemTime::now() - STALE * 2);
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o555))
            .expect("read-only");
        let refused = take_created(&lock, Instant::now(), Duration::from_secs(60));
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o755))
            .expect("writable");
        assert!(
            matches!(&refused, Err(SourceError::Unavailable { message })
                if message.contains("could not be removed")),
            "{refused:?}"
        );
        assert!(lock.exists(), "the lock is left where it is");
    }
}
