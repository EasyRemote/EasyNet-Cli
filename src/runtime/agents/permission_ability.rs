// EasyNet CLI — consent.{subscribe,decide} (PR-PERM)
// =============================================================
//
// File: src/runtime/system/permission_ability.rs
// Description: Two abilities that surface the approval-broker queue
//              over the IPC plane:
//
//   * `consent.subscribe` (Stream) — emits a snapshot of
//                                              the current pending
//                                              queue, then live
//                                              updates as new asks
//                                              fire. v1 returns a
//                                              one-shot snapshot
//                                              Vec; the live tail
//                                              lands at PR-INVOCATION-
//                                              EXEC-UNITY when the
//                                              IPC fan-out wires up.
//   * `consent.decide`    (RPC)    — deliver a decision
//                                              for a pending id.
//
// Cross-machine semantics
// -----------------------
// docs/rfc/permission-broker-v1.md §4 pins these as advisory: the
// subject_host's local broker is final. v1 daemon default policy
// is "accept the remote advisory as the decision". A hardened
// deployment can override at a future config layer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, StreamSource};
use crate::runtime::domain::{PermissionDecision, PermissionId};
use crate::runtime::execution::permission::PermissionService;

pub const ABILITY_SUBSCRIBE: &str = "consent.subscribe";
pub const ABILITY_DECIDE: &str = "consent.decide";

/// Register the two permission abilities on the registry.
pub fn register(reg: &mut LocalAbilityRegistry, perms: Arc<PermissionService>) {
    let p_for_sub = Arc::clone(&perms);
    reg.register_stream(
        ABILITY_SUBSCRIBE,
        Arc::new(move |args: Value| subscribe_handler(&p_for_sub, args)),
    );
    reg.register_rpc(
        ABILITY_DECIDE,
        Arc::new(move |args: Value| decide_handler(&perms, args)),
    );
}

/// `consent.subscribe` stream handler.
///
/// Args: `{ }` — no parameters in v1.
///
/// Returns a SnapshotThenLive stream when the SubscriberBroker is
/// installed: snapshot of currently-pending requests first, then a
/// broadcast::Receiver tailing every new pending request. When no
/// SubscriberBroker is installed (i.e. AllowAllBroker default),
/// returns an empty Snapshot — the queue is structurally empty.
fn subscribe_handler(svc: &PermissionService, _args: Value) -> anyhow::Result<StreamSource> {
    let snapshot: Vec<Value> = svc
        .pending()
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    match svc.subscriber() {
        Some(sub) => {
            // Convert PermissionRequest broadcast → JSON Value
            // broadcast via a relay task. broadcast doesn't support
            // map-in-place, so we spawn a forwarder and hand back
            // the relay's receiver.
            let mut typed_rx = sub.subscribe();
            let (json_tx, json_rx) = tokio::sync::broadcast::channel::<Value>(64);
            tokio::spawn(async move {
                loop {
                    match typed_rx.recv().await {
                        Ok(req) => {
                            let v = serde_json::to_value(&req).unwrap_or(Value::Null);
                            // If no live IPC subscribers attached the
                            // first message gets dropped; that's fine —
                            // late attaches see it via the snapshot.
                            let _ = json_tx.send(v);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // forwarder is slow; skip lost frames and
                            // keep going. The IPC client will get a
                            // Terminal{error} only if its own receiver
                            // also lags downstream.
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
            Ok(StreamSource::SnapshotThenLive(snapshot, json_rx))
        }
        None => Ok(StreamSource::Snapshot(snapshot)),
    }
}

/// `consent.decide` RPC handler.
///
/// Args: `{ "id": string, "decision": "allow" | "deny" | "allow_once" }`
/// Returns: `{ "ok": true }` on success; structured error otherwise.
///
/// Decision is parsed by serde via `PermissionDecision`'s tagged
/// representation; an unknown variant fails at parse rather than
/// silently routing to a default.
fn decide_handler(svc: &PermissionService, args: Value) -> anyhow::Result<Value> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("permission.decide: `id` required"))?;
    let decision_str = args
        .get("decision")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("permission.decide: `decision` required"))?;
    let decision = match decision_str {
        "allow" => PermissionDecision::Allow,
        "deny" => PermissionDecision::Deny,
        "allow_once" => PermissionDecision::AllowOnce,
        other => anyhow::bail!(
            "permission.decide: `decision` must be allow|deny|allow_once, got {other:?}"
        ),
    };
    svc.decide(&PermissionId::new(id), decision)?;
    Ok(json!({ "ok": true }))
}

pub fn subscribe_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
    })
}

pub fn decide_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "decision"],
        "properties": {
            "id": {"type": "string"},
            "decision": {"type": "string", "enum": ["allow", "deny", "allow_once"]},
        },
        "additionalProperties": false,
    })
}

pub fn subscribe_description() -> &'static str {
    "Subscribe to the approval-broker pending queue. v1 returns a one-shot snapshot of all \
     pending PermissionRequests; live updates land with PR-INVOCATION-EXEC-UNITY."
}

pub fn decide_description() -> &'static str {
    "Deliver a decision for a pending permission request by id. Decision must be one of \
     `allow`, `deny`, or `allow_once`. v1 cross-machine semantics: advisory only."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<PermissionService> {
        Arc::new(PermissionService::with_subscriber_broker())
    }

    #[tokio::test]
    async fn subscribe_returns_empty_for_idle_queue() {
        let svc = fresh();
        let frames = subscribe_handler(&svc, json!({}))
            .unwrap()
            .into_snapshot();
        assert!(frames.is_empty());
    }

    #[test]
    fn decide_unknown_id_returns_error() {
        let svc = fresh();
        let err = decide_handler(
            &svc,
            json!({"id": "perm-doesnotexist", "decision": "allow"}),
        )
        .unwrap_err();
        // Underlying error from SubscriberBroker mentions "unknown";
        // we don't care about exact wording, only that the call
        // surfaced loudly rather than silently no-op'd.
        assert!(format!("{err}").contains("unknown"));
    }

    #[test]
    fn decide_invalid_decision_string_rejects_at_parse() {
        let svc = fresh();
        let err = decide_handler(
            &svc,
            json!({"id": "x", "decision": "maybe"}),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("allow|deny|allow_once"));
    }

    #[test]
    fn decide_missing_fields_errors_clearly() {
        let svc = fresh();
        let no_id = decide_handler(&svc, json!({"decision": "allow"})).unwrap_err();
        assert!(format!("{no_id}").contains("id"));
        let no_decision = decide_handler(&svc, json!({"id": "x"})).unwrap_err();
        assert!(format!("{no_decision}").contains("decision"));
    }
}
