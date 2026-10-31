//! What a source asks before each write request it sends while the engine owns a create there.
//!
//! A copy that creates an item holds an intent on the item it copies, under a lease, until it
//! links the counterpart. Each destination write it sends after its create request has to renew
//! that lease, and to stop once another attempt holds it — including the several requests one
//! plugin call can send: a GitHub create is `createIssue` and then the card's fields, its
//! relationships and, should those fail, the delete that takes the issue back. The engine cannot
//! see those requests, so it installs a [`WriteGuard`] for the length of the call and the source
//! asks it, through [`<dyn WriteGuard>::check`], immediately before sending each one.
//!
//! The guard is task-local, so a source that never asks — every one that predates this — writes
//! exactly as it did, and one that asks outside a guarded call is answered at once.

use std::future::Future;
use std::sync::Arc;

use crate::SourceError;

tokio::task_local! {
    static GUARD: Arc<dyn WriteGuard>;
}

/// Renews the engine's hold on the creates a guarded call writes for, before each request.
#[async_trait::async_trait]
pub trait WriteGuard: Send + Sync {
    /// Renew, and answer whether the next write request may be sent: an error is the reason it
    /// may not — another attempt holds the create — and every later ask answers it again, so a
    /// source sends nothing more, not even a write taking back what it made.
    ///
    /// # Errors
    ///
    /// The refusal the source is to fail its call with, sending nothing more.
    async fn before_write(&self) -> Result<(), SourceError>;
}

impl dyn WriteGuard {
    /// Run `call` with `guard` answering every [`<dyn WriteGuard>::check`] made inside it.
    pub async fn scope<F: Future>(guard: Arc<dyn WriteGuard>, call: F) -> F::Output {
        GUARD.scope(guard, call).await
    }

    /// Ask the guard of the call in progress, if any, whether a write request may be sent now —
    /// a source calls this immediately before sending each write request but its create.
    ///
    /// # Errors
    ///
    /// The guard's refusal; with no guard installed, never.
    pub async fn check() -> Result<(), SourceError> {
        match GUARD.try_with(Arc::clone) {
            Ok(guard) => guard.before_write().await,
            Err(_) => Ok(()),
        }
    }
}
