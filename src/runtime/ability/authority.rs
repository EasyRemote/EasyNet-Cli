// EasyNet CLI - AuthorityBinding registry facts
// =============================================
//
// File: src/runtime/ability/authority.rs
// Description: Daemon-local governance predicates for ability advertisement
//              and invocation. Axon owns the proof envelope; EasyNet-Cli owns
//              the local policy source and binding decision.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::descriptor::{
    canonical_json_bytes, is_valid_ability_name, is_valid_descriptor_version, sha256_bytes,
    unique_record_for, AbilityDescriptorKey, CallMode,
};
use super::AbilityControlPlaneError;

const DEFAULT_INVOKE_POLICY_REF: &str = "ability_access_policy";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityPredicate {
    governs_advertise: bool,
    governs_invoke: bool,
}

impl AuthorityPredicate {
    pub fn advertise_and_invoke() -> Self {
        Self {
            governs_advertise: true,
            governs_invoke: true,
        }
    }

    pub fn governs_advertise(&self) -> bool {
        self.governs_advertise
    }

    pub fn governs_invoke(&self) -> bool {
        self.governs_invoke
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityScope {
    /// Human-readable owner plane: device, hub, agent:<id>, user:<id>, plugin:<id>.
    ///
    /// This is intentionally not a protocol owner. The protocol owner is the
    /// callee/owner URA resolved during advertisement/invocation.
    owner_projection: String,
    /// URA or stable local authority root that backs the binding. Before join
    /// this may be a local marker; once advertised externally it must resolve
    /// to a routable URA.
    authority_root: String,
}

impl AuthorityScope {
    pub fn new(
        owner_projection: impl Into<String>,
        authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let owner_projection = owner_projection.into();
        let authority_root = authority_root.into();
        if owner_projection.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityOwnerProjection);
        }
        if authority_root.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityRoot);
        }
        Ok(Self {
            owner_projection,
            authority_root,
        })
    }

    pub fn owner_projection(&self) -> &str {
        &self.owner_projection
    }

    pub fn authority_root(&self) -> &str {
        &self.authority_root
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityBindingKind {
    #[serde(rename = "self")]
    SelfBinding,
    HostedAgentDelegation,
}

impl AuthorityBindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfBinding => "self",
            Self::HostedAgentDelegation => "hosted_agent_delegation",
        }
    }
}

/// Signed local control claim that allows the host process to act for one
/// hosted Agent on administrative ability surfaces.
///
/// Invariant 1: every field that dispatch later compares with the invocation
/// envelope is inside the signed canonical payload.
///
/// Invariant 2: this is local control metadata only. Public transport
/// admission rejects the metadata key before this verifier can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedAgentDelegationClaims {
    kind: String,
    agent_ura: String,
    signing_authority: String,
    wire_caller_ura: String,
    wire_callee_ura: String,
    wire_subject_ura: String,
    ability: String,
}

impl HostedAgentDelegationClaims {
    pub fn new(
        agent_ura: impl Into<String>,
        signing_authority: impl Into<String>,
        wire_caller_ura: impl Into<String>,
        wire_callee_ura: impl Into<String>,
        wire_subject_ura: impl Into<String>,
        ability: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let claims = Self {
            kind: "hosted_agent".to_string(),
            agent_ura: agent_ura.into(),
            signing_authority: signing_authority.into(),
            wire_caller_ura: wire_caller_ura.into(),
            wire_callee_ura: wire_callee_ura.into(),
            wire_subject_ura: wire_subject_ura.into(),
            ability: ability.into(),
        };
        claims.validate_non_empty()?;
        Ok(claims)
    }

    pub fn signing_payload_bytes(&self, signer_ura: &str) -> Vec<u8> {
        canonical_json_bytes(&json!({
            "claims": self,
            "signer_ura": signer_ura,
        }))
    }

    pub fn signed_metadata_value(
        &self,
        signer_ura: &str,
        signature: &Signature,
    ) -> anyhow::Result<String> {
        let token = SignedHostedAgentDelegation {
            claims: self.clone(),
            signer_ura: signer_ura.to_string(),
            signature_b64: BASE64_STANDARD.encode(signature.to_bytes()),
        };
        serde_json::to_string(&token)
            .map_err(|err| anyhow::anyhow!("encode hosted-agent delegation token: {err}"))
    }

    fn validate_non_empty(&self) -> anyhow::Result<()> {
        if self.kind.trim().is_empty()
            || self.agent_ura.trim().is_empty()
            || self.signing_authority.trim().is_empty()
            || self.wire_caller_ura.trim().is_empty()
            || self.wire_callee_ura.trim().is_empty()
            || self.wire_subject_ura.trim().is_empty()
            || self.ability.trim().is_empty()
        {
            anyhow::bail!("hosted-agent delegation claims fields must be non-empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SignedHostedAgentDelegation {
    claims: HostedAgentDelegationClaims,
    signer_ura: String,
    signature_b64: String,
}

/// Product-level authority fact for a local hosted-agent call.
///
/// Axon proves the invocation envelope. EasyNet proves that an operator
/// invoking through the host device is allowed to act for one hosted Agent
/// on local administrative surfaces such as `meta.teach` and `meta.acquire`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentDelegationContext {
    agent_ura: String,
    signing_authority: String,
    wire_caller_ura: String,
    wire_callee_ura: String,
    wire_subject_ura: String,
    ability: String,
}

impl HostedAgentDelegationContext {
    const HOST_DEVICE_SIGNING_AUTHORITY: &'static str = "host_device";

    pub fn from_signed_metadata(
        raw: &str,
        envelope_caller: &str,
        envelope_callee: &str,
        envelope_subject: &str,
        envelope_ability: &str,
        verifying_key: VerifyingKey,
    ) -> anyhow::Result<Self> {
        let token: SignedHostedAgentDelegation = serde_json::from_str(raw)
            .map_err(|err| anyhow::anyhow!("invalid hosted-agent delegation token JSON: {err}"))?;
        token.claims.validate_non_empty()?;
        let signer_ura = token.signer_ura.trim();
        if signer_ura.is_empty() {
            anyhow::bail!("hosted-agent delegation token signer_ura must be non-empty");
        }
        if signer_ura != envelope_caller {
            anyhow::bail!(
                "hosted-agent delegation signer {signer_ura:?} does not match envelope caller {envelope_caller:?}"
            );
        }
        let signature_bytes = BASE64_STANDARD
            .decode(token.signature_b64.as_bytes())
            .map_err(|err| anyhow::anyhow!("decode hosted-agent delegation signature: {err}"))?;
        let signature_array: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!(
                "hosted-agent delegation signature must be 64 bytes, got {}",
                signature_bytes.len()
            )
        })?;
        let signature = Signature::from_bytes(&signature_array);
        verifying_key
            .verify(&token.claims.signing_payload_bytes(signer_ura), &signature)
            .map_err(|err| anyhow::anyhow!("hosted-agent delegation signature invalid: {err}"))?;
        Self::from_bound_claims(
            token.claims,
            envelope_caller,
            envelope_callee,
            envelope_subject,
            envelope_ability,
        )
    }

    fn from_bound_claims(
        claims: HostedAgentDelegationClaims,
        envelope_caller: &str,
        envelope_callee: &str,
        envelope_subject: &str,
        envelope_ability: &str,
    ) -> anyhow::Result<Self> {
        if claims.kind != "hosted_agent" {
            anyhow::bail!(
                "hosted-agent delegation token uses unsupported kind {:?}",
                claims.kind
            );
        }
        let agent_ura = claims.agent_ura.trim();
        let signing_authority = claims.signing_authority.trim();
        let wire_caller_ura = claims.wire_caller_ura.trim();
        let wire_callee_ura = claims.wire_callee_ura.trim();
        let wire_subject_ura = claims.wire_subject_ura.trim();
        let ability = claims.ability.trim();
        if agent_ura.is_empty()
            || signing_authority.is_empty()
            || wire_caller_ura.is_empty()
            || wire_callee_ura.is_empty()
            || wire_subject_ura.is_empty()
            || ability.is_empty()
        {
            anyhow::bail!("hosted-agent delegation metadata fields must be non-empty");
        }
        let parsed_agent = crate::ura::parse_ura(agent_ura)
            .map_err(|err| anyhow::anyhow!("invalid hosted-agent delegation agent_ura: {err}"))?;
        if parsed_agent.kind != crate::ura::URAKind::Agent {
            anyhow::bail!("hosted-agent delegation agent_ura must be an Agent URA");
        }
        if wire_caller_ura != envelope_caller
            || wire_callee_ura != envelope_callee
            || wire_subject_ura != envelope_subject
            || ability != envelope_ability
        {
            anyhow::bail!(
                "hosted-agent delegation metadata does not match the signed invocation envelope"
            );
        }
        Ok(Self {
            agent_ura: agent_ura.to_string(),
            signing_authority: signing_authority.to_string(),
            wire_caller_ura: wire_caller_ura.to_string(),
            wire_callee_ura: wire_callee_ura.to_string(),
            wire_subject_ura: wire_subject_ura.to_string(),
            ability: ability.to_string(),
        })
    }

    pub fn authorize(
        &self,
        expected_agent_ura: &str,
        persisted_signing_authority: &str,
        expected_ability: &str,
    ) -> anyhow::Result<HostedAgentAuthority> {
        if self.agent_ura != expected_agent_ura {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority targets {}, expected {}",
                self.agent_ura,
                expected_agent_ura
            );
        }
        let delegated_public_ability = public_ability_from_descriptor_ref(&self.ability)?;
        if delegated_public_ability != expected_ability {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority was issued for {}, expected {expected_ability}",
                self.ability
            );
        }
        if self.wire_caller_ura.trim().is_empty() || self.wire_subject_ura.trim().is_empty() {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority is missing bound caller or subject"
            );
        }
        if self.signing_authority != Self::HOST_DEVICE_SIGNING_AUTHORITY {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority uses unsupported signing_authority {:?}",
                self.signing_authority
            );
        }
        let parsed_host = crate::ura::parse_ura(&self.wire_callee_ura).map_err(|err| {
            anyhow::anyhow!("{expected_ability} hosted-agent host URA is invalid: {err}")
        })?;
        if parsed_host.kind != crate::ura::URAKind::Device {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority host must be a Device URA, got {:?}",
                parsed_host.kind
            );
        }
        let expected_host_authority = format!("hosted_by:{}", self.wire_callee_ura);
        if persisted_signing_authority != expected_host_authority {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority host {} does not match persisted authority {}",
                self.wire_callee_ura,
                persisted_signing_authority
            );
        }
        Ok(HostedAgentAuthority {
            agent_ura: self.agent_ura.clone(),
            host_device_ura: self.wire_callee_ura.clone(),
            ability: delegated_public_ability,
            binding_kind: AuthorityBindingKind::HostedAgentDelegation,
        })
    }
}

fn public_ability_from_descriptor_ref(descriptor_ref: &str) -> anyhow::Result<String> {
    let ability_ura =
        easynet_axon::invocation::axiom::ability_ura_from_descriptor_ref(descriptor_ref)
            .map_err(|err| anyhow::anyhow!("invalid delegated ability descriptor ref: {err}"))?;
    let selector = crate::ura::AbilitySelector::parse(ability_ura)?;
    Ok(selector.public_name().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentAuthority {
    agent_ura: String,
    host_device_ura: String,
    ability: String,
    binding_kind: AuthorityBindingKind,
}

impl HostedAgentAuthority {
    pub fn agent_ura(&self) -> &str {
        &self.agent_ura
    }

    pub fn host_device_ura(&self) -> &str {
        &self.host_device_ura
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn binding_kind(&self) -> AuthorityBindingKind {
        self.binding_kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBindingRecord {
    ability: String,
    descriptor_version: String,
    call_mode: CallMode,
    scope: AuthorityScope,
    predicate: AuthorityPredicate,
    binding_kind: AuthorityBindingKind,
    invoke_policy_ref: String,
    invoke_policy_hash: [u8; 32],
}

impl AuthorityBindingRecord {
    pub fn local_self(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        scope: AuthorityScope,
    ) -> Result<Self, AbilityControlPlaneError> {
        Self::local_self_with_manifest_policy(ability, descriptor_version, call_mode, scope, None)
    }

    pub fn local_self_with_manifest_policy(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        scope: AuthorityScope,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability = ability.into();
        let descriptor_version = descriptor_version.into();
        if ability.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityAbility);
        }
        if !is_valid_ability_name(&ability) {
            return Err(AbilityControlPlaneError::InvalidAuthorityAbility { ability });
        }
        if descriptor_version.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityDescriptorVersion);
        }
        if !is_valid_descriptor_version(&descriptor_version) {
            return Err(
                AbilityControlPlaneError::InvalidAuthorityDescriptorVersion {
                    version: descriptor_version,
                },
            );
        }
        Ok(Self {
            ability,
            descriptor_version,
            call_mode,
            scope,
            predicate: AuthorityPredicate::advertise_and_invoke(),
            binding_kind: AuthorityBindingKind::SelfBinding,
            invoke_policy_ref: DEFAULT_INVOKE_POLICY_REF.to_string(),
            invoke_policy_hash: invoke_policy_hash_for_manifest(manifest),
        })
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn descriptor_version(&self) -> &str {
        &self.descriptor_version
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn scope(&self) -> &AuthorityScope {
        &self.scope
    }

    pub fn predicate(&self) -> &AuthorityPredicate {
        &self.predicate
    }

    pub fn binding_kind(&self) -> AuthorityBindingKind {
        self.binding_kind
    }

    pub fn invoke_policy_ref(&self) -> &str {
        &self.invoke_policy_ref
    }

    pub fn invoke_policy_hash(&self) -> [u8; 32] {
        self.invoke_policy_hash
    }

    pub fn invoke_policy_hash_prefixed_hex(&self) -> String {
        format!("sha256:{}", hex::encode(self.invoke_policy_hash))
    }

    pub fn key(&self) -> AbilityDescriptorKey {
        AbilityDescriptorKey::from_validated_parts(
            self.ability.clone(),
            self.descriptor_version.clone(),
            self.call_mode,
        )
    }
}

fn invoke_policy_hash_for_manifest(
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> [u8; 32] {
    let access = manifest
        .map(crate::core::ability_spec::AbilityManifest::access)
        .unwrap_or_default();
    let payload = serde_json::json!({
        "policy_ref": DEFAULT_INVOKE_POLICY_REF,
        "access": access,
    });
    sha256_bytes(&canonical_json_bytes(&payload))
}

#[derive(Debug, Default, Clone)]
pub struct AuthorityBindingRegistry {
    bindings: BTreeMap<AbilityDescriptorKey, AuthorityBindingRecord>,
}

impl AuthorityBindingRegistry {
    pub fn bind(&mut self, binding: AuthorityBindingRecord) {
        self.bindings.insert(binding.key(), binding);
    }

    pub fn remove(&mut self, ability: &str) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|key, _| key.ability() != ability);
        self.bindings.len() != before
    }

    pub fn get(&self, ability: &str) -> Option<&AuthorityBindingRecord> {
        self.get_version(ability, super::DEFAULT_ABILITY_DESCRIPTOR_VERSION)
    }

    pub fn get_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Option<&AuthorityBindingRecord> {
        self.get_version_for_mode(
            ability,
            super::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
        )
    }

    pub fn get_version(
        &self,
        ability: &str,
        descriptor_version: &str,
    ) -> Option<&AuthorityBindingRecord> {
        unique_record_for(&self.bindings, ability, descriptor_version)
    }

    pub fn get_version_for_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Option<&AuthorityBindingRecord> {
        let key = AbilityDescriptorKey::new(ability, descriptor_version, call_mode).ok()?;
        self.bindings.get(&key)
    }

    pub fn contains(&self, ability: &str) -> bool {
        self.bindings.keys().any(|key| key.ability() == ability)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_DEVICE_URA: &str = "easynet:///r/default/device/local";

    #[test]
    fn authority_predicate_covers_advertise_and_invoke() {
        let record = AuthorityBindingRecord::local_self(
            "fs.read",
            "1.0.0",
            CallMode::Rpc,
            AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
        )
        .unwrap();
        assert!(record.predicate().governs_advertise());
        assert!(record.predicate().governs_invoke());
    }

    #[test]
    fn authority_scope_rejects_empty_values_without_panicking() {
        assert_eq!(
            AuthorityScope::new("", LOCAL_DEVICE_URA).unwrap_err(),
            AbilityControlPlaneError::EmptyAuthorityOwnerProjection
        );
        assert_eq!(
            AuthorityScope::new("device", " ").unwrap_err(),
            AbilityControlPlaneError::EmptyAuthorityRoot
        );
        assert_eq!(
            AuthorityBindingRecord::local_self(
                "bad/name",
                "1.0.0",
                CallMode::Rpc,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityAbility {
                ability: "bad/name".to_string()
            }
        );
        assert_eq!(
            AuthorityBindingRecord::local_self(
                "fs.read",
                "v1",
                CallMode::Rpc,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityDescriptorVersion {
                version: "v1".to_string()
            }
        );
    }

    #[test]
    fn invoke_policy_hash_changes_with_manifest_access() {
        let input = serde_json::json!({"type": "object"});
        let base = crate::core::ability_spec::AbilityManifest::new("quote", "quote", input.clone())
            .unwrap();
        let restricted = crate::core::ability_spec::AbilityManifest::new("quote", "quote", input)
            .unwrap()
            .with_access(crate::core::ability_spec::AccessPolicy {
                visibility: crate::core::ability_spec::Visibility::Selfish,
                allow_callers: None,
                deny_callers: None,
            })
            .unwrap();

        let base_record = AuthorityBindingRecord::local_self_with_manifest_policy(
            "mentor.quote",
            "1.0.0",
            CallMode::Rpc,
            AuthorityScope::new("agent:mentor", "agent_ura:mentor").unwrap(),
            Some(&base),
        )
        .unwrap();
        let restricted_record = AuthorityBindingRecord::local_self_with_manifest_policy(
            "mentor.quote",
            "1.0.0",
            CallMode::Rpc,
            AuthorityScope::new("agent:mentor", "agent_ura:mentor").unwrap(),
            Some(&restricted),
        )
        .unwrap();
        assert_ne!(
            base_record.invoke_policy_hash(),
            restricted_record.invoke_policy_hash(),
            "authority binding must prove which invoke access policy it governs"
        );
    }

    #[test]
    fn hosted_agent_delegation_token_verifies_bound_claims() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let caller = crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA;
        let callee = "easynet:///r/default/device/local";
        let subject = "easynet:///r/default/device/local";
        let ability = format!(
            "{}@1.0.0",
            crate::ura::owner_ability_ura(callee, "meta.acquire").unwrap()
        );
        let agent_ura = crate::ura::agent_ura("default", "u", "apprentice");
        let claims = HostedAgentDelegationClaims::new(
            agent_ura.as_str(),
            "host_device",
            caller,
            callee,
            subject,
            ability.as_str(),
        )
        .unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();

        let context = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            caller,
            callee,
            subject,
            ability.as_str(),
            signer.verifying_key(),
        )
        .unwrap();

        let authority = context
            .authorize(
                agent_ura.as_str(),
                "hosted_by:easynet:///r/default/device/local",
                "meta.acquire",
            )
            .unwrap();
        assert_eq!(authority.ability(), "meta.acquire");
    }

    #[test]
    fn hosted_agent_delegation_token_rejects_envelope_drift() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]);
        let caller = crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA;
        let agent_ura = crate::ura::agent_ura("default", "u", "apprentice");
        let ability = format!(
            "{}@1.0.0",
            crate::ura::owner_ability_ura("easynet:///r/default/device/local", "meta.acquire")
                .unwrap()
        );
        let claims = HostedAgentDelegationClaims::new(
            agent_ura,
            "host_device",
            caller,
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            ability.as_str(),
        )
        .unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();

        let err = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            caller,
            "easynet:///r/default/device/other",
            "easynet:///r/default/device/local",
            ability.as_str(),
            signer.verifying_key(),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("signed invocation envelope"),
            "{err}"
        );
    }
}
