// EasyNet CLI — Federation initialisation (RFC-002 + RFC-002.2)
// ==============================================================
//
// File: src/runtime/federation_init/mod.rs
//
// Single seam where the daemon decides whether to participate in
// federation and, if so, installs the production wiring:
//
//   * `BridgeForwardInvoker` — concrete `CliForwardInvoker` that
//     turns a remote `<self>.invoke(target=peer)` into a real
//     `federation.forward_invoke` call over the daemon's
//     existing gRPC bridge.
//
//   * `FederationStatusProbe` — process-wide handle exposing the
//     decision to observers (the `<self>.federation.status`
//     ability + operator log lines).
//
// Design contract
// ---------------
// The init function is a *pure decision* over (`Credentials`,
// `KeyringHandle`, optional `Bridge`):
//
//   ```
//   try_install_federation_routing(...) -> FederationInitOutcome
//   ```
//
// `FederationInitOutcome` is a typed enum reflecting one of four
// terminal states:
//
//   * `Disabled { reason }` — operator opted out (env var, missing
//     credentials, `*.localhost` tenant). Daemon runs in fully
//     local-only mode; cross-device invokes return
//     `target_not_registered` like before.
//   * `Installed { tenant, realm, device_uri }` — invoker is
//     registered; the next federation-shaped invoke routes
//     through the bridge.
//   * `AlreadyInstalled { ... }` — second call after a successful
//     first call. No-op; mirrors the set-once contract on
//     `forward::FORWARD_INVOKER`.
//   * `Failed { stage, reason }` — federation was wanted but a
//     prerequisite failed (no bridge, keyring lock, etc.). Daemon
//     keeps running; cross-device invokes are unavailable until
//     the operator restarts after fixing the root cause.
//
// **Failure mode discipline:** init failure NEVER returns Err to
// the daemon. The daemon's IPC + ping + permission paths must
// stay alive even when federation is mis-configured. `Failed`
// is the right shape — the operator sees the diagnostic via
// the status probe; the daemon does not crash-loop on a
// transient hub outage during boot.
//
// Tested in isolation: every test in this module exercises
// `try_install_federation_routing` with hand-built fixtures, no
// real bridge / no real daemon. The wiring point in
// `bin/easynet-daemon.rs` is intentionally a single line so it's
// trivially correct by inspection.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

pub mod outcome;
pub mod probe;
pub mod resolver_seed;

pub use outcome::{FederationInitOutcome, FederationStage};
pub use probe::FederationStatusProbe;

use std::sync::Arc;

use crate::persistence::config::Credentials;
use crate::runtime::keyring::bridge_forward::BridgeForwardInvoker;
use crate::runtime::keyring::KeyringHandle;
use crate::runtime::resolver as tenant_resolver;

/// Environment opt-out. When set to "1" / "true", the daemon
/// runs without federation wiring even if credentials look
/// federation-capable. Useful for: operators debugging a hub
/// outage; CI runs that don't need cross-device.
pub const ENV_FEDERATION_DISABLE: &str = "EASYNET_FEDERATION_DISABLE";

/// Construction-time inputs. Boot reads env vars + config files
/// **once**, builds this struct, and hands it to
/// `try_install_federation_routing`. Tests construct it directly.
/// Splitting "read env" from "decide what to do" is what makes
/// the decision logic deterministic + parallel-safe.
///
/// No Debug/Clone derive: `DendriteBridge` is `!Debug` (the SDK
/// keeps the FFI handle opaque) and the borrowed `Arc` references
/// don't benefit from cloning the wrapper. Construct fresh per
/// call.
pub struct FederationInitInputs<'a> {
    pub creds: &'a Credentials,
    pub keyring: &'a Arc<KeyringHandle>,
    pub bridge: Option<&'a Arc<easynet_axon::dendrite_bridge::DendriteBridge>>,
    /// Operator opt-out — boot reads `EASYNET_FEDERATION_DISABLE`
    /// here. When true, init returns `Disabled` regardless of
    /// other inputs.
    pub disabled_by_operator: bool,
    /// Resolver config (rendezvous list + static_hubs). Boot reads
    /// from env + `~/.config/easynet/rendezvous.json`.
    pub resolver_config: tenant_resolver::ResolverConfig,
}

/// Boot helper: read all federation-relevant env vars + config
/// files in one pass. Test code should construct `FederationInitInputs`
/// directly instead of calling this — that keeps tests off the
/// process-global env var.
pub fn read_inputs_from_env<'a>(
    creds: &'a Credentials,
    keyring: &'a Arc<KeyringHandle>,
    bridge: Option<&'a Arc<easynet_axon::dendrite_bridge::DendriteBridge>>,
) -> FederationInitInputs<'a> {
    let disabled_by_operator = std::env::var(ENV_FEDERATION_DISABLE)
        .ok()
        .as_deref()
        .map(|v| matches!(v, "1" | "true" | "TRUE"))
        .unwrap_or(false);
    FederationInitInputs {
        creds,
        keyring,
        bridge,
        disabled_by_operator,
        resolver_config: tenant_resolver::ResolverConfig::from_env_and_file(),
    }
}

/// Decide whether federation should be installed and, when yes,
/// install it. Returns a typed outcome the caller logs and / or
/// publishes through the status probe.
///
/// `bridge` is `Option` because the daemon's existing boot path
/// makes it possible to start with the bridge unconnected (Hub
/// unreachable at boot). When `None`, federation init reports
/// `Failed { stage = NoBridge }` so an operator can diagnose
/// without needing to read source.
pub fn try_install_federation_routing(
    inputs: FederationInitInputs<'_>,
) -> FederationInitOutcome {
    let FederationInitInputs {
        creds,
        keyring,
        bridge,
        disabled_by_operator,
        resolver_config,
    } = inputs;

    // ── Opt-out: env override (read once at boot, passed in) ───
    if disabled_by_operator {
        return FederationInitOutcome::Disabled {
            reason: format!("{ENV_FEDERATION_DISABLE} set"),
        };
    }

    // ── Opt-out: empty / unjoined credentials ──────────────────
    if creds.tenant_id.is_empty() || creds.node_id.is_empty() {
        return FederationInitOutcome::Disabled {
            reason: "credentials.json missing tenant_id / node_id (run `easynet device join`)"
                .into(),
        };
    }

    // ── Opt-out: `*.localhost` tenants are local-only by design ─
    let resolution = tenant_resolver::resolve(&creds.tenant_id, &resolver_config);
    if matches!(resolution.mode, tenant_resolver::AdmissionMode::LocalFast) {
        return FederationInitOutcome::Disabled {
            reason: format!(
                "tenant {:?} resolves to Local-fast mode (no federation hub configured)",
                creds.tenant_id
            ),
        };
    }

    // ── Prereq: bridge must be connected ───────────────────────
    let bridge = match bridge {
        Some(b) => Arc::clone(b),
        None => {
            return FederationInitOutcome::Failed {
                stage: FederationStage::BridgeUnavailable,
                reason: "daemon bridge is not connected; federation routes will be \
                         unavailable until the next successful bridge connect"
                    .into(),
            };
        }
    };

    // ── Prereq: keyring must hold a device subject ─────────────
    let device_uri = match keyring.device_subject() {
        Some(u) => u,
        None => {
            // Synthesise + bind on first install. Subsequent calls
            // re-use the bound subject so re-installs are idempotent.
            let synth = format!(
                "easynet:///r/{tenant}/agent/{node}",
                tenant = creds.tenant_id,
                node = creds.node_id
            );
            if let Err(e) = keyring.set_device_subject(synth.clone()) {
                return FederationInitOutcome::Failed {
                    stage: FederationStage::KeyringBind,
                    reason: format!("keyring.set_device_subject({synth}): {e}"),
                };
            }
            synth
        }
    };

    // ── Install (set-once). A second call after a successful
    //    first call is the AlreadyInstalled outcome.
    let installed = BridgeForwardInvoker::install_for_daemon(
        bridge,
        Arc::clone(keyring),
        creds.tenant_id.clone(),
        creds.tenant_id.clone(), // realm == tenant in v1 (see start.rs:511)
    );
    if !installed {
        return FederationInitOutcome::AlreadyInstalled {
            tenant: creds.tenant_id.clone(),
            realm: creds.tenant_id.clone(),
            device_uri,
        };
    }
    FederationInitOutcome::Installed {
        tenant: creds.tenant_id.clone(),
        realm: creds.tenant_id.clone(),
        device_uri,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Minimal `Credentials` fixture for the decision tests. We
    /// only set the fields the init function reads.
    fn creds(tenant: &str, node: &str) -> Credentials {
        Credentials {
            node_id: node.into(),
            credential_token: "tok".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            tenant_id: tenant.into(),
            deploy_signature: String::new(),
            hub_api_base: None,
        }
    }

    fn keyring() -> Arc<KeyringHandle> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keyring.json");
        let h = Arc::new(KeyringHandle::open_or_create(path, "p").unwrap());
        // Persist for the test's lifetime.
        std::mem::forget(dir);
        h
    }

    /// Build inputs without touching env vars. Tests pass
    /// `disabled_by_operator` and `resolver_config` directly,
    /// which is the whole point of the FederationInitInputs split:
    /// decision logic is deterministic, parallel-safe, no global
    /// state.
    fn inputs_for<'a>(
        creds: &'a Credentials,
        keyring: &'a Arc<KeyringHandle>,
        bridge: Option<&'a Arc<easynet_axon::dendrite_bridge::DendriteBridge>>,
        disabled_by_operator: bool,
    ) -> FederationInitInputs<'a> {
        FederationInitInputs {
            creds,
            keyring,
            bridge,
            disabled_by_operator,
            resolver_config: tenant_resolver::ResolverConfig::default(),
        }
    }

    #[test]
    fn disabled_when_operator_opted_out() {
        let c = creds("acme.com", "node-1");
        let k = keyring();
        let out = try_install_federation_routing(inputs_for(&c, &k, None, true));
        match out {
            FederationInitOutcome::Disabled { reason } => {
                assert!(reason.contains(ENV_FEDERATION_DISABLE), "{reason}");
            }
            other => panic!("expected Disabled, got {other:?}"),
        }
    }

    #[test]
    fn disabled_when_credentials_unjoined() {
        let c = creds("", "");
        let k = keyring();
        let out = try_install_federation_routing(inputs_for(&c, &k, None, false));
        match out {
            FederationInitOutcome::Disabled { reason } => {
                assert!(reason.contains("credentials.json"), "{reason}");
            }
            other => panic!("expected Disabled, got {other:?}"),
        }
    }

    #[test]
    fn disabled_for_localhost_tenants() {
        // `.localhost` tenants resolve to LocalFast under the
        // default resolver config — federation routes are
        // deliberately not installed.
        let c = creds("silan.localhost", "node-1");
        let k = keyring();
        let out = try_install_federation_routing(inputs_for(&c, &k, None, false));
        match out {
            FederationInitOutcome::Disabled { reason } => {
                assert!(reason.contains("Local-fast"), "{reason}");
            }
            other => panic!("expected Disabled, got {other:?}"),
        }
    }

    #[test]
    fn failed_when_bridge_missing_for_federated_tenant() {
        // FQDN tenant + no bridge → Failed{BridgeUnavailable}.
        // Daemon should keep running; the operator gets a
        // diagnostic via the status probe.
        let c = creds("acme.com", "node-1");
        let k = keyring();
        let out = try_install_federation_routing(inputs_for(&c, &k, None, false));
        match out {
            FederationInitOutcome::Failed { stage, reason } => {
                assert_eq!(stage, FederationStage::BridgeUnavailable);
                assert!(reason.contains("bridge"), "{reason}");
            }
            other => panic!("expected Failed{{BridgeUnavailable}}, got {other:?}"),
        }
    }

    // Note: the Installed and AlreadyInstalled paths require a
    // live DendriteBridge to be constructed, which (a) needs the
    // dendrite bridge dynamic library (b) tries to dial a Hub.
    // Both are out of scope for unit tests; they're exercised by
    // the daemon-boot integration tests in `tests/`.

    #[test]
    fn module_path_prefix_used_below_for_silencing_unused() {
        // Compile-time guard: keep the submodule imports referenced
        // even when downstream modules don't yet consume every
        // helper. Cheaper than `#[allow(unused)]` peppered through
        // the file.
        let _ = PathBuf::from("/dev/null");
        let _ = std::any::type_name::<probe::FederationStatusProbe>();
        let _ = std::any::type_name::<resolver_seed::ResolverSeed>();
    }
}
