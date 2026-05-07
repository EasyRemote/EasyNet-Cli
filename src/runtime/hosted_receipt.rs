// EasyNet CLI — Hosted Agent receipt model (RFC-001 §A12 / §1.3)
// ================================================================
//
// File: src/runtime/hosted_receipt.rs
//
// Per RFC-001 §A12 [P1], a daemon-spawned hosted Agent (consent /
// policy / mcp / llm-profile) does NOT own a private key. Its
// receipts are signed by the hosting device-profile Agent's key,
// and the hosted-vs-host relationship is carried explicitly so an
// offline verifier can reconstruct it.
//
// Receipt schema for hosted Agents
// --------------------------------
//
//   HostedAgentReceiptHeader {
//     callee_agent_uri:  "easynet:///r/<realm>/agent/<hosted-id>",
//     signer_agent_uri:  "easynet:///r/<realm>/agent/<host-id>",
//     host_attestation:  signed assertion that signer hosts callee
//   }
//
// `callee_agent_uri == signer_agent_uri` for self-signed Agents
// (§1.3 Model A — hub, backend, device-profile). They differ for
// hosted Agents (§1.3 Model B — every CLI-spawned Agent).
//
// What this module IS
// -------------------
// The Rust shape + a builder. Constructing the header forces
// callers to think through the model A/B distinction and to carry
// the host_attestation byte string they MUST have obtained via
// `federation.advertise_agent` (the hub records the attestation in
// the directory entry).
//
// What this module is NOT
// -----------------------
// - Not a signature implementation. Signing is done by the device-
//   profile's Ed25519 key in the daemon process; this module only
//   carries the bytes around.
// - Not a verifier. The axon-runtime's admission gate verifies the
//   signature against the directory entry's public_key field.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};

/// The hosted-vs-self distinction recorded on every Agent receipt
/// produced by this daemon. §A12 demands the callee/signer split
/// be observable, so the verifier can pick the right pubkey from
/// the directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum SigningModel {
    /// §1.3 Model A — Agent owns its own keypair. callee == signer
    /// in the receipt header. Used by hub-profile, device-profile,
    /// backend-profile.
    Selfsigned,
    /// §1.3 Model B — Agent has no key of its own; the hosting
    /// device-profile signs on its behalf. The receipt header
    /// carries both the apparent callee and the actual signer.
    HostedBy {
        /// Canonical URA of the hosting device-profile Agent.
        host_uri: String,
        /// Signed assertion (issued by the hub during
        /// `federation.advertise_agent`) that `host_uri` hosts
        /// the callee. Carried opaquely; the verifier checks
        /// against the hub's recorded directory entry.
        host_attestation: Vec<u8>,
    },
}

/// Header attached to every receipt this daemon emits. Goes in
/// the receipt body's metadata bag (the actual on-wire receipt
/// schema lives in the axon proto; this is the CLI-side staging
/// shape that the daemon hands to the dispatcher).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedAgentReceiptHeader {
    /// The Agent the caller targeted — `target.callee` in the
    /// envelope. For a `claude.skill.alive-video` invoke against
    /// the daemon, this is the LLM-profile Agent's URA.
    pub callee_agent_uri: String,
    /// Whose private key actually produced the signature. For
    /// Selfsigned, equal to `callee_agent_uri`. For HostedBy,
    /// equal to `host_uri`.
    pub signer_agent_uri: String,
    pub model: SigningModel,
}

#[derive(Debug, PartialEq)]
pub enum HostedReceiptError {
    EmptyCallee,
    EmptySigner,
    EmptyHostUri,
    EmptyAttestation,
    /// HostedBy declared but `signer_agent_uri != host_uri` — would
    /// produce an inconsistent receipt the verifier could not check.
    SignerNotHost,
    /// Selfsigned declared but `signer_agent_uri != callee_agent_uri`.
    SignerNotCallee,
}

impl std::fmt::Display for HostedReceiptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            HostedReceiptError::EmptyCallee => "callee_agent_uri must not be empty",
            HostedReceiptError::EmptySigner => "signer_agent_uri must not be empty",
            HostedReceiptError::EmptyHostUri => "HostedBy.host_uri must not be empty",
            HostedReceiptError::EmptyAttestation => "HostedBy.host_attestation must not be empty",
            HostedReceiptError::SignerNotHost => "HostedBy: signer_agent_uri must equal host_uri",
            HostedReceiptError::SignerNotCallee => {
                "Selfsigned: signer_agent_uri must equal callee_agent_uri"
            }
        };
        f.write_str(msg)
    }
}

impl std::error::Error for HostedReceiptError {}

impl HostedAgentReceiptHeader {
    /// Construct a Self-signed receipt header. callee == signer
    /// is enforced by the constructor — pass one URA in, get a
    /// header that records the same URA on both sides.
    pub fn new_selfsigned(agent_uri: impl Into<String>) -> Result<Self, HostedReceiptError> {
        let agent_uri = agent_uri.into();
        if agent_uri.trim().is_empty() {
            return Err(HostedReceiptError::EmptyCallee);
        }
        Ok(Self {
            callee_agent_uri: agent_uri.clone(),
            signer_agent_uri: agent_uri,
            model: SigningModel::Selfsigned,
        })
    }

    /// Construct a Hosted receipt header. Validates that the host
    /// URA is non-empty and that an attestation was supplied — a
    /// HostedBy receipt without attestation is unverifiable, and
    /// shipping one would silently degrade to "trust the daemon",
    /// which is exactly what §A12 forbids.
    pub fn new_hosted(
        callee_agent_uri: impl Into<String>,
        host_uri: impl Into<String>,
        host_attestation: Vec<u8>,
    ) -> Result<Self, HostedReceiptError> {
        let callee_agent_uri = callee_agent_uri.into();
        let host_uri = host_uri.into();
        if callee_agent_uri.trim().is_empty() {
            return Err(HostedReceiptError::EmptyCallee);
        }
        if host_uri.trim().is_empty() {
            return Err(HostedReceiptError::EmptyHostUri);
        }
        if host_attestation.is_empty() {
            return Err(HostedReceiptError::EmptyAttestation);
        }
        Ok(Self {
            callee_agent_uri,
            signer_agent_uri: host_uri.clone(),
            model: SigningModel::HostedBy {
                host_uri,
                host_attestation,
            },
        })
    }

    /// `true` when this receipt was signed by the same Agent that
    /// the caller targeted (Model A). Verifiers use this to short-
    /// circuit the host_attestation check.
    pub fn is_self_signed(&self) -> bool {
        matches!(self.model, SigningModel::Selfsigned)
    }

    /// Re-validate the invariants on a header that came in over
    /// serde. Use at trust boundaries (e.g. a daemon parsing a
    /// receipt from another daemon's relay) before acting on it.
    pub fn validate(&self) -> Result<(), HostedReceiptError> {
        if self.callee_agent_uri.trim().is_empty() {
            return Err(HostedReceiptError::EmptyCallee);
        }
        if self.signer_agent_uri.trim().is_empty() {
            return Err(HostedReceiptError::EmptySigner);
        }
        match &self.model {
            SigningModel::Selfsigned => {
                if self.signer_agent_uri != self.callee_agent_uri {
                    return Err(HostedReceiptError::SignerNotCallee);
                }
            }
            SigningModel::HostedBy {
                host_uri,
                host_attestation,
            } => {
                if host_uri.trim().is_empty() {
                    return Err(HostedReceiptError::EmptyHostUri);
                }
                if host_attestation.is_empty() {
                    return Err(HostedReceiptError::EmptyAttestation);
                }
                if &self.signer_agent_uri != host_uri {
                    return Err(HostedReceiptError::SignerNotHost);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selfsigned_rejects_empty_uri() {
        assert_eq!(
            HostedAgentReceiptHeader::new_selfsigned("").unwrap_err(),
            HostedReceiptError::EmptyCallee,
        );
    }

    #[test]
    fn selfsigned_records_same_uri_on_both_sides() {
        // URI v4.1.4: hub is realm-singleton (no sub-id); the v2
        // `agent/01HUB` shape has been retired.
        let uri = "easynet:///r/acme/hub";
        let h = HostedAgentReceiptHeader::new_selfsigned(uri).unwrap();
        assert_eq!(h.callee_agent_uri, uri);
        assert_eq!(h.signer_agent_uri, uri);
        assert_eq!(h.model, SigningModel::Selfsigned);
        assert!(h.is_self_signed());
    }

    #[test]
    fn hosted_rejects_empty_callee_or_host_or_attestation() {
        let host = "easynet:///r/acme/device/01DEV";
        assert_eq!(
            HostedAgentReceiptHeader::new_hosted("", host, vec![1]).unwrap_err(),
            HostedReceiptError::EmptyCallee,
        );
        assert_eq!(
            HostedAgentReceiptHeader::new_hosted("c", "", vec![1]).unwrap_err(),
            HostedReceiptError::EmptyHostUri,
        );
        assert_eq!(
            HostedAgentReceiptHeader::new_hosted("c", host, vec![]).unwrap_err(),
            HostedReceiptError::EmptyAttestation,
        );
    }

    #[test]
    fn hosted_records_distinct_callee_and_signer() {
        let callee = "easynet:///r/acme/agent/u1.01LLM";
        let host = "easynet:///r/acme/device/01DEV";
        let attestation = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let h = HostedAgentReceiptHeader::new_hosted(callee, host, attestation.clone()).unwrap();
        assert_eq!(h.callee_agent_uri, callee);
        assert_eq!(h.signer_agent_uri, host);
        assert!(!h.is_self_signed());
        match &h.model {
            SigningModel::HostedBy {
                host_uri,
                host_attestation,
            } => {
                assert_eq!(host_uri, host);
                assert_eq!(host_attestation, &attestation);
            }
            _ => panic!("expected HostedBy"),
        }
    }

    #[test]
    fn validate_catches_post_serde_tampering_with_signer_uri() {
        // A peer daemon hands us a Selfsigned-tagged receipt whose
        // signer doesn't match callee — must reject before we trust
        // the body.
        let bad = HostedAgentReceiptHeader {
            callee_agent_uri: "easynet:///r/acme/agent/u1.01A".into(),
            signer_agent_uri: "easynet:///r/acme/agent/u1.01B".into(),
            model: SigningModel::Selfsigned,
        };
        assert_eq!(
            bad.validate().unwrap_err(),
            HostedReceiptError::SignerNotCallee
        );
    }

    #[test]
    fn validate_catches_hosted_with_signer_not_equal_to_host_uri() {
        let bad = HostedAgentReceiptHeader {
            callee_agent_uri: "easynet:///r/acme/agent/u1.01LLM".into(),
            signer_agent_uri: "easynet:///r/acme/agent/u1.01OTHER".into(),
            model: SigningModel::HostedBy {
                host_uri: "easynet:///r/acme/device/01DEV".into(),
                host_attestation: vec![1],
            },
        };
        assert_eq!(
            bad.validate().unwrap_err(),
            HostedReceiptError::SignerNotHost
        );
    }

    #[test]
    fn validate_passes_for_well_formed_selfsigned_and_hosted() {
        let s = HostedAgentReceiptHeader::new_selfsigned("u").unwrap();
        assert!(s.validate().is_ok());
        let h = HostedAgentReceiptHeader::new_hosted("c", "host", vec![1]).unwrap();
        assert!(h.validate().is_ok());
    }

    #[test]
    fn header_round_trips_through_serde_for_both_models() {
        let s = HostedAgentReceiptHeader::new_selfsigned("u").unwrap();
        let j = serde_json::to_string(&s).unwrap();
        let back: HostedAgentReceiptHeader = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
        assert!(back.validate().is_ok());

        let h = HostedAgentReceiptHeader::new_hosted("c", "host", vec![0xAA]).unwrap();
        let j = serde_json::to_string(&h).unwrap();
        let back: HostedAgentReceiptHeader = serde_json::from_str(&j).unwrap();
        assert_eq!(back, h);
        assert!(back.validate().is_ok());
    }

    #[test]
    fn signing_model_serde_uses_snake_case_tag() {
        let h = HostedAgentReceiptHeader::new_selfsigned("u").unwrap();
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["model"]["model"], "selfsigned");
    }
}
