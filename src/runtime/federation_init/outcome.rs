// EasyNet CLI — Federation init outcome enum
// =============================================
//
// File: src/runtime/federation_init/outcome.rs
//
// `FederationInitOutcome` is the protocol between
// `try_install_federation_routing` and its callers (daemon boot,
// status-probe ability handlers, operator log emitters). It is
// deliberately a closed enum:
//
//   * Closed because adding a new state forces every pattern-match
//     in callers to be revisited — accidentally silent skipping of
//     a new failure mode is the bug we are most worried about.
//   * Each variant carries the structured fields a caller needs to
//     act, log, or surface diagnostics — no `String` blob with
//     freeform messaging.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use serde::{Deserialize, Serialize};

/// Stage at which init failed. Operator-facing values; both the
/// status probe and structured logs render the canonical name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederationStage {
    /// `bridge` argument was `None` when a federated tenant required
    /// it. Operator should check Hub reachability + bridge connect
    /// logs.
    BridgeUnavailable,
    /// Keyring did not hold a device subject and `set_device_subject`
    /// failed (disk full, keyring locked by another process).
    KeyringBind,
    /// Reserved for future stages — e.g. shard-resolver config
    /// validation, peer-table sanity checks. Adding a new stage
    /// requires touching every caller, by design.
    Other,
}

impl FederationStage {
    /// Stable string for log lines + the status probe receipt.
    pub fn code(self) -> &'static str {
        match self {
            Self::BridgeUnavailable => "bridge_unavailable",
            Self::KeyringBind => "keyring_bind",
            Self::Other => "other",
        }
    }
}

/// Closed enum of possible terminal states. Adding a variant is a
/// breaking change for all match-sites — prefer extending an
/// existing variant's fields when possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FederationInitOutcome {
    /// Operator opted out, OR the local tenant is by-design
    /// federation-disabled (`*.localhost`). Cross-device invokes
    /// fall through to `target_not_registered` like before.
    Disabled { reason: String },

    /// Federation routing was installed for the first time. The
    /// process-global `forward::FORWARD_INVOKER` slot is now
    /// populated; subsequent installs are no-ops.
    Installed {
        tenant: String,
        realm: String,
        device_ura: String,
    },

    /// A prior call to the same init function already populated
    /// the slot. The current call's caller is OK either way —
    /// the invoker is in place — but the field values reflect the
    /// state captured by THIS call (which may diverge from what
    /// the first install used if credentials changed without a
    /// daemon restart).
    AlreadyInstalled {
        tenant: String,
        realm: String,
        device_ura: String,
    },

    /// Federation was wanted (federated tenant, no env opt-out)
    /// but a prerequisite failed. Daemon keeps running; the
    /// operator sees this via the status probe + log line.
    Failed {
        stage: FederationStage,
        reason: String,
    },
}

impl FederationInitOutcome {
    /// True when federation is operational on this daemon. Used by
    /// the daemon-boot log to decide between "federation: ON" and
    /// "federation: OFF (...)" headlines.
    pub fn is_operational(&self) -> bool {
        matches!(self, Self::Installed { .. } | Self::AlreadyInstalled { .. })
    }

    /// Stable code for status-probe receipts + logs. Mirrors the
    /// JSON `kind` discriminator so external tools can grep
    /// either form.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Disabled { .. } => "disabled",
            Self::Installed { .. } => "installed",
            Self::AlreadyInstalled { .. } => "already_installed",
            Self::Failed { .. } => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_operational_matches_installed_states() {
        assert!(FederationInitOutcome::Installed {
            tenant: "t".into(),
            realm: "r".into(),
            device_ura: "u".into(),
        }
        .is_operational());
        assert!(FederationInitOutcome::AlreadyInstalled {
            tenant: "t".into(),
            realm: "r".into(),
            device_ura: "u".into(),
        }
        .is_operational());
        assert!(!FederationInitOutcome::Disabled { reason: "x".into() }.is_operational());
        assert!(!FederationInitOutcome::Failed {
            stage: FederationStage::BridgeUnavailable,
            reason: "x".into()
        }
        .is_operational());
    }

    #[test]
    fn outcome_serializes_to_tagged_json() {
        let v = serde_json::to_value(&FederationInitOutcome::Disabled {
            reason: "no creds".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], "disabled");
        assert_eq!(v["reason"], "no creds");
    }

    #[test]
    fn stage_code_is_stable() {
        assert_eq!(
            FederationStage::BridgeUnavailable.code(),
            "bridge_unavailable"
        );
        assert_eq!(FederationStage::KeyringBind.code(), "keyring_bind");
        assert_eq!(FederationStage::Other.code(), "other");
    }
}
