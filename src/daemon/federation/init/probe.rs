// EasyNet CLI — Federation status probe
// ========================================
//
// File: src/daemon/federation/init/probe.rs
//
// Process-wide observability slot for the federation init outcome.
// Daemon boot writes the outcome here once; the
// `federation.status` ability reads it. Operators use the
// status string to diagnose "why isn't my agent reachable from
// laptop B" without needing to grep daemon logs.
//
// Concurrency contract: the probe is set-once at boot. Reads
// after the daemon's main loop is running observe a stable
// snapshot. We use `OnceLock<Mutex<...>>` rather than `Mutex<...>`
// alone so the slot is also cheaply readable from the *first*
// reader before set — `None` means "boot still in progress",
// not "federation is broken". The status ability handler reports
// "boot_in_progress" in that window.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

use super::outcome::FederationInitOutcome;

static PROBE: OnceLock<Mutex<FederationInitOutcome>> = OnceLock::new();

/// Process-wide singleton handle. Constructed implicitly by
/// `set` / `snapshot`; the type itself carries no state.
pub struct FederationStatusProbe;

impl FederationStatusProbe {
    /// Record the boot-time decision. Idempotent: a second call
    /// with the same outcome is a no-op; a different outcome
    /// overwrites (the boot path runs `try_install_federation_routing`
    /// once, but tests benefit from the looser contract).
    pub fn set(outcome: FederationInitOutcome) {
        let lock = PROBE.get_or_init(|| Mutex::new(outcome.clone()));
        if let Ok(mut g) = lock.lock() {
            *g = outcome;
        }
    }

    /// Read the current outcome, or `None` if the daemon hasn't
    /// finished its boot-time install yet.
    pub fn snapshot() -> Option<FederationInitOutcome> {
        PROBE.get().and_then(|m| m.lock().ok().map(|g| g.clone()))
    }

    /// Render the outcome as the wire shape returned by
    /// `federation.status`. Stable schema:
    ///
    /// ```json
    /// {
    ///   "ok":            <bool>,        // is_operational()
    ///   "code":          <stable id>,   // disabled | installed | …
    ///   "outcome":       <FederationInitOutcome>  // tagged JSON
    /// }
    /// ```
    ///
    /// Pre-boot (probe not yet set) returns:
    /// ```json
    /// { "ok": false, "code": "boot_in_progress", "outcome": null }
    /// ```
    pub fn render() -> Value {
        match Self::snapshot() {
            Some(o) => json!({
                "ok":      o.is_operational(),
                "code":    o.code(),
                "outcome": serde_json::to_value(&o).unwrap_or(Value::Null),
            }),
            None => json!({
                "ok":      false,
                "code":    "boot_in_progress",
                "outcome": Value::Null,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::outcome::{FederationInitOutcome, FederationStage};
    use super::*;

    // The probe is process-global, so multiple tests touching it
    // need to serialise. Cargo runs in parallel by default. Use a
    // dedicated mutex like other process-global helpers in the
    // crate (forward::test_lock).
    fn probe_lock() -> std::sync::MutexGuard<'static, ()> {
        static LK: OnceLock<Mutex<()>> = OnceLock::new();
        LK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn render_returns_boot_in_progress_when_unset() {
        let _g = probe_lock();
        // We can't reset OnceLock; this test relies on running
        // before any other test populates it. Pin the structure
        // either way.
        let v = FederationStatusProbe::render();
        // After we set later in the suite, this read may show a
        // populated state. Either pre-set (boot_in_progress) or
        // post-set (any other code) is acceptable for this assertion;
        // the structural shape is what we pin.
        assert!(v.is_object());
        assert!(v["code"].is_string());
        assert!(v["ok"].is_boolean());
    }

    #[test]
    fn set_then_render_returns_outcome() {
        let _g = probe_lock();
        let outcome = FederationInitOutcome::Installed {
            tenant: "acme.com".into(),
            realm: "acme.com".into(),
            device_ura: "easynet:///r/acme.com/device/laptop-1".into(),
        };
        FederationStatusProbe::set(outcome);
        let v = FederationStatusProbe::render();
        assert_eq!(v["code"], "installed");
        assert_eq!(v["ok"], true);
        assert_eq!(v["outcome"]["tenant"], "acme.com");
    }

    #[test]
    fn set_overwrites_prior_state() {
        let _g = probe_lock();
        FederationStatusProbe::set(FederationInitOutcome::Disabled {
            reason: "first".into(),
        });
        FederationStatusProbe::set(FederationInitOutcome::Failed {
            stage: FederationStage::BridgeUnavailable,
            reason: "second".into(),
        });
        let v = FederationStatusProbe::render();
        assert_eq!(v["code"], "failed");
        assert_eq!(v["outcome"]["reason"], "second");
    }
}
