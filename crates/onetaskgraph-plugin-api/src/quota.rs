//! What a source's backend says is left of the allowance it meters the source's credential by,
//! and the gate a caller installs to wait or stop between any two requests a call sends.
//!
//! [`Metering`](crate::Metering) is what this source *spent*; a [`QuotaReading`] is what the
//! backend says is *left*, which other work drawing on the same credential moves too. The store
//! reports the reading and decides nothing about it: what it means — a floor to keep, a pause to
//! take — is the caller's.
//!
//! A reading taken before a call of hundreds of requests cannot keep a caller's floor inside
//! it, so a caller that needs to pace inside one installs a [`RequestGate`]. A source asks it,
//! through [`<dyn RequestGate>::before`], immediately before each request it sends — reads,
//! writes, retries and a quota read alike — and tells it, through
//! [`<dyn RequestGate>::after`], what each response said is left. Like
//! [`WriteGuard`](crate::WriteGuard) the gate is task-local, installed by the engine for the
//! length of each call it makes to a source: a source that never asks — every one that
//! predates this — sends exactly what it sent, and one that asks outside a gated call is
//! answered at once.

use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SourceError;

/// What one response — or one read of the backend's own allowance endpoint — said about one
/// rate-limit window of the backend.
///
/// **One window, never merged.** A credential may be metered by several windows of the same
/// resource at once — GitHub was measured answering one token from two concurrent GraphQL
/// windows, each with its own reset, while `GET /rate_limit` reported only one of them — so a
/// source reports each reading as the response that carried it said it, with its own
/// [`resets_at`](Self::resets_at). Choosing the least of several is the caller's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuotaReading {
    /// The backend's own name for what is metered — GitHub's resource names (`graphql`,
    /// `core`, `search` and so on), Linear's `requests` and `complexity`.
    // llmlint: ignore[invalid_states_unrepresentable] An open vocabulary the backend names and the store interprets none of: the contract three other repositories build against states it as a plain string (`resource: String`), and GitHub adds resources (`code_search` arrived after `search`), so an enumeration here would refuse a reading for being new.
    pub resource: String,
    /// The whole allowance of the window.
    pub limit: u64,
    /// What was left of it when the backend answered.
    pub remaining: u64,
    /// What the credential had spent of it — everything that shares the credential, not only
    /// this source.
    pub used: u64,
    /// When the window resets and the allowance comes back.
    pub resets_at: DateTime<Utc>,
}

/// One request a source is about to send, as a [`RequestGate`] is asked about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GatedRequest {
    /// The resource the request draws on, in [`QuotaReading::resource`]'s vocabulary; `local`
    /// for a write a source makes to its own files.
    // llmlint: ignore[invalid_states_unrepresentable] The same open vocabulary as `QuotaReading::resource`, which a gate compares it against; the contract states it as a plain string.
    pub resource: String,
    /// Whether the request creates content — an issue, a comment — which a backend may meter
    /// by a limit of its own (GitHub's secondary content-creation limits).
    pub content_creating: bool,
    /// What the source expects the request to cost against `resource`, when it can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_hint: Option<u64>,
}

tokio::task_local! {
    static GATE: Arc<dyn RequestGate>;
}

/// What a caller installs to wait or stop between any two requests a store call sends.
///
/// It holds the caller's policy; the store holds none. **A wait** — `before_request` not yet
/// returned — holds that source's next request and nothing else: a source asks it inline, so
/// another source's call, and another call to the same engine, go on. **A refusal** —
/// `before_request` answering `Err` — makes the source send nothing more in that call: what
/// already landed is answered as it is today, the item in flight as a failure part way
/// through it, and every later item of a batch with the refusal, nothing sent for it.
///
/// **One request is exempt: a stdio peer's `initialize` handshake.** The engine sends it once,
/// while the engine is being built, before `Engine::with_request_gate` can install a gate, and
/// it costs no backend quota. Every method sent a peer after the gate is installed is asked of
/// it first.
#[async_trait::async_trait]
pub trait RequestGate: Send + Sync {
    /// Answer whether `request` may be sent now — after as long a wait as the caller's policy
    /// asks for — or why not.
    ///
    /// # Errors
    ///
    /// The refusal the source is to fail its call with, sending nothing more.
    async fn before_request(&self, request: &GatedRequest) -> Result<(), SourceError>;

    /// What the response to a request said about each window it named.
    fn after_response(&self, readings: &[QuotaReading]);
}

impl dyn RequestGate {
    /// Run `call` with `gate` answering every [`<dyn RequestGate>::before`] and
    /// [`<dyn RequestGate>::after`] made inside it.
    pub async fn scope<F: Future>(gate: Arc<dyn RequestGate>, call: F) -> F::Output {
        GATE.scope(gate, call).await
    }

    /// The gate of the call in progress, if any — what a call that spawns work of its own
    /// carries into it with [`Self::scope`].
    #[must_use]
    pub fn current() -> Option<Arc<dyn RequestGate>> {
        GATE.try_with(Arc::clone).ok()
    }

    /// Ask the gate of the call in progress, if any, whether `request` may be sent now — a
    /// source calls this immediately before sending each request.
    ///
    /// # Errors
    ///
    /// The gate's refusal; with no gate installed, never.
    pub async fn before(request: &GatedRequest) -> Result<(), SourceError> {
        match Self::current() {
            Some(gate) => gate.before_request(request).await,
            None => Ok(()),
        }
    }

    /// Tell the gate of the call in progress, if any, what a response said is left; with no
    /// gate installed, or no readings, nothing happens.
    pub fn after(readings: &[QuotaReading]) {
        if readings.is_empty() {
            return;
        }
        if let Some(gate) = Self::current() {
            gate.after_response(readings);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A gate recording what it was asked and told, refusing once it has been asked `refuse_at`
    /// times.
    struct Recording {
        asked: Mutex<Vec<GatedRequest>>,
        told: Mutex<Vec<QuotaReading>>,
        refuse_at: usize,
    }

    #[async_trait::async_trait]
    impl RequestGate for Recording {
        async fn before_request(&self, request: &GatedRequest) -> Result<(), SourceError> {
            let mut asked = self.asked.lock().unwrap();
            asked.push(request.clone());
            if asked.len() >= self.refuse_at {
                return Err(SourceError::Refused {
                    message: "the gate refused".into(),
                });
            }
            Ok(())
        }

        fn after_response(&self, readings: &[QuotaReading]) {
            self.told.lock().unwrap().extend_from_slice(readings);
        }
    }

    fn request(resource: &str) -> GatedRequest {
        GatedRequest {
            resource: resource.into(),
            content_creating: false,
            cost_hint: None,
        }
    }

    fn reading() -> QuotaReading {
        QuotaReading {
            resource: "graphql".into(),
            limit: 5_000,
            remaining: 4_000,
            used: 1_000,
            resets_at: DateTime::from_timestamp(1_795_306_927, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn with_no_gate_installed_every_request_may_be_sent_and_nothing_is_told() {
        assert!(<dyn RequestGate>::before(&request("graphql")).await.is_ok());
        <dyn RequestGate>::after(&[reading()]);
        assert!(<dyn RequestGate>::current().is_none());
    }

    #[tokio::test]
    async fn inside_a_scope_the_gate_is_asked_told_and_its_refusal_is_answered() {
        let gate = Arc::new(Recording {
            asked: Mutex::new(Vec::new()),
            told: Mutex::new(Vec::new()),
            refuse_at: 2,
        });
        let installed: Arc<dyn RequestGate> = gate.clone();
        let (first, second) = <dyn RequestGate>::scope(installed, async {
            let first = <dyn RequestGate>::before(&request("graphql")).await;
            <dyn RequestGate>::after(&[reading()]);
            <dyn RequestGate>::after(&[]);
            let second = <dyn RequestGate>::before(&request("core")).await;
            (first, second)
        })
        .await;
        assert!(first.is_ok());
        assert_eq!(
            second,
            Err(SourceError::Refused {
                message: "the gate refused".into()
            })
        );
        assert_eq!(
            *gate.asked.lock().unwrap(),
            [request("graphql"), request("core")]
        );
        assert_eq!(*gate.told.lock().unwrap(), [reading()]);
        // Outside the scope, nothing is asked.
        assert!(<dyn RequestGate>::before(&request("graphql")).await.is_ok());
        assert_eq!(gate.asked.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_reading_serialises_as_the_contract_states_it() {
        assert_eq!(
            serde_json::to_value(reading()).unwrap(),
            serde_json::json!({"resource":"graphql","limit":5000,"remaining":4000,
                               "used":1000,"resets_at":"2026-11-22T00:22:07Z"})
        );
    }
}
