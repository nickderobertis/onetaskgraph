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
//! writes, retries and a quota read alike — tells it, through
//! [`<dyn RequestGate>::after`], what each response said is left, and holds the
//! [`AdmittedRequest`] that ask answered until the request is over, whose drop tells the gate
//! so through [`RequestGate::request_ended`]. Like
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
///
/// **Every admitted request ends, exactly once.** For each `before_request` that answered `Ok`
/// the gate is told [`request_ended`](Self::request_ended) once, whatever became of the
/// request — answered with readings or without, failed in transport before any header arrived,
/// a write to a source's own files, or its future dropped mid-flight by a cancellation or by a
/// sibling failing beside it. A request `before_request` refused is never ended, and neither is
/// one whose `before_request` future was itself dropped before it answered: a gate that takes
/// something inside `before_request` gives it back on its own drop path there.
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

    /// One request this gate admitted is over: nothing more of it will be sent or read, so a
    /// gate may treat this as proof that it is no longer in flight — a slot it keeps per
    /// request in flight can be given back. Delivered exactly once per admitted request, after
    /// any [`after_response`](Self::after_response) its response carried, and never for a
    /// request `before_request` refused. A gate that keeps no such count needs nothing here.
    fn request_ended(&self) {}
}

/// A request a [`RequestGate`] admitted, held by the source for as long as the request is in
/// flight: dropping it — on every exit, a cancelled or failed one included — tells the gate the
/// request is over through [`RequestGate::request_ended`], exactly once.
///
/// Bind it (`let admitted = …`) and drop it once the response has been read, or the request
/// failed; never hold it across a wait before a retry or across another request's ask, since a
/// gate admitting one request at a time would then wait on itself.
#[must_use = "dropping the admission at once tells the gate the request is over before it is sent"]
pub struct AdmittedRequest {
    gate: Option<Arc<dyn RequestGate>>,
}

impl AdmittedRequest {
    /// End the request now, rather than when the admission goes out of scope.
    pub fn end(self) {}
}

impl std::fmt::Debug for AdmittedRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmittedRequest")
            .field("gated", &self.gate.is_some())
            .finish()
    }
}

impl Drop for AdmittedRequest {
    fn drop(&mut self) {
        if let Some(gate) = self.gate.take() {
            gate.request_ended();
        }
    }
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
    /// source calls this immediately before sending each request, and holds the answer until
    /// the request is over: see [`AdmittedRequest`]. The admission ends at the gate that
    /// admitted it, wherever it is dropped.
    ///
    /// # Errors
    ///
    /// The gate's refusal, which admits nothing and is never ended; with no gate installed,
    /// never.
    pub async fn before(request: &GatedRequest) -> Result<AdmittedRequest, SourceError> {
        match Self::current() {
            Some(gate) => {
                gate.before_request(request).await?;
                Ok(AdmittedRequest { gate: Some(gate) })
            }
            None => Ok(AdmittedRequest { gate: None }),
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
        ended: Mutex<usize>,
        refuse_at: usize,
    }

    impl Recording {
        fn refusing_at(refuse_at: usize) -> Arc<Self> {
            Arc::new(Self {
                asked: Mutex::new(Vec::new()),
                told: Mutex::new(Vec::new()),
                ended: Mutex::new(0),
                refuse_at,
            })
        }

        fn ended(&self) -> usize {
            *self.ended.lock().unwrap()
        }
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

        fn request_ended(&self) {
            *self.ended.lock().unwrap() += 1;
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
        let gate = Recording::refusing_at(2);
        let installed: Arc<dyn RequestGate> = gate.clone();
        let (first, second) = <dyn RequestGate>::scope(installed, async {
            let first = <dyn RequestGate>::before(&request("graphql"))
                .await
                .map(drop);
            <dyn RequestGate>::after(&[reading()]);
            <dyn RequestGate>::after(&[]);
            let second = <dyn RequestGate>::before(&request("core")).await.map(drop);
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
        // The admitted request ended once; the refused one was never admitted, so never ends.
        assert_eq!(gate.ended(), 1);
        assert_eq!(
            *gate.asked.lock().unwrap(),
            [request("graphql"), request("core")]
        );
        assert_eq!(*gate.told.lock().unwrap(), [reading()]);
        // Outside the scope, nothing is asked and nothing ends.
        assert!(<dyn RequestGate>::before(&request("graphql")).await.is_ok());
        assert_eq!(gate.asked.lock().unwrap().len(), 2);
        assert_eq!(gate.ended(), 1);
    }

    #[tokio::test]
    async fn an_admission_ends_once_when_dropped_and_only_then_at_the_gate_that_admitted_it() {
        let gate = Recording::refusing_at(usize::MAX);
        let installed: Arc<dyn RequestGate> = gate.clone();
        let admitted = <dyn RequestGate>::scope(installed, async {
            <dyn RequestGate>::before(&request("graphql")).await
        })
        .await
        .expect("admitted");
        // Still in flight while the source holds it, though its scope is over.
        assert_eq!(gate.ended(), 0);
        admitted.end();
        assert_eq!(gate.ended(), 1);
    }

    #[tokio::test]
    async fn a_request_future_dropped_in_flight_ends_its_admission_once() {
        let gate = Recording::refusing_at(usize::MAX);
        let installed: Arc<dyn RequestGate> = gate.clone();
        let (started, mut start) = tokio::sync::oneshot::channel::<()>();
        let in_flight = <dyn RequestGate>::scope(installed, async move {
            let _admitted = <dyn RequestGate>::before(&request("graphql")).await?;
            drop(started);
            // A response that never comes: the future is dropped while it waits.
            std::future::pending::<()>().await;
            Ok::<(), SourceError>(())
        });
        let in_flight = tokio::spawn(in_flight);
        // The sender is dropped once the request was admitted and is in flight.
        assert!((&mut start).await.is_err());
        assert_eq!(gate.ended(), 0);
        in_flight.abort();
        assert!(in_flight.await.unwrap_err().is_cancelled());
        assert_eq!(gate.ended(), 1);
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
