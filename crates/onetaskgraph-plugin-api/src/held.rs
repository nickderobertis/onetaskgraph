//! A write held to the version of the task it writes: `--if-version`.
//!
//! The engine holds a write of one task to the version its caller read, and refuses the write
//! when the task is at any other. Where that check is made is the source's to say, through
//! [`TaskSource::holds_versions`](crate::TaskSource::holds_versions):
//!
//! - A source that serializes its own writes — a folder of Markdown, under the lock beside the
//!   task's file; an in-memory source, under the mutex over its work — makes the check and the
//!   write one step, so no competing write can land between them. It answers `true`, and reads
//!   the version the call is held to with [`HeldVersion::expected`] inside each task write,
//!   refusing a moved one with [`HeldVersion::moved`].
//! - A source whose backend offers no conditional write answers `false`, and the engine reads
//!   the task through it immediately before the write instead. The time between that read and
//!   the mutation is the backend's residual race, which each such plugin's documentation states.
//!
//! The hold is task-local, so a source that never asks writes exactly as it did, and one that
//! asks outside a held call is answered that nothing is held.

use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::{NativeId, SourceError};

tokio::task_local! {
    static HELD: Arc<HeldVersion>;
}

/// One task's write held to the version its caller read.
#[derive(Debug)]
pub struct HeldVersion {
    id: NativeId,
    version: String,
    /// The version the source found instead, once it has refused the write.
    moved: Mutex<Option<String>>,
}

impl HeldVersion {
    /// A write of the task `id` held to `version`.
    #[must_use]
    pub fn new(id: NativeId, version: String) -> Self {
        Self {
            id,
            version,
            moved: Mutex::new(None),
        }
    }

    /// Run `call` with this hold answering every [`HeldVersion::expected`] made inside it.
    pub async fn scope<F: Future>(held: Arc<Self>, call: F) -> F::Output {
        HELD.scope(held, call).await
    }

    /// The version the write in progress holds the task `id` to — `None` when no write in
    /// progress is held, or the one that is holds another task.
    #[must_use]
    pub fn expected(id: &NativeId) -> Option<String> {
        HELD.try_with(|held| (&held.id == id).then(|| held.version.clone()))
            .ok()
            .flatten()
    }

    /// Refuse the write in progress of the task `id`, which the source found at `found` rather
    /// than at the version it is held to: the refusal the source fails its call with, writing
    /// nothing, recorded so the engine reports it as the version having moved.
    #[must_use]
    pub fn moved(id: &NativeId, found: Option<String>) -> SourceError {
        let found = found.unwrap_or_default();
        let _ = HELD.try_with(|held| {
            if &held.id == id {
                *held
                    .moved
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(found.clone());
            }
        });
        SourceError::Refused {
            message: format!(
                "task {id} is at version {found:?}, not the version the write is held to, so \
                 nothing was written"
            ),
        }
    }

    /// The version the source found the task at when it refused the write, if it did.
    #[must_use]
    pub fn found(&self) -> Option<String> {
        self.moved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The task this hold is for.
    #[must_use]
    pub const fn id(&self) -> &NativeId {
        &self.id
    }

    /// The version the write is held to.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hold_answers_only_inside_its_scope_and_only_for_its_task() {
        let id = NativeId::from("t1");
        assert_eq!(HeldVersion::expected(&id), None);
        let held = Arc::new(HeldVersion::new(id.clone(), "v1".to_owned()));
        let inside = HeldVersion::scope(Arc::clone(&held), async {
            let other = HeldVersion::expected(&NativeId::from("t2"));
            let refused = HeldVersion::moved(&id, Some("v2".to_owned()));
            (HeldVersion::expected(&id), other, refused)
        })
        .await;
        assert_eq!(inside.0.as_deref(), Some("v1"));
        assert_eq!(inside.1, None);
        assert!(matches!(inside.2, SourceError::Refused { .. }));
        assert_eq!(held.found().as_deref(), Some("v2"));
    }
}
