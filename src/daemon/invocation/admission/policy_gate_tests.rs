use super::*;
use crate::cli::commands::test_support::HomeGuard;
use crate::daemon::invocation::admission::decision::{OwnerSource, PolicyDecisionReason};
use crate::daemon::invocation::admission::device_caller::{
    verify_device_invocation_purpose, DeviceInvocationPurposeScope,
};
use crate::daemon::invocation::admission::grant_matcher::{
    PermissionEffect, PermissionGrant, PermissionGrantLifetime, PermissionGrantState,
};
use crate::daemon::persistence::config::{save_credentials, Credentials};
use crate::daemon::trust::anchor::{TrustedAgent, TrustedPrincipalOwner};
use axon_sdk::pb::axon::v1::{AgentIdentity, SubjectIdentity};
use std::path::PathBuf;

fn identity(ura: &str) -> AgentIdentity {
    AgentIdentity {
        ura: ura.to_string(),
        profile: String::new(),
    }
}

fn verified_device_path(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    public_ability: &str,
    action: AccessAction,
    daemon_ura: Option<&str>,
) -> TrustedCallerPath {
    let purpose = verify_device_invocation_purpose(DeviceInvocationPurposeScope {
        caller_ura,
        callee_ura,
        subject_ura,
        public_ability,
        daemon_ura,
        action,
    })
    .expect("test invocation must have a valid Device purpose");
    TrustedCallerPath::DeviceCustody(purpose)
}

fn save_test_credentials() {
    save_credentials(&Credentials {
        node_id: "dev-1".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "https://127.0.0.1:50443".to_string(),
        realm: "test".to_string(),
        deploy_signature: String::new(),
        hub_api_base: Some("http://127.0.0.1:8080".to_string()),
        username: Some("alice".to_string()),
        user_id: Some("alice".to_string()),
        hub_pubkey_b64: None,
        hub_tls_ca_pem_b64: None,
        join_receipt_hash: Some("join-hash".to_string()),
    })
    .expect("save test credentials");
}

fn empty_anchor() -> RealmTrustAnchor {
    RealmTrustAnchor::default()
}

fn anchor_with_device_owner() -> RealmTrustAnchor {
    RealmTrustAnchor::from_parts_with_principal_owners(
        Vec::new(),
        vec![TrustedPrincipalOwner {
            principal_ura: "easynet:///r/test/device/dev-1".to_string(),
            owner_user_id: "alice".to_string(),
            owner_ura: "easynet:///r/test/user/alice".to_string(),
            added_at_unix_ms: 1,
        }],
        Vec::new(),
    )
    .expect("owner anchor")
}

fn anchor_with_hosted_agent_owner() -> RealmTrustAnchor {
    RealmTrustAnchor::from_parts_with_principal_owners(
        Vec::new(),
        vec![TrustedPrincipalOwner {
            principal_ura: "easynet:///r/test/agent/alice.worker".to_string(),
            owner_user_id: "alice".to_string(),
            owner_ura: "easynet:///r/test/user/alice".to_string(),
            added_at_unix_ms: 1,
        }],
        Vec::new(),
    )
    .expect("hosted agent owner anchor")
}

fn anchor_with_peer_realm() -> RealmTrustAnchor {
    RealmTrustAnchor::from_parts_with_principal_owners(
        vec![TrustedAgent {
            agent_ura: "easynet:///r/peer/authority".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::Hub,
            added_at_unix_ms: 1,
            origin_realm: Some("peer".to_string()),
            hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
            tls_ca_pem_path: Some(PathBuf::from("/tmp/peer-ca.pem")),
        }],
        Vec::new(),
        Vec::new(),
    )
    .expect("peer anchor")
}

#[test]
fn trusted_caller_path_classifies_federated_actor_kinds() {
    assert_eq!(
        TrustedCallerPath::from_federated_invocation_caller(
            "easynet:///r/peer/user/alice",
            "chat",
            None,
        )
        .expect("federated User caller"),
        TrustedCallerPath::User
    );
    assert_eq!(
        TrustedCallerPath::from_federated_invocation_caller(
            "easynet:///r/peer/agent/alice.worker",
            "chat",
            None,
        )
        .expect("federated Agent caller"),
        TrustedCallerPath::AgentDeviceCustody
    );
    assert_eq!(
        TrustedCallerPath::from_federated_invocation_caller(
            "easynet:///r/peer/authority",
            "chat",
            None,
        )
        .expect("federated Authority caller"),
        TrustedCallerPath::Hub
    );
}

#[test]
fn trusted_caller_path_requires_invocation_purpose_for_federated_device() {
    let caller = "easynet:///r/peer/device/dev-1";
    let authority = "easynet:///r/peer/authority";
    let ability = crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES;
    let TrustedCallerPath::DeviceCustody(purpose) = verified_device_path(
        caller,
        authority,
        caller,
        ability,
        AccessAction::Manage,
        Some(authority),
    ) else {
        unreachable!()
    };
    assert_eq!(
        TrustedCallerPath::from_federated_invocation_caller(caller, ability, Some(purpose),)
            .expect("verified federated Device custody requires publication purpose"),
        TrustedCallerPath::DeviceCustody(purpose)
    );
    let error = TrustedCallerPath::from_federated_invocation_caller(caller, "shell.run", None)
        .expect_err("ordinary public abilities must reject Device callers before policy");
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert!(error.message().contains("DEVICE_CALLER_PURPOSE_UNVERIFIED"));
}

#[test]
fn trusted_caller_path_rejects_non_actor_federated_callers() {
    let error = TrustedCallerPath::from_federated_invocation_caller(
        "easynet:///r/peer/resource/user.alice/pages",
        "chat",
        None,
    )
    .expect_err("resources are subjects, not caller actors");
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(
        error.message().contains("FEDERATED_CALLER_KIND_MISMATCH"),
        "{error:?}"
    );
}

#[test]
fn derived_child_path_accepts_only_callable_parent_actors() {
    assert_eq!(
        TrustedCallerPath::from_derived_child_caller(
            "easynet:///r/test/agent/device.dev-1.automation"
        )
        .expect("derived SystemAgent caller"),
        TrustedCallerPath::AgentDeviceCustody
    );
    assert_eq!(
        TrustedCallerPath::from_derived_child_caller("easynet:///r/test/service/alice.pages")
            .expect("derived Service caller"),
        TrustedCallerPath::AgentDeviceCustody
    );
    assert_eq!(
        TrustedCallerPath::from_derived_child_caller("easynet:///r/test/authority")
            .expect("derived Authority caller"),
        TrustedCallerPath::Hub
    );
    let error = TrustedCallerPath::from_derived_child_caller("easynet:///r/test/device/dev-1")
        .expect_err("Device is an execution host, not a derived callable actor");
    assert!(
        error
            .message()
            .contains("DERIVED_CHILD_CALLER_KIND_MISMATCH"),
        "{error:?}"
    );
}

fn anchor_with_hub_a_peer() -> RealmTrustAnchor {
    RealmTrustAnchor::from_parts_with_principal_owners(
        vec![TrustedAgent {
            agent_ura: "easynet:///r/hub-a.local/authority".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::Hub,
            added_at_unix_ms: 1,
            origin_realm: Some("hub-a.local".to_string()),
            hub_endpoint: Some("https://hub-a:50443".to_string()),
            tls_ca_pem_path: Some(PathBuf::from("/tmp/hub-a-ca.pem")),
        }],
        Vec::new(),
        Vec::new(),
    )
    .expect("hub-a peer anchor")
}

#[test]
fn trusted_device_subject_projects_anchor_owner() {
    let anchor = anchor_with_device_owner();
    let owner = resolve_owner(
        "easynet:///r/test/device/dev-1",
        "easynet:///r/test/device/dev-1",
        &anchor,
    )
    .expect("anchor owner resolution");

    assert_eq!(
        owner.owner_user_ura.as_deref(),
        Some("easynet:///r/test/user/alice")
    );
    assert_eq!(
        owner.owner_ura.as_deref(),
        Some("easynet:///r/test/user/alice")
    );
}

#[test]
fn system_agent_callee_projects_sponsor_devices_durable_user_owner() {
    let anchor = anchor_with_device_owner();
    let owner = resolve_owner(
        "easynet:///r/test/resource/device.dev-1/fs/tmp/archive.tar.gz",
        "easynet:///r/test/agent/device.dev-1.locomotion",
        &anchor,
    )
    .expect("SystemAgent sponsor owner resolution");

    assert_eq!(
        owner.owner_user_ura.as_deref(),
        Some("easynet:///r/test/user/alice")
    );
    assert_eq!(owner.owner_source, OwnerSource::Callee);
}

#[test]
fn system_agent_callee_without_durable_sponsor_owner_stays_unresolved() {
    let owner = resolve_owner(
        "easynet:///r/test/resource/device.dev-1/fs/tmp/archive.tar.gz",
        "easynet:///r/test/agent/device.dev-1.locomotion",
        &empty_anchor(),
    )
    .expect("missing sponsor owner must fail closed");

    assert!(owner.owner_user_ura.is_none());
    assert_eq!(owner.owner_source, OwnerSource::Unresolved);
}

#[test]
fn paired_device_subject_does_not_project_credentials_owner() {
    let _home = HomeGuard::new();
    save_test_credentials();
    let anchor = empty_anchor();
    let owner = resolve_owner(
        "easynet:///r/test/device/dev-1",
        "easynet:///r/test/device/dev-1",
        &anchor,
    )
    .expect("ordinary policy owner resolution must ignore local credentials");

    assert!(owner.owner_user_ura.is_none());
    assert!(owner.owner_ura.is_none());
    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
}

#[test]
fn device_principal_projection_ignores_malformed_local_credentials() {
    let _home = HomeGuard::new();
    let state_dir = crate::daemon::persistence::config::state_dir();
    std::fs::create_dir_all(&state_dir).expect("create isolated state dir");
    std::fs::write(state_dir.join("credentials.json"), b"{").expect("write malformed credentials");

    let principal = principal_for(
        verified_device_path(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/authority",
            "easynet:///r/test/device/dev-1",
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            Some("easynet:///r/test/authority"),
        ),
        "easynet:///r/test/device/dev-1",
        &empty_anchor(),
    )
    .expect("ordinary policy principal projection must not read local credentials");

    assert_eq!(principal.kind, PrincipalKind::DeviceCustody);
    assert_eq!(principal.caller_user_ura, None);
}

#[test]
fn device_principal_projection_does_not_inherit_owner_allow() {
    let principal = principal_for(
        verified_device_path(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/authority",
            "easynet:///r/test/device/dev-1",
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            Some("easynet:///r/test/authority"),
        ),
        "easynet:///r/test/device/dev-1",
        &anchor_with_device_owner(),
    )
    .expect("device principal projection");

    assert_eq!(principal.kind, PrincipalKind::DeviceCustody);
    assert_eq!(principal.token_class, None);
    assert_eq!(principal.caller_user_ura, None);
}

#[test]
fn hosted_agent_key_projects_agent_not_device_or_user_owner() {
    let principal = principal_for(
        TrustedCallerPath::AgentDeviceCustody,
        "easynet:///r/test/agent/alice.worker",
        &anchor_with_hosted_agent_owner(),
    )
    .expect("hosted agent principal projection");

    assert_eq!(principal.kind, PrincipalKind::Agent);
    assert_eq!(principal.id, "easynet:///r/test/agent/alice.worker");
    assert_eq!(principal.token_id, None);
    assert_eq!(principal.token_class, None);
    assert_eq!(principal.caller_user_ura, None);
}

#[test]
fn verified_caller_projection_separates_custody_path_from_principal_kind() {
    let trust_path = TrustedCallerPath::from_verified_invocation_caller(
        "easynet:///r/test/agent/alice.worker",
        VerifiedCallerEvidence::TrustAnchorRole(TrustAnchorRole::Device),
        "chat",
        None,
    )
    .expect("device trust-anchor role plus Agent caller lowers to AgentDeviceCustody");
    let verified = VerifiedCallerProjection::from_trusted_path(
        trust_path,
        "easynet:///r/test/agent/alice.worker".to_string(),
        &anchor_with_hosted_agent_owner(),
    )
    .expect("device-mediated hosted Agent caller must classify");

    assert_eq!(verified.trust_path, TrustedCallerPath::AgentDeviceCustody);
    assert_eq!(verified.principal.kind, PrincipalKind::Agent);
    assert_eq!(
        verified.principal.id,
        "easynet:///r/test/agent/alice.worker"
    );
}

#[test]
fn agent_device_custody_never_matches_device_publication_custody_scope() {
    let local_authority = "easynet:///r/test/authority";
    let hosted_agent = "easynet:///r/test/agent/alice.worker";
    let ability_ura = crate::core::ura::owner_ability_ura(
        local_authority,
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
    )
    .expect("authority ability");

    assert!(!device_publication_custody_manage_scope(
        hosted_agent,
        local_authority,
        hosted_agent,
        &ability_ura,
        Some(local_authority),
        TrustedCallerPath::AgentDeviceCustody,
        AccessAction::Manage,
    ));
}

#[test]
fn local_device_owner_resolution_ignores_malformed_credentials() {
    let _home = HomeGuard::new();
    let state_dir = crate::daemon::persistence::config::state_dir();
    std::fs::create_dir_all(&state_dir).expect("create isolated state dir");
    std::fs::write(state_dir.join("credentials.json"), b"{").expect("write malformed credentials");
    let anchor = empty_anchor();

    let owner = resolve_owner(
        "easynet:///r/test/device/dev-1",
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("ordinary policy owner resolution must not read local credentials");

    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
    assert!(owner.owner_user_ura.is_none());
}

#[test]
fn paired_device_ability_does_not_project_credentials_owner() {
    let _home = HomeGuard::new();
    save_test_credentials();
    let anchor = empty_anchor();
    let owner = resolve_owner(
        "easynet:///r/test/ability/device.dev-1.federation.advertise_abilities",
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("ordinary policy device ability owner resolution must ignore local credentials");

    assert!(owner.owner_user_ura.is_none());
    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
}

#[test]
fn device_owned_ability_subject_does_not_project_device_owner() {
    let anchor = anchor_with_device_owner();
    let device_owned_ability =
        crate::core::ura::owner_ability_ura("easynet:///r/test/device/dev-1", "node.describe")
            .expect("direct Device-owned Ability URA");
    let owner = resolve_owner(
        &device_owned_ability,
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("direct Device-owned Ability URA must fail closed as an ordinary policy subject");

    assert!(owner.owner_user_ura.is_none());
    assert!(owner.owner_ura.is_none());
    assert_eq!(owner.owner_source, OwnerSource::Unresolved);
}

#[test]
fn local_authority_ability_without_trust_owner_stays_unresolved() {
    let _home = HomeGuard::new();
    let anchor = empty_anchor();
    let owner = resolve_owner(
        "easynet:///r/test/ability/authority.federation.discover",
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("authority owner resolution");

    assert!(owner.owner_user_ura.is_none());
    assert!(owner.owner_ura.is_none());
    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
}

#[test]
fn authority_ability_does_not_project_paired_device_credentials_owner() {
    let _home = HomeGuard::new();
    save_test_credentials();
    let anchor = empty_anchor();
    let owner = resolve_owner(
        "easynet:///r/test/ability/authority.federation.discover",
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("authority owner resolution should not fail for saved device credentials");

    assert!(owner.owner_user_ura.is_none());
    assert!(owner.owner_ura.is_none());
    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
    assert!(
        owner
            .audit_warnings
            .iter()
            .any(|warning| warning.contains("no authoritative owner source")),
        "authority owner without explicit authority fact must stay unresolved: {owner:?}"
    );
}

#[test]
fn authority_subject_does_not_project_paired_device_credentials_owner() {
    let _home = HomeGuard::new();
    save_test_credentials();
    let anchor = empty_anchor();
    let owner = resolve_owner(
        "easynet:///r/test/authority",
        "easynet:///r/test/authority",
        &anchor,
    )
    .expect("authority subject resolution should not fail for saved device credentials");

    assert!(owner.owner_user_ura.is_none());
    assert!(owner.owner_ura.is_none());
    assert_eq!(
        owner.owner_source,
        crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
    );
}

#[test]
fn user_subject_projects_owner_policy_allow() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/user/alice")),
        callee: Some(identity("easynet:///r/test/agent/alice.worker")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/user/alice".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "meta.list_resources",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::User,
        daemon_ura: None,
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect("owner user must pass policy");
    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
}

#[test]
fn derived_user_authority_keeps_system_agent_caller_and_allows_owned_agent_child() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/agent/device.dev-1.automation")),
        callee: Some(identity("easynet:///r/test/agent/alice.worker")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/agent/alice.worker".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "chat",
        action: AccessAction::Invoke,
        safe_read: false,
        trusted_path: TrustedCallerPath::AgentDeviceCustody,
        daemon_ura: Some("easynet:///r/test/device/dev-1"),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:derived-child".to_string()),
        signature_key_id: Some("system-agent-key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: Some(
            PrincipalProjection::accountable_user("easynet:///r/test/user/alice")
                .expect("canonical accountable User"),
        ),
        rejector_ura: Some("easynet:///r/test/device/dev-1".to_string()),
    })
    .expect("parent User authority must admit its owned Agent child");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::OwnerAllow);
    assert_eq!(
        decision.caller_ura,
        "easynet:///r/test/agent/device.dev-1.automation"
    );
    assert_eq!(
        decision.principal_kind,
        crate::daemon::invocation::admission::decision::PrincipalKind::User
    );
    assert_eq!(decision.principal_id, "easynet:///r/test/user/alice");
}

#[test]
fn user_subject_owner_allow_is_realm_scoped() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/realm-a/user/alice")),
        callee: Some(identity("easynet:///r/realm-b/agent/alice.worker")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/realm-b/user/alice".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let error = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "meta.list_resources",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::User,
        daemon_ura: None,
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect_err("same user id in a different Realm must not receive OwnerAllow");

    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert!(
        error.message().contains("NON_INTERACTIVE_DENY"),
        "unexpected error: {error}"
    );
}

#[test]
fn hub_link_principal_gets_descriptor_safe_read_default() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity("easynet:///r/test/agent/alice.worker")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/user/alice".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "meta.list_resources",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: None,
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect("trusted hub-link principal may read descriptor-safe metadata");
    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
    assert_eq!(decision.principal_kind, PrincipalKind::Token);
    assert_eq!(
        decision.token_id.as_deref(),
        Some("easynet:///r/test/authority")
    );
}

#[test]
fn hosted_agent_cannot_owner_allow_through_host_key_custody() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let agent = "easynet:///r/test/agent/alice.worker";
    let envelope = Envelope {
        caller: Some(identity(agent)),
        callee: Some(identity(agent)),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/user/alice".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "remote_desktop.attach",
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::AgentDeviceCustody,
        daemon_ura: None,
        trust_anchor: &anchor_with_hosted_agent_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:hosted-agent-key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect_err("hosted Agent custody must not inherit user OwnerAllow");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("\"principal_kind\":\"agent\""),
        "expected Agent principal projection, got: {}",
        err.message()
    );
    assert!(
        err.message()
            .contains("\"reason\":\"NON_INTERACTIVE_DENY\""),
        "expected ordinary denial without explicit Agent grant, got: {}",
        err.message()
    );
}

#[test]
fn hosted_agent_explicit_grant_allows_without_user_owner_projection() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let agent = "easynet:///r/test/agent/alice.worker";
    let subject = "easynet:///r/test/user/alice";
    let ability = "remote_desktop.attach";
    let ability_ura = ability_ura_for(agent, ability).expect("agent ability URA");
    stores
        .with_store("easynet:///r/test/user/alice", |store| {
            store.create_grant(
                PermissionGrant {
                    grant_id: "agent-explicit-grant".to_string(),
                    owner_user_ura: "easynet:///r/test/user/alice".to_string(),
                    principal_kind: PrincipalKind::Agent,
                    principal_id: agent.to_string(),
                    token_id: None,
                    token_class: None,
                    session_id: None,
                    session_expires_at: None,
                    callee_ura: Some(agent.to_string()),
                    subject_ura_pattern: Some(subject.to_string()),
                    ability_ura_pattern: Some(ability_ura.clone()),
                    actions: vec![AccessAction::Stream],
                    constraints: None,
                    effect: PermissionEffect::Allow,
                    lifetime: PermissionGrantLifetime::Permanent,
                    state: PermissionGrantState::Active,
                    expires_at: None,
                    review_required_after: None,
                    last_reviewed_at: None,
                    last_used_at: None,
                    created_by: "easynet:///r/test/user/alice".to_string(),
                    created_at: "2026-08-07T00:00:00Z".to_string(),
                    updated_at: None,
                    revoked_at: None,
                    reason: Some("hosted Agent explicit grant regression".to_string()),
                },
                "easynet:///r/test/user/alice",
            )
        })
        .expect("open policy store")
        .expect("create explicit Agent grant");

    let envelope = Envelope {
        caller: Some(identity(agent)),
        callee: Some(identity(agent)),
        subject: Some(SubjectIdentity {
            ura: subject.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability,
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::AgentDeviceCustody,
        daemon_ura: None,
        trust_anchor: &anchor_with_hosted_agent_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:hosted-agent-key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect("explicit Agent grant must admit hosted Agent");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::ExplicitGrantAllow);
    assert_eq!(decision.principal_kind, PrincipalKind::Agent);
    assert_eq!(decision.grant_id.as_deref(), Some("agent-explicit-grant"));
    assert_eq!(
        decision.owner_user_ura.as_deref(),
        Some("easynet:///r/test/user/alice")
    );
}

#[test]
fn hosted_agent_once_grant_is_consumed_after_first_admission() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let agent = "easynet:///r/test/agent/alice.worker";
    let subject = "easynet:///r/test/user/alice";
    let ability = "remote_desktop.attach";
    let ability_ura = ability_ura_for(agent, ability).expect("agent ability URA");
    stores
        .with_store("easynet:///r/test/user/alice", |store| {
            store.create_grant(
                PermissionGrant {
                    grant_id: "agent-once-grant".to_string(),
                    owner_user_ura: "easynet:///r/test/user/alice".to_string(),
                    principal_kind: PrincipalKind::Agent,
                    principal_id: agent.to_string(),
                    token_id: None,
                    token_class: None,
                    session_id: None,
                    session_expires_at: None,
                    callee_ura: Some(agent.to_string()),
                    subject_ura_pattern: Some(subject.to_string()),
                    ability_ura_pattern: Some(ability_ura.clone()),
                    actions: vec![AccessAction::Stream],
                    constraints: None,
                    effect: PermissionEffect::Allow,
                    lifetime: PermissionGrantLifetime::Once,
                    state: PermissionGrantState::Active,
                    expires_at: None,
                    review_required_after: None,
                    last_reviewed_at: None,
                    last_used_at: None,
                    created_by: "easynet:///r/test/user/alice".to_string(),
                    created_at: "2026-08-07T00:00:00Z".to_string(),
                    updated_at: None,
                    revoked_at: None,
                    reason: Some("hosted Agent once grant regression".to_string()),
                },
                "easynet:///r/test/user/alice",
            )
        })
        .expect("open policy store")
        .expect("create once Agent grant");

    let envelope = Envelope {
        caller: Some(identity(agent)),
        callee: Some(identity(agent)),
        subject: Some(SubjectIdentity {
            ura: subject.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability,
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::AgentDeviceCustody,
        daemon_ura: None,
        trust_anchor: &anchor_with_hosted_agent_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:hosted-agent-key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect("first use of once grant must admit");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.grant_id.as_deref(), Some("agent-once-grant"));

    let consumed = stores
        .with_store("easynet:///r/test/user/alice", |store| {
            store.grant("agent-once-grant").cloned()
        })
        .expect("open policy store")
        .expect("grant exists");
    assert_eq!(consumed.state, PermissionGrantState::Expired);
    assert!(consumed.last_used_at.is_some());

    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability,
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::AgentDeviceCustody,
        daemon_ura: None,
        trust_anchor: &anchor_with_hosted_agent_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:hosted-agent-key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect_err("second use of once grant must fail closed");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message()
            .contains("\"reason\":\"NON_INTERACTIVE_DENY\""),
        "expected consumed once grant to stop matching, got: {}",
        err.message()
    );
}

#[test]
fn realm_authority_can_read_descriptor_safe_device_metadata_before_owner_binding() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let device = "easynet:///r/test/device/dev-1";
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity(device)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "node.describe",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(device),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(device.to_string()),
    })
    .expect("realm Authority must read public Device runtime metadata");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
    assert_eq!(decision.owner_source, OwnerSource::Unresolved);
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(decision.caller_ura, "easynet:///r/test/authority");
    assert_eq!(decision.callee_ura, device);
}

#[test]
fn realm_authority_public_read_does_not_admit_device_owned_ability_subject() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let device = "easynet:///r/test/device/dev-1";
    let device_owned_ability =
        crate::core::ura::owner_ability_ura(device, "node.describe").expect("Device ability URA");
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity(device)),
        subject: Some(SubjectIdentity {
            ura: device_owned_ability,
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "node.describe",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(device),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(device.to_string()),
    })
    .expect_err("Device-owned ability URAs are migration facts, not public read subjects");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("\"reason\":\"OWNER_UNRESOLVED\""),
        "expected owner resolution denial instead of Device-owned ability policy allow, got: {}",
        err.message()
    );
}

#[test]
fn realm_authority_public_device_read_stays_bound_to_local_device() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let local_device = "easynet:///r/test/device/local-dev";
    let other_device = "easynet:///r/test/device/other-dev";
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity(other_device)),
        subject: Some(SubjectIdentity {
            ura: other_device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "node.describe",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(local_device),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(local_device.to_string()),
    })
    .expect_err("authority public read must not target a different local daemon owner");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("\"reason\":\"OWNER_UNRESOLVED\""),
        "expected owner unresolved outside local daemon scope, got: {}",
        err.message()
    );
}

#[test]
fn local_authority_self_read_enters_policy_without_user_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let subject = crate::core::ura::owner_ability_ura(authority, "federation.discover")
        .expect("authority ability subject");
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: subject,
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "federation.discover",
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("local authority must read its descriptor-bound system catalog");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(decision.caller_ura, authority);
    assert_eq!(decision.callee_ura, authority);
}

#[test]
fn local_authority_descriptor_ref_self_read_enters_policy_without_user_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/hub/authority";
    let subject = crate::core::ura::owner_ability_ura(authority, "federation.discover")
        .expect("authority ability subject");
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
            authority,
            "federation.discover",
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("descriptor ref");
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: subject,
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: &descriptor_ref,
        action: AccessAction::Read,
        safe_read: true,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("local authority must read descriptor-bound system catalog");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/hub/ability/authority.federation.discover"
    );
}

#[test]
fn local_authority_self_stream_enters_policy_without_user_session_authority() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let subject = crate::core::ura::resource_dot_ura(
        "test",
        "authority",
        "invoke/federation.subscribe_directory_v2",
    );
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: subject,
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "federation.subscribe_directory_v2",
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("local authority must stream its own directory events without a user authority");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::SystemRuleAllow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.authority.self_stream")
    );
    assert!(decision.grant_id.is_none());
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/test/ability/authority.federation.subscribe_directory_v2"
    );
}

#[test]
fn trusted_peer_authority_can_stream_federation_directory_without_user_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let local_authority = "easynet:///r/hub-b.local/authority";
    let peer_authority = "easynet:///r/hub-a.local/authority";
    let envelope = Envelope {
        caller: Some(identity(peer_authority)),
        callee: Some(identity(local_authority)),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/hub-a.local/resource/hub.federation/directory/hub-b.local"
                .to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "federation.subscribe_directory_v2",
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(local_authority),
        trust_anchor: &anchor_with_hub_a_peer(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(local_authority.to_string()),
    })
    .expect("trusted peer authority should stream federation directory");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::SystemRuleAllow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.authority.peer_directory_stream")
    );
    assert!(decision.grant_id.is_none());
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/hub-b.local/ability/authority.federation.subscribe_directory_v2"
    );
}

#[test]
fn peer_directory_stream_matcher_denies_near_match_subject_without_grant_fallback() {
    let local_authority = "easynet:///r/hub-b.local/authority";
    let peer_authority = "easynet:///r/hub-a.local/authority";
    let ability_ura = crate::core::ura::owner_ability_ura(
        local_authority,
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    )
    .expect("directory ability URA");

    let matched = VerifiedAuthorityPeerDirectoryStream::classify(
        peer_authority,
        local_authority,
        "easynet:///r/hub-a.local/resource/hub.federation/directory/hub-b.local-extra",
        &ability_ura,
        Some(local_authority),
        TrustedCallerPath::Hub,
        AccessAction::Stream,
        &anchor_with_hub_a_peer(),
    );

    assert!(matches!(
        matched,
        AuthorityPeerDirectoryStreamMatch::Denied(
            "directory subject does not exactly bind caller and callee realms"
        )
    ));
}

#[test]
fn local_authority_self_manage_can_revoke_owned_device_directory_entry() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/device/dev-1".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "federation.revoke",
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(authority),
        trust_anchor: &anchor_with_device_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("realm authority must manage its own federation directory");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::SystemRuleAllow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.authority.self_manage")
    );
    assert!(decision.grant_id.is_none());
    assert_eq!(
        decision.owner_user_ura.as_deref(),
        Some("easynet:///r/test/user/alice")
    );
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/test/ability/authority.federation.revoke"
    );
}

#[test]
fn device_publication_custody_can_advertise_device_projection_without_user_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            action: AccessAction::Manage,
            safe_read: false,
            trusted_path: verified_device_path(
                device,
                authority,
                device,
                crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
                AccessAction::Manage,
                Some(authority),
            ),
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:device".to_string()),
            verified_authority_id: None,
            verified_session_id: None,
        accountable_principal: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("device must advertise its own authority-scoped publication after federation join");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::SystemRuleAllow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.device.publication_custody_manage")
    );
    assert!(decision.grant_id.is_none());
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(decision.owner_source, OwnerSource::Unresolved);
    assert_eq!(decision.principal_kind, PrincipalKind::DeviceCustody);
    assert_eq!(decision.caller_ura, device);
    assert_eq!(decision.subject_ura, device);
    assert_eq!(decision.callee_ura, authority);
}

#[test]
fn device_publication_custody_can_advertise_hosted_agent_projection() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let agent = "easynet:///r/test/agent/alice.worker";
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: agent.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: verified_device_path(
            device,
            authority,
            agent,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            Some(authority),
        ),
        daemon_ura: Some(authority),
        trust_anchor: &anchor_with_device_owner(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("Device custody must carry an Agent-owned projection to its Authority");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.device.publication_custody_manage")
    );
    assert_eq!(decision.principal_kind, PrincipalKind::DeviceCustody);
    assert_eq!(decision.caller_ura, device);
    assert_eq!(decision.subject_ura, agent);
    assert_eq!(decision.callee_ura, authority);
}

#[test]
fn device_self_session_stream_can_open_authority_carrier_without_user_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN,
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: verified_device_path(
            device,
            authority,
            device,
            crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN,
            AccessAction::Stream,
            Some(authority),
        ),
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("device must open its own authority session carrier after federation join");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(decision.reason, PolicyDecisionReason::SystemRuleAllow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.device.self_session_stream")
    );
    assert!(decision.grant_id.is_none());
    assert!(decision.owner_user_ura.is_none());
    assert_eq!(decision.owner_source, OwnerSource::Unresolved);
    assert_eq!(decision.principal_kind, PrincipalKind::DeviceCustody);
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/test/ability/authority.session.open"
    );
}

#[test]
fn device_lifecycle_self_revoke_can_only_revoke_the_calling_device() {
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let ability = crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE;
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: verified_device_path(
            device,
            authority,
            device,
            ability,
            AccessAction::Manage,
            Some(authority),
        ),
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("a Device may revoke only its own lifecycle registration");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.device.lifecycle_self_revoke_manage")
    );
}

#[test]
fn device_hosted_agent_retraction_reaches_durable_hub_authorization() {
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let agent = "easynet:///r/test/agent/alice.worker";
    let ability = crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE;
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: agent.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: verified_device_path(
            device,
            authority,
            agent,
            ability,
            AccessAction::Manage,
            Some(authority),
        ),
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect("exact hosted-Agent retraction proceeds to durable host/incarnation checks");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(
        decision.policy_rule_id.as_deref(),
        Some("system.device.hosted_agent_retraction_manage")
    );
}

#[test]
fn hosted_agent_retraction_purpose_cannot_be_reused_for_device_self_revoke() {
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let agent = "easynet:///r/test/agent/alice.worker";
    let ability = crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE;
    let hosted_path = verified_device_path(
        device,
        authority,
        agent,
        ability,
        AccessAction::Manage,
        Some(authority),
    );
    let self_envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let error = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &self_envelope,
        ability,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: hosted_path,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect_err("hosted-Agent purpose is tuple-bound and cannot revoke the Device");

    assert_eq!(error.code(), tonic::Code::PermissionDenied);
}

#[test]
fn device_session_purpose_cannot_be_reused_for_publication() {
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let envelope = Envelope {
        caller: Some(identity(device)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: device.to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let session_path = verified_device_path(
        device,
        authority,
        device,
        crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN,
        AccessAction::Stream,
        Some(authority),
    );

    let error = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: session_path,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:device".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect_err("a session-purpose proof must not authorize publication custody");

    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert!(
        error.message().contains("OWNER_UNRESOLVED"),
        "unexpected denial: {error:?}"
    );
}

#[test]
fn local_authority_self_manage_rejects_cross_realm_subject() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(authority)),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/other/device/dev-1".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "federation.revoke",
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(authority),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(authority.to_string()),
    })
    .expect_err("authority self-manage must stay realm-bounded");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("\"reason\":\"OWNER_UNRESOLVED\""),
        "expected owner unresolved after authority self-manage gate rejects cross-realm subject, got: {}",
        err.message()
    );
}

#[test]
fn hub_link_principal_cannot_stream_without_grant() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity("easynet:///r/test/agent/alice.worker")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/user/alice".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "remote_desktop.attach",
        action: AccessAction::Stream,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: None,
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: None,
    })
    .expect_err("trusted hub-link principal cannot stream without grant");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("\"reason\":\"TOKEN_SCOPE_DENIED\""),
        "expected token scope denial, got: {}",
        err.message()
    );
}

#[test]
fn hub_link_can_submit_exact_invocation_lifecycle_cancel_without_product_grant() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let authority = "easynet:///r/test/authority";
    let device = "easynet:///r/test/device/dev-1";
    let envelope = Envelope {
        caller: Some(identity(authority)),
        callee: Some(identity(device)),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/resource/user.alice/invoke/terminal.attach".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: crate::daemon::ability::names::governance::INVOCATION_CANCEL,
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some(device),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some(device.to_string()),
    })
    .expect("exact generic lifecycle control reaches the target registry");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(
        decision.reason,
        PolicyDecisionReason::InvocationLifecycleControlAllow
    );
    assert_eq!(
        decision.ability_ura,
        "easynet:///r/test/ability/device.dev-1.invocation.cancel"
    );
}

#[test]
fn lifecycle_control_classifier_rejects_lookalike_ability() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/test/authority")),
        callee: Some(identity("easynet:///r/test/device/dev-1")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/test/resource/user.alice/invoke/terminal.attach".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };

    let error = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "invocation.cancel.extra",
        action: AccessAction::Manage,
        safe_read: false,
        trusted_path: TrustedCallerPath::Hub,
        daemon_ura: Some("easynet:///r/test/device/dev-1"),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: Some("ed25519:key".to_string()),
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some("easynet:///r/test/device/dev-1".to_string()),
    })
    .expect_err("a product ability lookalike must remain under normal grants");

    assert!(error.message().contains("TOKEN_SCOPE_DENIED"));
}

#[test]
fn local_hub_allows_forwarding_to_trusted_remote_owner_realm() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/local/user/alice")),
        callee: Some(identity("easynet:///r/peer/device/callee")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/peer/resource/user.bob/invoke/shell.run".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "shell.run",
        action: AccessAction::Invoke,
        safe_read: false,
        trusted_path: TrustedCallerPath::User,
        daemon_ura: Some("easynet:///r/local/authority"),
        trust_anchor: &anchor_with_peer_realm(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some("easynet:///r/local/authority".to_string()),
    })
    .expect("local hub may forward to an operator-pinned peer realm");

    assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    assert_eq!(
        decision.reason,
        PolicyDecisionReason::FederationForwardAllow
    );
    assert_eq!(
        decision.owner_user_ura.as_deref(),
        Some("easynet:///r/peer/user/bob")
    );
    assert_eq!(
        decision.rejector_ura.as_deref(),
        Some("easynet:///r/local/authority")
    );
}

#[test]
fn local_hub_does_not_forward_to_untrusted_remote_owner_realm() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let stores = AccessControlStoreRegistry::ephemeral();
    let envelope = Envelope {
        caller: Some(identity("easynet:///r/local/user/alice")),
        callee: Some(identity("easynet:///r/peer/device/callee")),
        subject: Some(SubjectIdentity {
            ura: "easynet:///r/peer/resource/user.bob/invoke/shell.run".to_string(),
            profile: String::new(),
        }),
        ..Envelope::default()
    };
    let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
        envelope: &envelope,
        ability: "shell.run",
        action: AccessAction::Invoke,
        safe_read: false,
        trusted_path: TrustedCallerPath::User,
        daemon_ura: Some("easynet:///r/local/authority"),
        trust_anchor: &empty_anchor(),
        access_control_stores: &stores,
        canonical_hash: Some("sha256:test".to_string()),
        signature_key_id: None,
        verified_authority_id: None,
        verified_session_id: None,
        accountable_principal: None,
        rejector_ura: Some("easynet:///r/local/authority".to_string()),
    })
    .expect_err("untrusted remote realm cannot use the forward allow state");

    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message()
            .contains("\"reason\":\"NON_INTERACTIVE_DENY\""),
        "expected ordinary policy denial without peer trust, got: {}",
        err.message()
    );
}

#[test]
fn policy_ability_projection_accepts_descriptor_ref_without_rewrapping() {
    let callee = "easynet:///r/test/authority";
    let ability_ura = crate::core::ura::owner_ability_ura(callee, "identity.register_pubkey")
        .expect("hub ability URA");
    let descriptor_binding =
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x44; 32],
            "manage",
        )
        .expect("test descriptor binding");
    let descriptor_ref = format!("{ability_ura}@{descriptor_binding}");

    let projected =
        ability_ura_for(callee, &descriptor_ref).expect("descriptor ref projects to ability URA");

    assert_eq!(projected, ability_ura);
    assert!(
        !projected.contains("@"),
        "policy input ability_ura must not carry descriptor version"
    );
    assert!(
        !projected.contains("hub.easynet:///"),
        "descriptor ref must not be treated as a public ability name"
    );
}
