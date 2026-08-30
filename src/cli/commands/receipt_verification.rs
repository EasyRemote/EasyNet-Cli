// EasyNet CLI - receipt verification projection
// =============================================
//
// File: src/cli/receipt_verification.rs
// Description: CLI-local receipt verification state shared by invocation
//              surfaces. This module names what the CLI has actually proven;
//              it is not an Axon receipt verifier and must not report a
//              positive or negative verification result unless a verifier ran.

use std::fmt;

#[cfg(feature = "axon-pb")]
use anyhow::{anyhow, bail, Context as _};
#[cfg(feature = "axon-pb")]
use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
#[cfg(feature = "axon-pb")]
use prost::Message as _;

#[cfg(feature = "axon-pb")]
pub const FINALIZATION_PROOF_SET_SCHEMA: &str = "easynet.remoteapp.receipt-proof-set.v2";
#[cfg(feature = "axon-pb")]
pub const RECEIPT_SIGNER_KEYSET_SCHEMA: &str = "easynet.remoteapp.receipt-signer-keyset.v1";

#[cfg(feature = "axon-pb")]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizationProofSet {
    pub schema: String,
    pub campaign: ReceiptCampaignBinding,
    pub proofs: Vec<FinalizationProof>,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptCampaignBinding {
    pub campaign_id: String,
    pub run_id: String,
    pub challenge_nonce_b64: String,
    pub domain_id: String,
    pub caller_device_ura: String,
    pub provider_device_ura: String,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizationProof {
    pub proof_id: String,
    pub invocation_ura: String,
    pub descriptor_ref: String,
    pub subject_ura: String,
    pub caller_ura: String,
    pub callee_ura: String,
    pub session_id: String,
    pub arguments_b64: String,
    pub encoding: String,
    pub admission_receipt_b64: String,
    pub terminal_receipt_b64: String,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptSignerKeyset {
    pub schema: String,
    pub keys: Vec<ReceiptSignerKey>,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptSignerKey {
    pub signer_ura: String,
    pub ed25519_public_key_b64: String,
}

#[cfg(feature = "axon-pb")]
pub struct CampaignReceiptKeyResolver {
    keys: std::collections::BTreeMap<String, Vec<ed25519_dalek::VerifyingKey>>,
}

#[cfg(feature = "axon-pb")]
impl CampaignReceiptKeyResolver {
    pub fn from_keyset(keyset: ReceiptSignerKeyset) -> anyhow::Result<Self> {
        if keyset.schema != RECEIPT_SIGNER_KEYSET_SCHEMA {
            bail!("receipt signer keyset schema must be {RECEIPT_SIGNER_KEYSET_SCHEMA:?}");
        }
        if keyset.keys.is_empty() {
            bail!("receipt signer keyset must contain at least one key");
        }
        let mut keys: std::collections::BTreeMap<_, Vec<_>> = Default::default();
        for (index, row) in keyset.keys.into_iter().enumerate() {
            crate::core::identity::RuntimeIdentityUra::parse(&row.signer_ura)
                .with_context(|| format!("receipt signer keys[{index}].signer_ura is invalid"))?;
            let raw = B64_STANDARD
                .decode(&row.ed25519_public_key_b64)
                .with_context(|| {
                    format!("receipt signer keys[{index}].ed25519_public_key_b64 is invalid")
                })?;
            let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
                anyhow!(
                    "receipt signer keys[{index}].ed25519_public_key_b64 decoded to {} bytes; expected 32",
                    raw.len()
                )
            })?;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&bytes).with_context(|| {
                format!("receipt signer keys[{index}] is not an Ed25519 public key")
            })?;
            let entry = keys.entry(row.signer_ura).or_default();
            if entry.contains(&key) {
                bail!("receipt signer keys[{index}] duplicates an existing key");
            }
            if entry.len() >= axon_sdk::invocation::MAX_KEYS_PER_AGENT_URA {
                bail!("receipt signer keyset exceeds Axon's per-signer key limit");
            }
            entry.push(key);
        }
        Ok(Self { keys })
    }
}

#[cfg(feature = "axon-pb")]
impl axon_sdk::invocation::KeyResolver for CampaignReceiptKeyResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        self.keys
            .get(signer_ura)
            .and_then(|keys| keys.first())
            .copied()
            .ok_or_else(|| {
                axon_sdk::invocation::AxonError::permission_denied(
                    "remoteapp_campaign_receipt_signer_untrusted",
                )
                .with_message(format!(
                    "signed campaign keyset does not trust receipt signer {signer_ura:?}"
                ))
            })
    }

    fn resolve_all(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        self.keys.get(signer_ura).cloned().ok_or_else(|| {
            axon_sdk::invocation::AxonError::permission_denied(
                "remoteapp_campaign_receipt_signer_untrusted",
            )
            .with_message(format!(
                "signed campaign keyset does not trust receipt signer {signer_ura:?}"
            ))
        })
    }
}

#[cfg(feature = "axon-pb")]
fn decode_receipt(
    encoded: &str,
    proof_index: usize,
    stage: &str,
) -> anyhow::Result<axon_sdk::pb::axon::v1::InvocationReceipt> {
    let bytes = B64_STANDARD
        .decode(encoded)
        .with_context(|| format!("proofs[{proof_index}] {stage} receipt is not base64"))?;
    axon_sdk::pb::axon::v1::InvocationReceipt::decode(bytes.as_slice()).with_context(|| {
        format!("proofs[{proof_index}] {stage} receipt is not an InvocationReceipt protobuf")
    })
}

#[cfg(feature = "axon-pb")]
const MAX_CAMPAIGN_ARGUMENTS_BYTES: usize = 1024 * 1024;

#[cfg(feature = "axon-pb")]
const CAMPAIGN_NONCE_DOMAIN: &[u8] = b"easynet.remoteapp.campaign-invocation-nonce.v1\0";

#[cfg(feature = "axon-pb")]
fn require_non_empty<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(value)
}

#[cfg(feature = "axon-pb")]
fn parse_canonical_uuid(value: &str, field: &str) -> anyhow::Result<uuid::Uuid> {
    let value = require_non_empty(value, field)?;
    let parsed = uuid::Uuid::parse_str(value).with_context(|| format!("{field} is not a UUID"))?;
    if parsed.to_string() != value {
        bail!("{field} must use canonical lowercase hyphenated UUID form");
    }
    Ok(parsed)
}

#[cfg(feature = "axon-pb")]
fn decode_campaign_challenge(campaign: &ReceiptCampaignBinding) -> anyhow::Result<[u8; 32]> {
    parse_canonical_uuid(&campaign.campaign_id, "campaign.campaign_id")?;
    parse_canonical_uuid(&campaign.run_id, "campaign.run_id")?;
    require_non_empty(&campaign.domain_id, "campaign.domain_id")?;
    for (field, value) in [
        (
            "campaign.caller_device_ura",
            campaign.caller_device_ura.as_str(),
        ),
        (
            "campaign.provider_device_ura",
            campaign.provider_device_ura.as_str(),
        ),
    ] {
        require_non_empty(value, field)?;
        if !value.starts_with("easynet:///") {
            bail!("{field} must be an EasyNet URA");
        }
    }
    if campaign.caller_device_ura == campaign.provider_device_ura {
        bail!("campaign topology must use distinct caller and provider devices");
    }
    let raw = B64_STANDARD
        .decode(&campaign.challenge_nonce_b64)
        .context("campaign.challenge_nonce_b64 is invalid base64")?;
    raw.as_slice().try_into().map_err(|_| {
        anyhow!(
            "campaign.challenge_nonce_b64 decoded to {} bytes; expected 32",
            raw.len()
        )
    })
}

#[cfg(feature = "axon-pb")]
fn decode_campaign_arguments(proof: &FinalizationProof, index: usize) -> anyhow::Result<Vec<u8>> {
    let arguments = B64_STANDARD
        .decode(&proof.arguments_b64)
        .with_context(|| format!("proofs[{index}].arguments_b64 is invalid base64"))?;
    if arguments.len() > MAX_CAMPAIGN_ARGUMENTS_BYTES {
        bail!(
            "proofs[{index}].arguments_b64 decoded to {} bytes; maximum is {}",
            arguments.len(),
            MAX_CAMPAIGN_ARGUMENTS_BYTES
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&arguments)
        .with_context(|| format!("proofs[{index}].arguments_b64 is not JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("proofs[{index}] invocation arguments must be a JSON object"))?;
    let observed_session_id = object
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("proofs[{index}] invocation arguments omit session_id"))?;
    if observed_session_id != proof.session_id {
        bail!(
            "proofs[{index}] invocation arguments session_id {observed_session_id:?} does not match expected {:?}",
            proof.session_id
        );
    }
    Ok(arguments)
}

#[cfg(feature = "axon-pb")]
fn append_nonce_part(hash_input: &mut Vec<u8>, value: &[u8], field: &str) -> anyhow::Result<()> {
    let length = u32::try_from(value.len())
        .with_context(|| format!("{field} is too large for campaign nonce derivation"))?;
    hash_input.extend_from_slice(&length.to_be_bytes());
    hash_input.extend_from_slice(value);
    Ok(())
}

/// Derive the invocation nonce that makes a receipt single-use for one signed
/// RemoteApp campaign. Every variable-length part is length-prefixed so no two
/// tuples can share a byte representation.
#[cfg(feature = "axon-pb")]
fn derive_campaign_invocation_nonce(
    campaign: &ReceiptCampaignBinding,
    challenge: &[u8; 32],
    proof: &FinalizationProof,
) -> anyhow::Result<[u8; 16]> {
    let mut input = Vec::with_capacity(512);
    input.extend_from_slice(CAMPAIGN_NONCE_DOMAIN);
    for (field, value) in [
        ("campaign_id", campaign.campaign_id.as_bytes()),
        ("run_id", campaign.run_id.as_bytes()),
        ("challenge", challenge.as_slice()),
        ("domain_id", campaign.domain_id.as_bytes()),
        ("caller_device_ura", campaign.caller_device_ura.as_bytes()),
        (
            "provider_device_ura",
            campaign.provider_device_ura.as_bytes(),
        ),
        ("proof_id", proof.proof_id.as_bytes()),
        ("descriptor_ref", proof.descriptor_ref.as_bytes()),
        ("subject_ura", proof.subject_ura.as_bytes()),
        ("caller_ura", proof.caller_ura.as_bytes()),
        ("callee_ura", proof.callee_ura.as_bytes()),
        ("session_id", proof.session_id.as_bytes()),
    ] {
        append_nonce_part(&mut input, value, field)?;
    }
    let digest = axon_sdk::invocation::sha256(&input);
    let mut nonce = [0_u8; 16];
    nonce.copy_from_slice(&digest[..16]);
    Ok(nonce)
}

/// Verify a complete RemoteApp receipt proof set with Axon's canonical
/// finalization verifier. The JSON document supplies transport bytes and
/// expected bindings only; no caller-projected `verified` boolean is trusted.
#[cfg(feature = "axon-pb")]
pub fn verify_finalization_proof_set_with_resolver(
    proof_set: FinalizationProofSet,
    resolver: &dyn axon_sdk::invocation::KeyResolver,
) -> anyhow::Result<serde_json::Value> {
    if proof_set.schema != FINALIZATION_PROOF_SET_SCHEMA {
        bail!("receipt proof-set schema must be {FINALIZATION_PROOF_SET_SCHEMA:?}");
    }
    if proof_set.proofs.is_empty() {
        bail!("receipt proof set must contain at least one proof");
    }

    let challenge = decode_campaign_challenge(&proof_set.campaign)?;
    let mut seen_invocations = std::collections::BTreeSet::new();
    let mut seen_proof_ids = std::collections::BTreeSet::new();
    let mut verified = Vec::with_capacity(proof_set.proofs.len());
    for (index, proof) in proof_set.proofs.into_iter().enumerate() {
        require_non_empty(&proof.proof_id, &format!("proofs[{index}].proof_id"))?;
        if !seen_proof_ids.insert(proof.proof_id.clone()) {
            bail!("proofs[{index}] duplicates proof_id {:?}", proof.proof_id);
        }
        if proof.encoding != "prost.base64" {
            bail!(
                "proofs[{index}].encoding must be \"prost.base64\", got {:?}",
                proof.encoding
            );
        }
        if !seen_invocations.insert(proof.invocation_ura.clone()) {
            bail!(
                "proofs[{index}] duplicates invocation_ura {:?}",
                proof.invocation_ura
            );
        }
        for (field, value) in [
            ("invocation_ura", proof.invocation_ura.as_str()),
            ("descriptor_ref", proof.descriptor_ref.as_str()),
            ("subject_ura", proof.subject_ura.as_str()),
            ("caller_ura", proof.caller_ura.as_str()),
            ("callee_ura", proof.callee_ura.as_str()),
            ("session_id", proof.session_id.as_str()),
        ] {
            require_non_empty(value, &format!("proofs[{index}].{field}"))?;
        }
        let arguments = decode_campaign_arguments(&proof, index)?;
        let arguments_digest = axon_sdk::invocation::sha256(&arguments);
        let expected_nonce =
            derive_campaign_invocation_nonce(&proof_set.campaign, &challenge, &proof)?;
        let admission = decode_receipt(&proof.admission_receipt_b64, index, "admission")?;
        let terminal = decode_receipt(&proof.terminal_receipt_b64, index, "terminal")?;
        if terminal.ability_binding != proof.descriptor_ref {
            bail!(
                "proofs[{index}] terminal descriptor {:?} does not match expected {:?}",
                terminal.ability_binding,
                proof.descriptor_ref
            );
        }
        let checkpoints = crate::daemon::invocation::receipts::finalization_projection::verify_wire_finalization_checkpoints(
            admission,
            terminal,
            resolver,
        )
        .with_context(|| format!("proofs[{index}] Axon finalization verification failed"))?;
        let admission = checkpoints.admission();
        let terminal = checkpoints.terminal();
        if terminal.state() != axon_sdk::invocation::InvocationState::Completed {
            bail!(
                "proofs[{index}] terminal receipt must prove a successful Completed invocation, got {:?}",
                terminal.state()
            );
        }
        let binding = terminal.axiom_binding();
        if binding.payload_digest != arguments_digest {
            bail!(
                "proofs[{index}] verified receipt arguments digest {} does not match supplied arguments {}",
                hex::encode(binding.payload_digest),
                hex::encode(arguments_digest)
            );
        }
        if binding.invocation_nonce != expected_nonce {
            bail!(
                "proofs[{index}] verified invocation nonce {} is not derived from this campaign challenge; expected {}",
                hex::encode(binding.invocation_nonce),
                hex::encode(expected_nonce)
            );
        }
        if binding.subject.ura != proof.subject_ura {
            bail!(
                "proofs[{index}] verified subject {:?} does not match expected {:?}",
                binding.subject.ura,
                proof.subject_ura
            );
        }
        if binding.caller.ura != proof.caller_ura {
            bail!(
                "proofs[{index}] verified caller {:?} does not match expected {:?}",
                binding.caller.ura,
                proof.caller_ura
            );
        }
        if binding.callee.ura != proof.callee_ura {
            bail!(
                "proofs[{index}] verified callee {:?} does not match expected {:?}",
                binding.callee.ura,
                proof.callee_ura
            );
        }
        let invocation_ura = axon_sdk::ura::invocation_record_ura_for_binding(
            &binding.subject.ura,
            &binding.callee.ura,
            &binding.caller.ura,
            terminal.invocation_id(),
        )
        .ok_or_else(|| anyhow!("proofs[{index}] verified receipt binding has no Invocation URA"))?;
        if invocation_ura != proof.invocation_ura {
            bail!(
                "proofs[{index}] verified invocation URA {invocation_ura:?} does not match expected {:?}",
                proof.invocation_ura
            );
        }
        let ability_ura =
            axon_sdk::invocation::ability_ura_from_descriptor_ref(&proof.descriptor_ref)
                .with_context(|| format!("proofs[{index}] descriptor_ref is invalid"))?;
        let ability_name = axon_sdk::ura::qualified_ability_name(ability_ura).ok_or_else(|| {
            anyhow!("proofs[{index}] descriptor_ref has no qualified ability name")
        })?;
        verified.push(serde_json::json!({
            "proof_id": proof.proof_id,
            "invocation_ura": invocation_ura,
            "invocation_id": terminal.invocation_id(),
            "descriptor_ref": proof.descriptor_ref,
            "ability_name": ability_name,
            "subject_ura": binding.subject.ura,
            "caller_ura": binding.caller.ura,
            "callee_ura": binding.callee.ura,
            "session_id": proof.session_id,
            "arguments_sha256": format!("sha256:{}", hex::encode(arguments_digest)),
            "campaign_invocation_nonce": hex::encode(binding.invocation_nonce),
            "admission_receipt_hash": format!("sha256:{}", hex::encode(admission.self_hash())),
            "terminal_receipt_hash": format!("sha256:{}", hex::encode(terminal.self_hash())),
            "cryptographic_verification": "axon_finalization_checkpoints_verified",
        }));
    }
    Ok(serde_json::json!({
        "schema": FINALIZATION_PROOF_SET_SCHEMA,
        "status": "verified",
        "verification_scope": "all_campaign_bound_admission_and_terminal_checkpoints",
        "campaign": {
            "campaign_id": proof_set.campaign.campaign_id,
            "run_id": proof_set.campaign.run_id,
            "challenge_nonce_b64": proof_set.campaign.challenge_nonce_b64,
            "domain_id": proof_set.campaign.domain_id,
            "caller_device_ura": proof_set.campaign.caller_device_ura,
            "provider_device_ura": proof_set.campaign.provider_device_ura,
        },
        "proof_count": verified.len(),
        "proofs": verified,
    }))
}

#[cfg(feature = "axon-pb")]
pub fn verify_finalization_proof_set_at_daemon(
    proof_set: FinalizationProofSet,
    daemon_endpoint: std::path::PathBuf,
) -> anyhow::Result<serde_json::Value> {
    let resolver = crate::support::platform::local_daemon_grpc::CanonicalRuntimeReceiptResolver::for_daemon_endpoint(
        daemon_endpoint,
    );
    verify_finalization_proof_set_with_resolver(proof_set, &resolver)
}

/// CLI-local receipt-chain verification state.
///
/// Invariant 1: `NotPerformed` means this process did not perform offline
/// verification. It is not equivalent to "verification failed".
///
/// Invariant 2: ledger-projected verification remains a separate field because
/// it describes what the daemon persisted, not what this CLI process proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliReceiptChainVerification {
    NotPerformed,
}

impl CliReceiptChainVerification {
    /// State emitted by current CLI surfaces until a real verifier is wired.
    pub const fn not_performed() -> Self {
        Self::NotPerformed
    }

    /// Stable operator-facing label used in table/TUI renderers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotPerformed => "not_performed",
        }
    }
}

impl fmt::Display for CliReceiptChainVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod campaign_tests {
    use super::*;

    fn campaign(challenge: u8) -> ReceiptCampaignBinding {
        ReceiptCampaignBinding {
            campaign_id: "11111111-1111-4111-8111-111111111111".to_string(),
            run_id: "22222222-2222-4222-8222-222222222222".to_string(),
            challenge_nonce_b64: B64_STANDARD.encode([challenge; 32]),
            domain_id: "input_injection".to_string(),
            caller_device_ura: "easynet:///r/test/device/caller".to_string(),
            provider_device_ura: "easynet:///r/test/device/provider".to_string(),
        }
    }

    fn proof(arguments: &[u8]) -> FinalizationProof {
        FinalizationProof {
            proof_id: "focus-target".to_string(),
            invocation_ura: "easynet:///r/test/invocation/01".to_string(),
            descriptor_ref: format!(
                "easynet:///r/test/ability/system-agent.device.remote_desktop.focus_target@1.0.0#{}",
                "4".repeat(64)
            ),
            subject_ura: "easynet:///r/test/resource/window.1".to_string(),
            caller_ura: "easynet:///r/test/user/caller".to_string(),
            callee_ura: "easynet:///r/test/agent/device.provider.remote-desktop".to_string(),
            session_id: "rd-signed-campaign".to_string(),
            arguments_b64: B64_STANDARD.encode(arguments),
            encoding: "prost.base64".to_string(),
            admission_receipt_b64: B64_STANDARD.encode(b"admission"),
            terminal_receipt_b64: B64_STANDARD.encode(b"terminal"),
        }
    }

    #[test]
    fn campaign_nonce_matches_cross_language_vector_and_changes_with_challenge() {
        let arguments = br#"{"session_id":"rd-signed-campaign"}"#;
        let proof = proof(arguments);
        let first_campaign = campaign(b'n');
        let first_challenge = decode_campaign_challenge(&first_campaign).unwrap();
        let nonce =
            derive_campaign_invocation_nonce(&first_campaign, &first_challenge, &proof).unwrap();
        assert_eq!(hex::encode(nonce), "0473da408bad47ffe339c2ddab3cb725");

        let replacement_campaign = campaign(b'm');
        let replacement_challenge = decode_campaign_challenge(&replacement_campaign).unwrap();
        let replacement =
            derive_campaign_invocation_nonce(&replacement_campaign, &replacement_challenge, &proof)
                .unwrap();
        assert_ne!(nonce, replacement);
    }

    #[test]
    fn campaign_arguments_are_bound_to_the_declared_session() {
        let valid = proof(br#"{"session_id":"rd-signed-campaign"}"#);
        assert!(decode_campaign_arguments(&valid, 0).is_ok());

        let mismatched = proof(br#"{"session_id":"another-session"}"#);
        let error = decode_campaign_arguments(&mismatched, 0).unwrap_err();
        assert!(error.to_string().contains("does not match expected"));
    }
}
