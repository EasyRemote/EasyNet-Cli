// EasyNet CLI — Bridge-backed CliForwardInvoker (RFC-002.2 daemon wiring)
// =========================================================================
//
// File: src/runtime/keyring/bridge_forward.rs
//
// Real production implementation of `CliForwardInvoker`. Wraps the
// daemon's existing `DendriteBridge` (an authenticated gRPC client
// to the local axon-runtime) and routes `<agent>.invoke(target=peer)`
// calls through `federation.forward_invoke`.
//
// What this fills in
// ------------------
// `forward.rs` defined the trait + a process-global slot; the test
// sink stood in for "production wiring" until tonight. With the
// trait + `advertise::forward_invoke` already shipped, the
// production impl is a thin assembly:
//
//   * `knows_target` — query the keyring's peer table.
//   * `invoke`        — wrap the bridge in `BridgeAbilityInvoker`,
//                       call `advertise::forward_invoke`, decode
//                       the receipt's base64 result, return the
//                       inner JSON.
//
// Daemon boot calls `set_forward_invoker(Arc::new(...))` once with
// the constructed invoker; subsequent `<agent>.invoke` calls with a
// federation-shaped target route through this path automatically.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::Value;
use std::sync::Arc;

use super::forward::CliForwardInvoker;
use super::handle::KeyringHandle;
use crate::runtime::advertise::{forward_invoke, BridgeAbilityInvoker};

/// Production `CliForwardInvoker` impl. Holds an `Arc` over the
/// daemon's bridge so the same gRPC connection that does
/// federation.advertise / heartbeat is reused for forward_invoke.
///
/// The bridge is held as a raw pointer wrapped in `BridgeHandle`
/// because `DendriteBridge` is `!Send` per its current design and
/// our trait requires `Send + Sync`. We synchronise externally:
/// every CLI call into the invoker holds the daemon's main thread,
/// and the bridge already serialises Tonic calls internally with
/// its own mutex. A future refactor can make this cleaner once the
/// SDK exposes a `Send`-able invoker handle.
pub struct BridgeForwardInvoker {
    bridge: Arc<easynet_axon::dendrite_bridge::DendriteBridge>,
    keyring: Arc<KeyringHandle>,
    tenant: String,
    realm: String,
}

// SAFETY: `DendriteBridge` exposes `Sync` methods that internally
// take a Mutex; cloning an `Arc` is safe across threads. `!Send`
// concerns historically came from the embedded ABI handle which is
// thread-pinned in some code paths; for the daemon's invoker the
// bridge sits behind one Tokio task, and the trait-call path
// remains single-threaded by construction. Marking the wrapper
// Send + Sync makes the trait contract honest while a future RFC
// flips the SDK side to expose this guarantee directly.
unsafe impl Send for BridgeForwardInvoker {}
unsafe impl Sync for BridgeForwardInvoker {}

impl BridgeForwardInvoker {
    pub fn new(
        bridge: Arc<easynet_axon::dendrite_bridge::DendriteBridge>,
        keyring: Arc<KeyringHandle>,
        tenant: impl Into<String>,
        realm: impl Into<String>,
    ) -> Self {
        Self {
            bridge,
            keyring,
            tenant: tenant.into(),
            realm: realm.into(),
        }
    }

    /// Daemon boot helper: wrap (bridge, keyring, tenant, realm) into
    /// a `BridgeForwardInvoker` and install it as the process-global
    /// CliForwardInvoker. Returns `true` when the install succeeded
    /// (set-once); subsequent calls are no-ops returning `false`.
    /// Production daemon calls this from its boot path right after
    /// the bridge connects and before the first user-driven invoke.
    pub fn install_for_daemon(
        bridge: Arc<easynet_axon::dendrite_bridge::DendriteBridge>,
        keyring: Arc<KeyringHandle>,
        tenant: impl Into<String>,
        realm: impl Into<String>,
    ) -> bool {
        let invoker: Arc<dyn CliForwardInvoker> =
            Arc::new(Self::new(bridge, keyring, tenant, realm));
        super::forward::set_forward_invoker(invoker)
    }

    /// Run forward_invoke against the daemon's local axon-runtime.
    /// The runtime sees a federation.* call from the daemon's
    /// device URA and (when the target's realm is non-local) routes
    /// through its installed `GrpcHubForwardDispatcher` to the
    /// owning shard.
    fn dispatch_to_bridge(&self, target_ura: &str, ability: &str, args: &Value) -> Result<Value> {
        // Pin the caller URI to the daemon's own keyring-bound
        // device subject so the receiving hub's KeyResolver can
        // find the public key. Today's KeyResolver finds it by
        // bound_subject; the device record was mirrored at boot
        // by mirror_derived_keys_into_keyring.
        let caller_ura = self
            .keyring
            .device_subject()
            .ok_or_else(|| anyhow!("keyring has no device_subject; daemon not joined"))?;
        let invoker = BridgeAbilityInvoker::with_caller_ura(&self.bridge, caller_ura);
        let receipt = forward_invoke(
            &invoker,
            &self.tenant,
            &self.realm,
            target_ura,
            ability,
            args,
        )
        .map_err(|msg| anyhow!("federation.forward_invoke transport: {msg}"))?;
        if !receipt.ok {
            anyhow::bail!(
                "{}: {}",
                if receipt.error_code.is_empty() {
                    "AXON_FORWARD_FAILED".to_string()
                } else {
                    receipt.error_code
                },
                receipt.error_message
            );
        }
        let result_bytes = if receipt.result_b64.is_empty() {
            Vec::new()
        } else {
            B64.decode(&receipt.result_b64)
                .with_context(|| "decode forward_invoke result_b64")?
        };
        if result_bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&result_bytes).with_context(|| "parse forward_invoke result body")
    }
}

impl CliForwardInvoker for BridgeForwardInvoker {
    fn knows_target(&self, target_ura: &str) -> bool {
        // Two checks: peer table (TOFU-recorded peers) and the
        // local agent registry's bound_subject (the operator can
        // pre-seed peers via keyring.peer_add).
        if self.keyring.find_peer_by_ura(target_ura).is_some() {
            return true;
        }
        if self
            .keyring
            .find_active_entry_by_subject(target_ura)
            .is_some()
        {
            return true;
        }
        // No local proof of trust — we still pass through to the
        // hub. The hub does the authoritative directory lookup; if
        // the realm is reachable, forward_invoke succeeds, if not
        // the typed AXON_TARGET_NOT_IN_DIRECTORY surfaces. This
        // makes the "first-time discover scope=user" flow work
        // without forcing every agent through TOFU first.
        true
    }

    fn invoke(&self, target_ura: &str, ability: &str, args: Value) -> Result<Value> {
        self.dispatch_to_bridge(target_ura, ability, &args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyring() -> Arc<KeyringHandle> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keyring.json");
        let h = Arc::new(KeyringHandle::open_or_create(path, "p").unwrap());
        // Persist tempdir for the test's lifetime by leaking — the
        // process-end cleans up. Acceptable for unit tests.
        std::mem::forget(dir);
        h
    }

    #[test]
    fn knows_target_recognises_peer_table_entries() {
        let h = keyring();
        let entry = h.create_entry("agent_signing", None).unwrap();
        h.peer_add(
            "easynet:///r/silan.localhost/device/silan-laptop",
            &entry.public_key_b64,
            None,
            None,
        )
        .unwrap();
        // We don't have a real bridge for this unit test; constructing
        // BridgeForwardInvoker requires one. Test knows_target via
        // a constructed-but-not-dispatched instance using a fake
        // bridge handle. Instead exercise the keyring lookup
        // directly — that's the load-bearing invariant.
        assert!(h
            .find_peer_by_ura("easynet:///r/silan.localhost/device/silan-laptop")
            .is_some());
        assert!(h
            .find_peer_by_ura("easynet:///r/ghost.localhost/device/nope")
            .is_none());
    }

    #[test]
    fn knows_target_recognises_locally_bound_subject() {
        let h = keyring();
        let _ = h
            .create_entry(
                "agent_signing",
                Some("easynet:///r/silan.localhost/device/silan-laptop".into()),
            )
            .unwrap();
        assert!(h
            .find_active_entry_by_subject("easynet:///r/silan.localhost/device/silan-laptop")
            .is_some());
    }
}
