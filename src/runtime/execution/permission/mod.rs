// EasyNet CLI — Execution / Permission sub-service
// =================================================
//
// File: src/runtime/execution/permission/mod.rs
// Description: Approval broker + pending-request queue. v1 contract
//              is the "approval broker" semantics frozen in
//              docs/rfc/permission-broker-v1.md — interactive
//              human approval for sensitive agent actions, NOT
//              capability security.
//
// What v1 is NOT
// --------------
// - Not capability-based (no signed claim binds decisions to
//   future Invocations).
// - Not concurrent-strict (allow_once is best-effort across races).
// - Not cross-machine grant-equivalent (remote decisions are
//   advisory; the subject_host's local broker is final).
// - Not audit-grade (decisions are not signed).
//
// All four are pinned in the RFC; touching this module without
// reading the RFC first is a likely category error.
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::runtime::domain::{
    PermissionDecision, PermissionId, PermissionRequest, PermissionSensitivity, SessionId,
    TenantId,
};

/// Context a handler supplies to the broker at admission time.
/// PR-INVOCATION-EXEC-UNITY extends the call site to construct
/// this from the in-flight Invocation.
///
/// `capability_claim` is reserved for v2 signed invocation. v1
/// always None; populating it has no effect.
#[derive(Debug, Clone)]
pub struct AskContext {
    pub prompt: String,
    pub sensitivity: PermissionSensitivity,
    pub session: SessionId,
    pub tenant: TenantId,
    /// v2 capability-claim payload (AXIOM §6.3). v1 always None.
    #[allow(dead_code)]
    pub capability_claim: Option<CapabilityClaim>,
}

/// Opaque v2 capability-claim placeholder. v1 defines the type so
/// `AskContext` compiles; v2 will expand it under AXIOM §6.3.
#[derive(Debug, Clone)]
pub struct CapabilityClaim {
    #[allow(dead_code)]
    pub(crate) _v2_signed_bytes: Vec<u8>,
}

/// Trait implemented by every broker variant. v1 ships
/// AllowAllBroker (default; preserves pre-PR behaviour) and
/// SubscriberBroker (engages when an external observer is
/// listening on `consent.subscribe`).
///
/// `ask` is synchronous in v1 because the call site (mission
/// dispatch) is sync. The SubscriberBroker turns the
/// inherently-async wait-for-decision into a sync block via a
/// blocking receive on a tokio oneshot channel.
pub trait PermissionBroker: Send + Sync {
    fn ask(&self, ctx: AskContext) -> PermissionDecision;
}

/// Default broker: every ask returns Allow. Preserves the
/// behaviour the codebase had before PR-PERM, so a deployment that
/// has not opted into the subscriber path keeps working unchanged.
#[derive(Debug, Default)]
pub struct AllowAllBroker;

impl AllowAllBroker {
    pub fn new() -> Self {
        Self
    }
}

impl PermissionBroker for AllowAllBroker {
    fn ask(&self, _ctx: AskContext) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// Subscriber broker: when at least one observer is listening on
/// `consent.subscribe`, every ask becomes a pending
/// request published to the broadcast channel. The handler waits
/// for `decide_permission` to deliver a decision via the per-
/// request response channel.
///
/// v1 implementation note: the wait is bounded by an internal
/// timeout (default 10 minutes); a request that times out resolves
/// to `Deny` (fail-closed). The timeout is the v1 contract for
/// "subscriber went away mid-request" — the subscriber dropping
/// out leaves no decider, and a default-allow on timeout would
/// invert the security posture.
pub struct SubscriberBroker {
    pending: RwLock<BTreeMap<PermissionId, PendingState>>,
    publish: broadcast::Sender<PermissionRequest>,
    timeout: std::time::Duration,
}

struct PendingState {
    request: PermissionRequest,
    decider_tx: tokio::sync::oneshot::Sender<PermissionDecision>,
}

impl std::fmt::Debug for PendingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingState")
            .field("request", &self.request)
            .finish()
    }
}

impl SubscriberBroker {
    pub fn new() -> Self {
        Self::with_timeout(std::time::Duration::from_secs(600))
    }

    pub fn with_timeout(timeout: std::time::Duration) -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            pending: RwLock::new(BTreeMap::new()),
            publish: tx,
            timeout,
        }
    }

    /// Subscribe to the live pending-request stream. Each call
    /// returns a fresh receiver; subscribers obtained after a
    /// pending request was published do NOT see it (the broker is
    /// not a backlog — late joiners are expected to call
    /// `pending_snapshot` once and tail from there).
    pub fn subscribe(&self) -> broadcast::Receiver<PermissionRequest> {
        self.publish.subscribe()
    }

    /// Snapshot of every currently-pending request. PR-PERM's
    /// `consent.subscribe` handler emits this snapshot
    /// before tailing live updates so a Client sees the full
    /// queue on first connection.
    pub fn pending_snapshot(&self) -> Vec<PermissionRequest> {
        self.pending
            .read()
            .ok()
            .map(|g| g.values().map(|p| p.request.clone()).collect())
            .unwrap_or_default()
    }

    /// Deliver a decision. Removes the pending state, sends the
    /// decision to the waiting handler. Returns Err when the id
    /// is unknown (already decided or never created).
    pub fn decide(
        &self,
        id: &PermissionId,
        decision: PermissionDecision,
    ) -> anyhow::Result<()> {
        let pending = {
            let mut g = self
                .pending
                .write()
                .map_err(|_| anyhow::anyhow!("SubscriberBroker lock poisoned"))?;
            g.remove(id)
        };
        match pending {
            Some(p) => {
                let _ = p.decider_tx.send(decision);
                Ok(())
            }
            None => anyhow::bail!("permission id {id} unknown (already decided?)"),
        }
    }

    /// True when at least one subscriber is currently connected.
    /// The Kernel uses this to swap between SubscriberBroker and
    /// AllowAllBroker behaviour: if no Client is listening, the
    /// dispatch should not block.
    pub fn has_subscribers(&self) -> bool {
        self.publish.receiver_count() > 0
    }

    fn fresh_id() -> PermissionId {
        PermissionId::new(format!("perm-{}", Uuid::new_v4()))
    }
}

impl Default for SubscriberBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionBroker for SubscriberBroker {
    fn ask(&self, ctx: AskContext) -> PermissionDecision {
        // Fail-open to AllowAll when no subscriber is connected.
        // Plan v10.5 explicitly opts for "AllowAllBroker behaviour"
        // when there is no observer; turning the broker into a
        // hard block under that condition would freeze every
        // mission run on a daemon with no UI attached.
        if !self.has_subscribers() {
            return PermissionDecision::Allow;
        }
        let id = Self::fresh_id();
        let req = PermissionRequest {
            id: id.clone(),
            session: ctx.session.clone(),
            tenant: ctx.tenant.clone(),
            prompt: ctx.prompt.clone(),
            sensitivity: ctx.sensitivity,
            created_unix_ms: chrono::Utc::now().timestamp_millis(),
            decision: None,
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Insert pending state.
        if let Ok(mut g) = self.pending.write() {
            g.insert(
                id.clone(),
                PendingState {
                    request: req.clone(),
                    decider_tx: tx,
                },
            );
        } else {
            return PermissionDecision::Deny;
        }
        // Publish to subscribers. send() returning Err means there
        // are zero receivers; we already gated on has_subscribers
        // but a race between the gate and here is possible; on
        // race, fail-closed — the conservative choice.
        if self.publish.send(req).is_err() {
            // Clean up pending state on publish failure.
            let _ = self.pending.write().map(|mut g| g.remove(&id));
            return PermissionDecision::Deny;
        }
        // Block on the decision. v1 uses the tokio runtime if one
        // exists; otherwise builds a temporary current-thread one.
        // This is the v1-acceptable bridge between sync mission
        // dispatch and async pending-decision wait.
        wait_for_decision(rx, self.timeout).unwrap_or(PermissionDecision::Deny)
    }
}

/// Bridge an async oneshot receiver into a sync caller with a
/// bounded timeout. Returns `Some(decision)` on success, `None` on
/// timeout or sender drop.
fn wait_for_decision(
    rx: tokio::sync::oneshot::Receiver<PermissionDecision>,
    timeout: std::time::Duration,
) -> Option<PermissionDecision> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let _g = handle.enter();
        return tokio::task::block_in_place(|| {
            handle.block_on(async move {
                tokio::select! {
                    res = rx => res.ok(),
                    _ = tokio::time::sleep(timeout) => None,
                }
            })
        });
    }
    // No runtime in scope: spin a temporary current-thread one.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return None,
    };
    rt.block_on(async move {
        tokio::select! {
            res = rx => res.ok(),
            _ = tokio::time::sleep(timeout) => None,
        }
    })
}

/// Permission sub-service handle. Owns the broker the Kernel
/// installs at boot. v1 default is AllowAllBroker; the daemon bin
/// swaps to SubscriberBroker when at least one subscriber wires up.
pub struct PermissionService {
    broker: Arc<dyn PermissionBroker>,
    subscriber: Option<Arc<SubscriberBroker>>,
}

impl Default for PermissionService {
    fn default() -> Self {
        Self {
            broker: Arc::new(AllowAllBroker::new()),
            subscriber: None,
        }
    }
}

impl PermissionService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the SubscriberBroker variant. The daemon bin calls
    /// this at boot; tests may opt in for behaviour assertions.
    pub fn with_subscriber_broker() -> Self {
        let s = Arc::new(SubscriberBroker::new());
        Self {
            broker: s.clone(),
            subscriber: Some(s),
        }
    }

    /// The broker handle the dispatch path consumes. Cloneable so
    /// PR-INVOCATION-EXEC-UNITY can stash one in the Kernel's
    /// admission phase.
    pub fn broker(&self) -> &Arc<dyn PermissionBroker> {
        &self.broker
    }

    /// Borrow the SubscriberBroker if one is installed. Used by
    /// `consent.subscribe` handler to attach to the
    /// broadcast channel + serve the snapshot.
    pub fn subscriber(&self) -> Option<&Arc<SubscriberBroker>> {
        self.subscriber.as_ref()
    }

    /// PR-PERM `consent.decide` entry point. Forwards
    /// to the SubscriberBroker if installed; rejects with a clear
    /// error otherwise (AllowAllBroker has no pending state to
    /// decide on).
    pub fn decide(
        &self,
        id: &PermissionId,
        decision: PermissionDecision,
    ) -> anyhow::Result<()> {
        match &self.subscriber {
            Some(s) => s.decide(id, decision),
            None => anyhow::bail!(
                "permission.decide called but no SubscriberBroker is installed; \
                 default broker auto-allows every request and has no pending queue"
            ),
        }
    }

    /// Snapshot the pending queue (or empty when AllowAllBroker
    /// is installed).
    pub fn pending(&self) -> Vec<PermissionRequest> {
        self.subscriber
            .as_ref()
            .map(|s| s.pending_snapshot())
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for PermissionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionService")
            .field("has_subscriber", &self.subscriber.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(prompt: &str) -> AskContext {
        AskContext {
            prompt: prompt.into(),
            sensitivity: PermissionSensitivity::Medium,
            session: SessionId::new("s1"),
            tenant: TenantId::default_v1(),
            capability_claim: None,
        }
    }

    #[test]
    fn allow_all_broker_returns_allow_for_every_ask() {
        // Pin the v1 default. Anything else would regress every
        // pre-PR-PERM call site.
        let b = AllowAllBroker::new();
        assert_eq!(b.ask(ctx("anything")), PermissionDecision::Allow);
        assert_eq!(b.ask(ctx("dangerous")), PermissionDecision::Allow);
    }

    #[test]
    fn subscriber_broker_falls_back_to_allow_when_no_observers() {
        // A daemon that has no Client connected on
        // `consent.subscribe` must not block on every
        // mission — that would freeze all autonomous runs. Falling
        // back to Allow here matches AllowAllBroker behaviour.
        let b = SubscriberBroker::new();
        assert!(!b.has_subscribers());
        assert_eq!(b.ask(ctx("noobs")), PermissionDecision::Allow);
    }

    #[test]
    fn decide_unknown_id_errors() {
        let b = SubscriberBroker::new();
        let err = b
            .decide(&PermissionId::new("ghost"), PermissionDecision::Allow)
            .unwrap_err();
        assert!(format!("{err}").contains("unknown"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscriber_broker_round_trips_decision_when_observer_present() {
        // End-to-end: subscribe (becomes the observer), spawn the
        // ask in a blocking task, collect the published pending,
        // decide(Allow), assert the ask returns Allow.
        let b = Arc::new(SubscriberBroker::with_timeout(
            std::time::Duration::from_secs(2),
        ));
        let mut rx = b.subscribe();
        // Spawn ask on a blocking thread so it can block_on the
        // oneshot wait inside this same multi_thread runtime.
        let b2 = Arc::clone(&b);
        let ask_task = tokio::task::spawn_blocking(move || b2.ask(ctx("approve me")));
        // Collect the pending request the broker published.
        let pending = rx.recv().await.expect("pending broadcast");
        b.decide(&pending.id, PermissionDecision::Allow).unwrap();
        let decision = ask_task.await.unwrap();
        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscriber_broker_times_out_to_deny() {
        // Spirit: when the observer goes silent mid-request, the
        // broker must fail-closed rather than hang the mission.
        // 50ms timeout, no decision delivered — expect Deny.
        let b = Arc::new(SubscriberBroker::with_timeout(
            std::time::Duration::from_millis(50),
        ));
        // Need a subscriber so the broker's "no subscribers" gate
        // does not short-circuit. We just attach and never decide.
        let _rx = b.subscribe();
        let b2 = Arc::clone(&b);
        let decision = tokio::task::spawn_blocking(move || b2.ask(ctx("never decided")))
            .await
            .unwrap();
        assert_eq!(decision, PermissionDecision::Deny);
    }

    #[test]
    fn permission_service_default_uses_allow_all() {
        let s = PermissionService::new();
        assert!(s.subscriber().is_none());
        // decide() must error because AllowAllBroker has no queue.
        let err = s
            .decide(&PermissionId::new("x"), PermissionDecision::Allow)
            .unwrap_err();
        assert!(format!("{err}").contains("no SubscriberBroker"));
    }
}
