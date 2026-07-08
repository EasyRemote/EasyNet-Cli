// EasyNet CLI — RFC-014 child invocation builder
// ===============================================
//
// File: src/daemon/invocation/admission/child_invocation_builder.rs
// Description: Canonical construction seam for carrier and composite child
//              invocations before target admission.
//
// Protocol Responsibility:
// Bind selected route facts, descriptor ref, subject, args hash, and origin
// authority without adding fields to the public Invocation primitive.
//
// Implementation Approach:
// Pure builder over typed inputs. It fails closed before dispatch when signed
// material is missing, route-selected descriptor facts drift, or public carrier
// input lacks origin caller / AuthorityProof authority.
//
// Usage Contract:
// Carrier, session, federation, and composite paths must provide selected
// route facts and authority material here instead of patching signed envelopes
// after route selection.
//
// Architectural Position:
// Daemon admission lower layer. It owns child construction diagnostics, not
// policy matching, transport dialing, handler dispatch, or receipt storage.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::authority_proof::AuthorityProof;
use super::decision::{PolicyDecisionReason, SignatureDecisionReason, TraceStage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildInvocationBuildInput {
    pub route: SelectedChildRoute,
    pub child_subject_ura: String,
    pub args: Vec<u8>,
    pub authority: ChildInvocationAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedChildRoute {
    pub route_ref: String,
    pub selected_callee_ura: String,
    pub execution_host_ura: Option<String>,
    pub public_ability: String,
    pub dispatch_key: String,
    pub descriptor_version: String,
    pub selected_descriptor_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildInvocationAuthority {
    ExternallySigned(ExternallySignedChildInvocation),
    OriginCaller(OriginCallerAuthority),
    AuthorityProof(AuthorityProof),
    DaemonInternalSystem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternallySignedChildInvocation {
    pub caller_ura: String,
    pub signed_callee_ura: String,
    pub signed_descriptor_ref: String,
    pub signed_subject_ura: String,
    pub canonical_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginCallerAuthority {
    pub caller_ura: String,
    pub principal_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltChildInvocation {
    pub caller_ura: String,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub descriptor_ref: String,
    pub descriptor_version: String,
    pub dispatch_key: String,
    pub route_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_host_ura: Option<String>,
    pub args_hash: String,
    pub authority_shape: ChildAuthorityShape,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_proof_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildAuthorityShape {
    ExternallySigned,
    OriginCaller,
    AuthorityProof,
    DaemonInternalSystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildInvocationBuildFailure {
    pub stage: TraceStage,
    pub code: ChildInvocationBuildFailureCode,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejector_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_reason: Option<SignatureDecisionReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_reason: Option<PolicyDecisionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChildInvocationBuildFailureCode {
    SignedEnvelopeRouteMutation,
    SignedDescriptorRefMissing,
    SignedDescriptorRefMismatch,
    MissingOriginCaller,
    AuthorityProofMissing,
    AuthorityProofMismatch,
    DescriptorBindingMissing,
    ChildSubjectMissing,
}

pub struct ChildInvocationBuilder;

impl ChildInvocationBuilder {
    pub fn build(
        input: ChildInvocationBuildInput,
    ) -> Result<BuiltChildInvocation, ChildInvocationBuildFailure> {
        validate_route(&input.route)?;
        if input.child_subject_ura.trim().is_empty() {
            return Err(failure(
                &input.route,
                TraceStage::PolicyDenied,
                ChildInvocationBuildFailureCode::ChildSubjectMissing,
                "child invocation subject is required",
                None,
                Some(PolicyDecisionReason::MissingGrant),
            ));
        }

        let ability_ura = ability_ura(&input.route)?;
        let args_hash = format!("sha256:{}", hex::encode(Sha256::digest(&input.args)));

        match input.authority {
            ChildInvocationAuthority::ExternallySigned(signed) => {
                validate_externally_signed(&input.route, &input.child_subject_ura, &signed)?;
                Ok(BuiltChildInvocation {
                    caller_ura: signed.caller_ura,
                    callee_ura: input.route.selected_callee_ura,
                    subject_ura: input.child_subject_ura,
                    ability_ura,
                    descriptor_ref: input.route.selected_descriptor_ref,
                    descriptor_version: input.route.descriptor_version,
                    dispatch_key: input.route.dispatch_key,
                    route_ref: input.route.route_ref,
                    execution_host_ura: input.route.execution_host_ura,
                    args_hash,
                    authority_shape: ChildAuthorityShape::ExternallySigned,
                    canonical_hash: Some(signed.canonical_hash),
                    authority_proof_id: None,
                })
            }
            ChildInvocationAuthority::OriginCaller(origin) => {
                if origin.caller_ura.trim().is_empty() || origin.principal_id.trim().is_empty() {
                    return Err(failure(
                        &input.route,
                        TraceStage::PolicyDenied,
                        ChildInvocationBuildFailureCode::MissingOriginCaller,
                        "public carrier child invocation requires origin caller authority",
                        None,
                        Some(PolicyDecisionReason::MissingOriginCaller),
                    ));
                }
                Ok(BuiltChildInvocation {
                    caller_ura: origin.caller_ura,
                    callee_ura: input.route.selected_callee_ura,
                    subject_ura: input.child_subject_ura,
                    ability_ura,
                    descriptor_ref: input.route.selected_descriptor_ref,
                    descriptor_version: input.route.descriptor_version,
                    dispatch_key: input.route.dispatch_key,
                    route_ref: input.route.route_ref,
                    execution_host_ura: input.route.execution_host_ura,
                    args_hash,
                    authority_shape: ChildAuthorityShape::OriginCaller,
                    canonical_hash: None,
                    authority_proof_id: None,
                })
            }
            ChildInvocationAuthority::AuthorityProof(proof) => {
                validate_authority_proof_binding(&input.route, &input.child_subject_ura, &proof)?;
                Ok(BuiltChildInvocation {
                    caller_ura: proof.principal_id.clone(),
                    callee_ura: input.route.selected_callee_ura,
                    subject_ura: input.child_subject_ura,
                    ability_ura,
                    descriptor_ref: input.route.selected_descriptor_ref,
                    descriptor_version: input.route.descriptor_version,
                    dispatch_key: input.route.dispatch_key,
                    route_ref: input.route.route_ref,
                    execution_host_ura: input.route.execution_host_ura,
                    args_hash,
                    authority_shape: ChildAuthorityShape::AuthorityProof,
                    canonical_hash: proof.canonical_hash.clone(),
                    authority_proof_id: Some(proof.proof_id),
                })
            }
            ChildInvocationAuthority::DaemonInternalSystem => Ok(BuiltChildInvocation {
                caller_ura: crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
                    .to_string(),
                callee_ura: input.route.selected_callee_ura,
                subject_ura: input.child_subject_ura,
                ability_ura,
                descriptor_ref: input.route.selected_descriptor_ref,
                descriptor_version: input.route.descriptor_version,
                dispatch_key: input.route.dispatch_key,
                route_ref: input.route.route_ref,
                execution_host_ura: input.route.execution_host_ura,
                args_hash,
                authority_shape: ChildAuthorityShape::DaemonInternalSystem,
                canonical_hash: None,
                authority_proof_id: None,
            }),
        }
    }
}

fn validate_route(route: &SelectedChildRoute) -> Result<(), ChildInvocationBuildFailure> {
    if route.selected_callee_ura.trim().is_empty()
        || route.public_ability.trim().is_empty()
        || route.dispatch_key.trim().is_empty()
        || route.descriptor_version.trim().is_empty()
        || route.selected_descriptor_ref.trim().is_empty()
    {
        return Err(failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::DescriptorBindingMissing,
            "selected route lacks callee, descriptor ref, version, public ability, or dispatch key",
            Some(SignatureDecisionReason::SignedDescriptorRefMissing),
            None,
        ));
    }
    let expected = crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        &route.selected_callee_ura,
        &route.public_ability,
        &route.descriptor_version,
    )
    .map_err(|err| {
        failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::SignedDescriptorRefMismatch,
            format!("selected route descriptor ref is invalid: {err}"),
            Some(SignatureDecisionReason::SignedDescriptorRefMismatch),
            None,
        )
    })?;
    if expected != route.selected_descriptor_ref {
        return Err(failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::SignedDescriptorRefMismatch,
            "selected descriptor ref does not match route callee, ability, and descriptor version",
            Some(SignatureDecisionReason::SignedDescriptorRefMismatch),
            None,
        ));
    }
    Ok(())
}

fn validate_externally_signed(
    route: &SelectedChildRoute,
    subject_ura: &str,
    signed: &ExternallySignedChildInvocation,
) -> Result<(), ChildInvocationBuildFailure> {
    if signed.signed_descriptor_ref.trim().is_empty() {
        return Err(failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::SignedDescriptorRefMissing,
            "externally signed child invocation lacks signed descriptor ref",
            Some(SignatureDecisionReason::SignedDescriptorRefMissing),
            None,
        ));
    }
    if signed.signed_callee_ura != route.selected_callee_ura
        || signed.signed_descriptor_ref != route.selected_descriptor_ref
        || signed.signed_subject_ura != subject_ura
    {
        return Err(failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::SignedEnvelopeRouteMutation,
            "route-selected child invocation mutated signed callee, descriptor ref, or subject",
            Some(SignatureDecisionReason::SignedEnvelopeRouteMutation),
            None,
        ));
    }
    Ok(())
}

fn validate_authority_proof_binding(
    route: &SelectedChildRoute,
    subject_ura: &str,
    proof: &AuthorityProof,
) -> Result<(), ChildInvocationBuildFailure> {
    let ability_ura = ability_ura(route)?;
    if proof.callee_ura != route.selected_callee_ura
        || proof.subject_ura != subject_ura
        || proof.ability_ura != ability_ura
        || proof.audience_ura
            != route
                .execution_host_ura
                .as_deref()
                .unwrap_or(route.selected_callee_ura.as_str())
    {
        return Err(failure(
            route,
            TraceStage::AuthorityDenied,
            ChildInvocationBuildFailureCode::AuthorityProofMismatch,
            "authority proof does not bind selected callee, subject, ability, or audience",
            None,
            Some(PolicyDecisionReason::AuthorityProofMismatch),
        ));
    }
    Ok(())
}

fn ability_ura(route: &SelectedChildRoute) -> Result<String, ChildInvocationBuildFailure> {
    crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &route.selected_descriptor_ref,
    )
    .map_err(|err| {
        failure(
            route,
            TraceStage::SignatureDenied,
            ChildInvocationBuildFailureCode::SignedDescriptorRefMismatch,
            format!("selected descriptor ref cannot project ability URA: {err}"),
            Some(SignatureDecisionReason::SignedDescriptorRefMismatch),
            None,
        )
    })
}

fn failure(
    route: &SelectedChildRoute,
    stage: TraceStage,
    code: ChildInvocationBuildFailureCode,
    reason: impl Into<String>,
    signature_reason: Option<SignatureDecisionReason>,
    policy_reason: Option<PolicyDecisionReason>,
) -> ChildInvocationBuildFailure {
    ChildInvocationBuildFailure {
        stage,
        code,
        reason: reason.into(),
        route_ref: (!route.route_ref.trim().is_empty()).then(|| route.route_ref.clone()),
        rejector_ura: (!route.selected_callee_ura.trim().is_empty())
            .then(|| route.selected_callee_ura.clone()),
        signature_reason,
        policy_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
    use crate::daemon::invocation::admission::decision::{AccessAction, PrincipalKind};

    fn route() -> SelectedChildRoute {
        let callee = "easynet:///r/test/device/dev-a".to_string();
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                &callee,
                "meta.list_resources",
                DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            )
            .expect("descriptor");
        SelectedChildRoute {
            route_ref: "route-ref::test".to_string(),
            selected_callee_ura: callee,
            execution_host_ura: Some("easynet:///r/test/device/dev-a".to_string()),
            public_ability: "meta.list_resources".to_string(),
            dispatch_key: "meta.list_resources".to_string(),
            descriptor_version: DEFAULT_ABILITY_DESCRIPTOR_VERSION.to_string(),
            selected_descriptor_ref: descriptor_ref,
        }
    }

    #[test]
    fn externally_signed_child_rejects_route_mutation() {
        let err = ChildInvocationBuilder::build(ChildInvocationBuildInput {
            route: route(),
            child_subject_ura: "easynet:///r/test/device/dev-a".to_string(),
            args: b"{}".to_vec(),
            authority: ChildInvocationAuthority::ExternallySigned(
                ExternallySignedChildInvocation {
                    caller_ura: "easynet:///r/test/user/alice".to_string(),
                    signed_callee_ura: "easynet:///r/test/device/other".to_string(),
                    signed_descriptor_ref: route().selected_descriptor_ref,
                    signed_subject_ura: "easynet:///r/test/device/dev-a".to_string(),
                    canonical_hash: "sha256:abc".to_string(),
                },
            ),
        })
        .expect_err("route mutation must fail");
        assert_eq!(
            err.code,
            ChildInvocationBuildFailureCode::SignedEnvelopeRouteMutation
        );
        assert_eq!(
            err.signature_reason,
            Some(SignatureDecisionReason::SignedEnvelopeRouteMutation)
        );
    }

    #[test]
    fn authority_proof_child_binds_selected_route() {
        let built = ChildInvocationBuilder::build(ChildInvocationBuildInput {
            route: route(),
            child_subject_ura: "easynet:///r/test/device/dev-a".to_string(),
            args: br#"{"limit":10}"#.to_vec(),
            authority: ChildInvocationAuthority::AuthorityProof(AuthorityProof {
                proof_id: "proof-1".to_string(),
                grant_id: Some("grant-1".to_string()),
                permission_request_id: None,
                owner_user_id: "alice".to_string(),
                principal_kind: PrincipalKind::Token,
                principal_id: "token-principal".to_string(),
                token_id: Some("token-1".to_string()),
                callee_ura: "easynet:///r/test/device/dev-a".to_string(),
                subject_ura: "easynet:///r/test/device/dev-a".to_string(),
                ability_ura:
                    crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                        &route().selected_descriptor_ref,
                    )
                    .unwrap(),
                action: AccessAction::Read,
                nonce: None,
                canonical_hash: Some("sha256:abc".to_string()),
                session_id: None,
                session_owner_user_id: None,
                allowed_followup_abilities: vec![],
                session_expires_at: None,
                issued_at: "2026-07-09T00:00:00Z".to_string(),
                expires_at: "2026-07-09T00:05:00Z".to_string(),
                issuer_ura: "easynet:///r/test/user/alice".to_string(),
                audience_ura: "easynet:///r/test/device/dev-a".to_string(),
                signature: "ed25519:test".to_string(),
            }),
        })
        .expect("authority proof child");
        assert_eq!(built.authority_shape, ChildAuthorityShape::AuthorityProof);
        assert_eq!(built.authority_proof_id.as_deref(), Some("proof-1"));
    }
}
