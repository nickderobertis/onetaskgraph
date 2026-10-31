//! A create that is safe to retry.
//!
//! Every create a copy makes is preceded by an **intent** recorded on the item being copied,
//! at [`MetadataKey::CREATING_KEY`]: the create's deterministic key, when it was first taken,
//! which copy attempt holds it and until when. The created item carries the same key, at
//! [`MetadataKey::CREATE_KEY`] — or as its own id, on a source that takes an id of the caller's
//! choosing. The intent is removed in the write that records the item's
//! [`MetadataKey::COPIES_KEY`] link, so an item with no create in flight carries none.
//!
//! A copy that finds an intent for its destination resolves it before every other rule: it asks
//! the destination for the item carrying the key ([`TaskSource::created_with_key`]) and, found,
//! finishes and links that one instead of creating a second. A destination whose lookup by key
//! lags a create is given until the source's quiescence window after the intent was first taken
//! to list it, and only then is created in again, with the same key.
//!
//! Who owns an intent depends on the copy's source. An item whose source reports a path on this
//! machine is taken, renewed and released under an exclusive lock file beside it, so exactly one
//! attempt on one host owns a create. A hosted source offers no lock, so its intent is advisory:
//! a live lease another attempt holds still refuses, but two attempts that read before either
//! wrote both proceed, and a destination whose lookup lags is listed again by key after the
//! create so a duplicate is reported rather than linked.
//!
//! [`TaskSource::created_with_key`]: onetaskgraph_plugin_api::TaskSource::created_with_key

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use onetaskgraph_plugin_api::{Location, MetadataKey, NativeId, SourceName, Task, TaskUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GlobalId;
use crate::resolve::ResolvedSource;

use super::super::{Engine, EngineError};
use super::refused;

/// The namespace every create key is a version-5 UUID under: the UUID this store names its
/// creates by, documented in the README so another implementation computes the same keys.
pub const NAMESPACE: uuid::Uuid = uuid::Uuid::from_u128(0x7a1c_3f0e_5b2d_4c8a_9e61_0f4d_2b7c_5a39);

/// How far before an intent's `at` a lookup by key reaches, for a destination clock that runs
/// behind this host's.
pub const CLOCK_SKEW: TimeDelta = TimeDelta::seconds(300);

/// How long a lock file beside an item may stand before it is taken to be a dead process's.
const STALE_LOCK: std::time::Duration = std::time::Duration::from_secs(30);

/// The key of the create that copies `source` into `destination`, replacing the link `replaces`
/// names when it replaces one: the same for every attempt at that create, and a different one
/// for a `--recreate` after a stale link.
#[must_use]
pub fn key(source: &GlobalId, destination: &SourceName, replaces: Option<&GlobalId>) -> uuid::Uuid {
    let name = format!(
        "{source}|{destination}|{}",
        replaces.map(ToString::to_string).unwrap_or_default()
    );
    uuid::Uuid::new_v5(&NAMESPACE, name.as_bytes())
}

/// One destination's entry of an item's `onetaskgraph.creating`, each member read as what it
/// is: a key and an attempt that are UUIDs, and two RFC 3339 UTC times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Intent {
    /// The create's key.
    pub(crate) key: uuid::Uuid,
    /// When the key was first taken, to the second.
    #[serde(with = "seconds")]
    pub(crate) at: DateTime<Utc>,
    /// The copy attempt holding it.
    pub(crate) attempt: uuid::Uuid,
    /// Until when it holds it, to the second.
    #[serde(with = "seconds")]
    pub(crate) lease_until: DateTime<Utc>,
}

impl Intent {
    fn at(&self) -> Option<DateTime<Utc>> {
        Some(self.at)
    }

    /// The key as the created item carries it.
    pub(crate) fn key(&self) -> String {
        self.key.hyphenated().to_string()
    }

    /// Whether another attempt than `attempt` holds this intent's lease at `now`.
    fn held_by_another(&self, attempt: uuid::Uuid, now: DateTime<Utc>) -> bool {
        self.attempt != attempt && self.lease_until > now
    }
}

/// A fresh version-4 UUID naming one engine's copy attempts.
///
/// Its random bits come from the standard library's per-process randomly keyed hasher, mixed
/// with this process's id and the time, rather than from a random-number crate this workspace
/// would otherwise carry twice: an attempt only has to differ from every other process's.
#[must_use]
pub fn attempt() -> uuid::Uuid {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        hasher.write_u32(std::process::id());
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// An intent's times as RFC 3339 UTC to the second, and read back only as that.
mod seconds {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        when: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&when.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        let spelled = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&spelled)
            .map(|when| when.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

/// `when`, as an intent spells a time.
pub(crate) fn seconds_of(when: DateTime<Utc>) -> String {
    when.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Now, to the second, as an intent records a time.
fn now() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp(now.timestamp(), 0).unwrap_or(now)
}

/// The intent `metadata` records for `destination`, or why the entry there is not one: a
/// malformed entry is refused rather than read as none, since reading it as none would let a
/// second create through.
pub(crate) fn held(
    metadata: &BTreeMap<String, Value>,
    destination: &SourceName,
) -> Result<Option<Intent>, String> {
    let Some(entry) = metadata
        .get(MetadataKey::CREATING_KEY)
        .and_then(|creating| creating.get(destination.as_str()))
    else {
        return Ok(None);
    };
    serde_json::from_value(entry.clone())
        .map(Some)
        .map_err(|error| {
            format!(
                "its {} entry for {destination} is not an intent to create — {error}",
                MetadataKey::CREATING_KEY
            )
        })
}

/// `onetaskgraph.creating` as `held` holds it, with `destination`'s entry replaced by `intent`
/// — or removed, for `None` — and `None` when nothing is left in it.
pub(crate) fn replaced(
    held: Option<&Value>,
    destination: &SourceName,
    intent: Option<&Intent>,
) -> Option<Value> {
    let mut entries = held.and_then(Value::as_object).cloned().unwrap_or_default();
    match intent {
        Some(intent) => {
            entries.insert(
                destination.to_string(),
                serde_json::to_value(intent).unwrap_or(Value::Null),
            );
        }
        None => {
            entries.remove(destination.as_str());
        }
    }
    (!entries.is_empty()).then_some(Value::Object(entries))
}

/// An exclusive lock beside one item's file on this machine, removed when dropped.
///
/// Taken by creating `<file>.lock` exclusively, which every platform's filesystem refuses a
/// second process, and released by removing it. A lock older than a few seconds' work is a
/// dead process's, and is taken over. It excludes nothing across hosts or on a network
/// filesystem that does not honour an exclusive create.
pub(crate) struct ItemLock {
    path: PathBuf,
}

impl ItemLock {
    /// Wait for and take the lock beside `item`.
    fn take(item: &Path) -> std::io::Result<Self> {
        let mut path = item.as_os_str().to_owned();
        path.push(".lock");
        let path = PathBuf::from(path);
        let started = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|held| held.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > STALE_LOCK * 2 {
                        return Err(error);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for ItemLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The test-only point a copy stops at by exiting, as a process killed there would:
/// `OTG_TEST_STOP_AT` naming `after-intent` or `before-link`. Unset, as everywhere but
/// the journeys that prove a retry, it does nothing.
pub(crate) fn stop_at(point: &str) {
    if std::env::var("OTG_TEST_STOP_AT").is_ok_and(|named| named == point) {
        std::process::exit(86);
    }
}

impl Engine {
    /// The settings of `source` for a create safe to retry.
    fn create_settings(&self, source: &SourceName) -> crate::config::CreateSettings {
        self.creates.get(source).copied().unwrap_or_default()
    }

    /// The task `id` as its source holds it now, and the lock beside its file when it has one on
    /// this machine — the read an intent is decided on.
    async fn locked_read(
        &self,
        source: &ResolvedSource,
        id: &GlobalId,
    ) -> Result<(Task, Option<ItemLock>), EngineError> {
        let task = source
            .source()
            .get_task(&id.native)
            .await
            .map_err(|error| refused(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        let lock = match &task.location {
            Some(Location::Path(path)) => {
                Some(ItemLock::take(Path::new(path)).map_err(|error| {
                    refused(
                        source,
                        onetaskgraph_plugin_api::SourceError::Unavailable {
                            message: format!("could not lock {path} to record a create: {error}"),
                        },
                    )
                })?)
            }
            _ => None,
        };
        // Read again under the lock, so what is decided is what no other process can change.
        let task = match lock {
            Some(_) => source
                .source()
                .get_task(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?,
            None => task,
        };
        Ok((task, lock))
    }

    /// The intent `task` records for `destination`, or the refusal of an entry that is not one.
    pub(crate) fn held_or_refused(
        &self,
        source: &ResolvedSource,
        id: &GlobalId,
        task: &Task,
        destination: &SourceName,
    ) -> Result<Option<Intent>, EngineError> {
        held(&task.metadata, destination).map_err(|problem| {
            refused(
                source,
                onetaskgraph_plugin_api::SourceError::Malformed {
                    message: format!(
                        "{id}: {problem}; next: remove that entry once no copy of {id} is running"
                    ),
                },
            )
        })
    }

    /// Whether `id`'s source keeps it on this machine, where an intent is lock-guarded.
    pub(crate) fn is_local(task: &Task) -> bool {
        matches!(task.location, Some(Location::Path(_)))
    }

    /// Take, or take over, the intent to create `id`'s counterpart in `destination` under
    /// `key`, leased until `lease_until` — refusing as `create-in-progress` when another attempt
    /// holds a live lease, before anything is written.
    pub(crate) async fn take_intent(
        &self,
        id: &GlobalId,
        destination: &SourceName,
        key: uuid::Uuid,
        lease_until: Option<DateTime<Utc>>,
    ) -> Result<Intent, EngineError> {
        let source = self.writable(&id.source)?;
        let (task, _lock) = self.locked_read(source, id).await?;
        let now = now();
        let held = self.held_or_refused(source, id, &task, destination)?;
        if let Some(held) = &held
            && held.held_by_another(self.attempt, now)
        {
            return Err(EngineError::CreateInProgress {
                item: id.to_string(),
                until: seconds_of(held.lease_until),
            });
        }
        let at = held
            .as_ref()
            .filter(|held| held.key == key)
            .map_or(now, |held| held.at);
        let intent = Intent {
            key,
            at,
            attempt: self.attempt,
            lease_until: lease_until.unwrap_or(now + self.create_settings(&id.source).lease()),
        };
        let value = replaced(
            task.metadata.get(MetadataKey::CREATING_KEY),
            destination,
            Some(&intent),
        )
        .unwrap_or(Value::Null);
        source
            .source()
            .set_task_metadata(&id.native, &MetadataKey::creating(), &value)
            .await
            .map_err(|error| refused(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        Ok(intent)
    }

    /// Renew this attempt's intent to create `id`'s counterpart in `destination`, before a
    /// destination write it sends after its create — refusing as `create-in-progress`, and
    /// writing nothing more, once the intent no longer names this attempt.
    pub(crate) async fn renew_intent(
        &self,
        id: &GlobalId,
        destination: &SourceName,
        until: Option<DateTime<Utc>>,
    ) -> Result<(), EngineError> {
        let source = self.writable(&id.source)?;
        let (task, _lock) = self.locked_read(source, id).await?;
        let Some(mut held) = self.held_or_refused(source, id, &task, destination)? else {
            return Ok(());
        };
        if held.attempt != self.attempt {
            return Err(EngineError::CreateInProgress {
                item: id.to_string(),
                until: seconds_of(held.lease_until),
            });
        }
        held.lease_until = until.unwrap_or(now() + self.create_settings(&id.source).lease());
        let value = replaced(
            task.metadata.get(MetadataKey::CREATING_KEY),
            destination,
            Some(&held),
        )
        .unwrap_or(Value::Null);
        source
            .source()
            .set_task_metadata(&id.native, &MetadataKey::creating(), &value)
            .await
            .map_err(|error| refused(source, error))?;
        Ok(())
    }

    /// Whether a create carrying `intent`'s key that got no answer may have landed in
    /// `destination`: asked of a destination whose lookup by key is current, and assumed of
    /// one whose lookup lags or cannot be made.
    pub(crate) async fn may_have_landed(
        &self,
        destination: &SourceName,
        intent: Option<&Intent>,
    ) -> bool {
        let (Some(intent), Ok(destination)) = (intent, self.writable(destination)) else {
            return true;
        };
        if destination.source().create_key_lags() {
            return true;
        }
        self.relisted(destination, intent, &[])
            .await
            .map_or(true, |found| !found.is_empty())
    }

    /// Put a file on this machine back to `bytes`, under its lock: what a copy that took every
    /// write back leaves of an item whose intent it recorded.
    pub(crate) fn restore_snapshot(&self, path: &Path, bytes: &[u8]) {
        if let Ok(_lock) = ItemLock::take(path) {
            let _ = std::fs::write(path, bytes);
        }
    }

    /// Remove this attempt's intent to create `id`'s counterpart in `destination`, once a copy
    /// that could not finish took back everything it wrote: nothing was made to resolve.
    pub(crate) async fn drop_intent(
        &self,
        id: &GlobalId,
        destination: &SourceName,
    ) -> Result<(), EngineError> {
        let source = self.writable(&id.source)?;
        let (task, _lock) = self.locked_read(source, id).await?;
        if held(&task.metadata, destination)
            .ok()
            .flatten()
            .is_none_or(|held| held.attempt != self.attempt)
        {
            return Ok(());
        }
        let mut update = TaskUpdate::default();
        match replaced(
            task.metadata.get(MetadataKey::CREATING_KEY),
            destination,
            None,
        ) {
            Some(rest) => {
                update.metadata_set.insert(MetadataKey::creating(), rest);
            }
            None => {
                update.metadata_remove.insert(MetadataKey::creating());
            }
        }
        source
            .source()
            .update_task(&id.native, &update)
            .await
            .map_err(|error| refused(source, error))?;
        Ok(())
    }

    /// Record `links` at `onetaskgraph.copies` on `id` and remove its intent for `destination`,
    /// in one write: the link write a copy already makes, carrying the removal with it.
    pub(crate) async fn link_settling_intent(
        &self,
        id: &GlobalId,
        destination: &SourceName,
        links: Value,
    ) -> Result<Option<Option<String>>, onetaskgraph_plugin_api::SourceError> {
        let source = self.writable(&id.source).map_err(|error| {
            onetaskgraph_plugin_api::SourceError::Refused {
                message: error.to_string(),
            }
        })?;
        let read = source.source().get_task(&id.native).await?;
        let Some(task) = read else {
            return Ok(None);
        };
        let _lock = match &task.location {
            Some(Location::Path(path)) => ItemLock::take(Path::new(path)).ok(),
            _ => None,
        };
        let task = match _lock {
            Some(_) => match source.source().get_task(&id.native).await? {
                Some(task) => task,
                None => return Ok(None),
            },
            None => task,
        };
        let mut update = TaskUpdate::default();
        update.metadata_set.insert(MetadataKey::copies(), links);
        match replaced(
            task.metadata.get(MetadataKey::CREATING_KEY),
            destination,
            None,
        ) {
            Some(rest) => {
                update.metadata_set.insert(MetadataKey::creating(), rest);
            }
            None => {
                update.metadata_remove.insert(MetadataKey::creating());
            }
        }
        let outcome = source.source().update_task(&id.native, &update).await?;
        Ok(outcome.map(|outcome| outcome.task.version))
    }

    /// Resolve an intent `metadata` records for `destination` before every other rule: the item
    /// the destination holds under its key, when exactly one does; a refusal naming every one
    /// when several do; and `None` when none does — once a destination whose lookup lags has
    /// had until the quiescence window after the intent was first taken to list it.
    ///
    /// `waits` is false for a dry run and for `task locate`, which never wait, renew or write.
    pub(crate) async fn resolve_intent(
        &self,
        destination: &ResolvedSource,
        id: &GlobalId,
        intent: &Intent,
        repositories: &[onetaskgraph_plugin_api::Repository],
        waits: bool,
    ) -> Result<Option<NativeId>, EngineError> {
        let since = intent.at().map(|at| at - CLOCK_SKEW);
        let mut waited = false;
        loop {
            let found = destination
                .source()
                .created_with_key(
                    onetaskgraph_plugin_api::MetadataRecord::Task,
                    &intent.key(),
                    since,
                    &onetaskgraph_plugin_api::WriteTarget::New {
                        repositories,
                        project: None,
                    },
                )
                .await
                .map_err(|error| refused(destination, error))?;
            match found.as_slice() {
                [one] => return Ok(Some(one.clone())),
                [] => {}
                several => {
                    return Err(EngineError::CreateDuplicated {
                        item: id.to_string(),
                        destination: destination.name().to_string(),
                        found: several
                            .iter()
                            .map(|native| {
                                GlobalId::new(destination.name().clone(), native.clone())
                                    .to_string()
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    });
                }
            }
            if waited || !waits || !destination.source().create_key_lags() {
                return Ok(None);
            }
            let Some(at) = intent.at() else {
                return Ok(None);
            };
            let window_end = at + self.create_settings(destination.name()).quiescence();
            let now = Utc::now();
            if now >= window_end {
                return Ok(None);
            }
            // Taken over by this attempt — refused while another's lease is live — and leased
            // once, to outlast the whole wait, so no renewal falls during it.
            self.take_intent(
                id,
                destination.name(),
                intent.key,
                Some(window_end + self.create_settings(&id.source).lease()),
            )
            .await?;
            let wait = (window_end - now).to_std().unwrap_or_default();
            self.clock.sleep(wait).await;
            waited = true;
        }
    }

    /// Every item `destination` lists under `key` once a create carrying it has been sent,
    /// for a destination whose lookup by key lags: one is the create, and several are a
    /// duplicate reported rather than linked.
    pub(crate) async fn relisted(
        &self,
        destination: &ResolvedSource,
        intent: &Intent,
        repositories: &[onetaskgraph_plugin_api::Repository],
    ) -> Result<Vec<NativeId>, EngineError> {
        destination
            .source()
            .created_with_key(
                onetaskgraph_plugin_api::MetadataRecord::Task,
                &intent.key(),
                intent.at().map(|at| at - CLOCK_SKEW),
                &onetaskgraph_plugin_api::WriteTarget::New {
                    repositories,
                    project: None,
                },
            )
            .await
            .map_err(|error| refused(destination, error))
    }
}
