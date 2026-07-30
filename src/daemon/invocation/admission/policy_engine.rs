// EasyNet CLI — RFC-014 pure policy engine
// =========================================

use chrono::{DateTime, Utc};

use super::decision::{
    AccessAction, OwnerResolution, PolicyDecision, PolicyDecisionOutcome, PolicyDecisionReason,
    PrincipalKind, TokenClass,
};
use super::grant_matcher::{
    GrantMatchInput, PermissionEffect, PermissionGrant, PermissionGrantMatcher,
};

#[derive(Debug, Clone)]
pub struct PolicyInput {
    pub owner: OwnerResolution,
    pub caller_user_id: Option<String>,
    pub caller_ura: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    pub token_id: Option<String>,
    pub token_class: Option<TokenClass>,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub action: AccessAction,
    pub safe_read: bool,
    pub authority_self_read: bool,
    pub authority_self_manage: bool,
    pub authority_self_stream: bool,
    pub authority_peer_directory_stream: bool,
    pub realm_authority_public_read: bool,
    pub device_self_publication_manage: bool,
    pub device_self_session_stream: bool,
    pub interactive_context_available: bool,
    pub canonical_hash: Option<String>,
    pub signature_key_id: Option<String>,
    pub verified_authority_id: Option<String>,
    pub rejector_ura: Option<String>,
    pub now: DateTime<Utc>,
    pub grants: Vec<PermissionGrant>,
}

pub struct PolicyEngine;

impl PolicyEngine {
    #[must_use]
    pub fn check(input: PolicyInput) -> PolicyDecision {
        let owner_user_id = input
            .owner
            .owner_user_id
            .clone()
            .filter(|owner| !owner.trim().is_empty());
        let matcher = PermissionGrantMatcher::new(&input.grants);

        if input.action == AccessAction::Manage
            && (input.authority_self_manage || input.device_self_publication_manage)
        {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                None,
            );
        }
        if input.action == AccessAction::Stream && input.device_self_session_stream {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                None,
            );
        }
        if input.action == AccessAction::Stream && input.authority_self_stream {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                None,
            );
        }
        if input.action == AccessAction::Stream && input.authority_peer_directory_stream {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                None,
            );
        }

        if let Some(owner_user_id) = owner_user_id.as_deref() {
            let grant_input = GrantMatchInput {
                owner_user_id,
                principal_kind: input.principal_kind,
                principal_id: &input.principal_id,
                token_id: input.token_id.as_deref(),
                callee_ura: &input.callee_ura,
                subject_ura: &input.subject_ura,
                ability_ura: &input.ability_ura,
                action: input.action,
                now: input.now,
            };
            if let Some(grant) = matcher.find(&grant_input, PermissionEffect::Deny) {
                return decision(
                    &input,
                    PolicyDecisionOutcome::Deny,
                    PolicyDecisionReason::ExplicitDeny,
                    Some(grant.grant_id.clone()),
                );
            }
        }

        // Verified authority is bounded by its verifier, but it never
        // overrides an explicit owner deny. Bootstrap authority may be the
        // only source of owner truth during first publication, so it remains
        // valid when owner resolution is not yet available.
        if input.verified_authority_id.is_some() {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                None,
            );
        }

        if input.authority_self_read && input.action == AccessAction::Read && input.safe_read {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::HubTokenReadAllow,
                None,
            );
        }
        if input.realm_authority_public_read
            && input.action == AccessAction::Read
            && input.safe_read
        {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::HubTokenReadAllow,
                None,
            );
        }

        let Some(owner_user_id) = owner_user_id else {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::OwnerUnresolved,
                None,
            );
        };

        let grant_input = GrantMatchInput {
            owner_user_id: &owner_user_id,
            principal_kind: input.principal_kind,
            principal_id: &input.principal_id,
            token_id: input.token_id.as_deref(),
            callee_ura: &input.callee_ura,
            subject_ura: &input.subject_ura,
            ability_ura: &input.ability_ura,
            action: input.action,
            now: input.now,
        };

        if input.caller_user_id.as_deref() == Some(owner_user_id.as_str()) {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::OwnerAllow,
                None,
            );
        }

        if input.token_class == Some(TokenClass::HubLink)
            && input.action == AccessAction::Read
            && input.safe_read
        {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::HubTokenReadAllow,
                None,
            );
        }

        if let Some(grant) = matcher.find(&grant_input, PermissionEffect::Allow) {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::ExplicitGrantAllow,
                Some(grant.grant_id.clone()),
            );
        }

        if let Some(grant) = matcher.find_reconfirmation_required(&grant_input) {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::GrantReconfirmationRequired,
                Some(grant.grant_id.clone()),
            );
        }

        if input.interactive_context_available {
            return decision(
                &input,
                PolicyDecisionOutcome::Prompt,
                PolicyDecisionReason::InteractiveApprovalRequired,
                None,
            );
        }

        if input.principal_kind == PrincipalKind::Token && input.action != AccessAction::Read {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::TokenScopeDenied,
                None,
            );
        }

        decision(
            &input,
            PolicyDecisionOutcome::Deny,
            PolicyDecisionReason::NonInteractiveDeny,
            None,
        )
    }
}

fn decision(
    input: &PolicyInput,
    outcome: PolicyDecisionOutcome,
    reason: PolicyDecisionReason,
    grant_id: Option<String>,
) -> PolicyDecision {
    PolicyDecision {
        decision: outcome,
        reason,
        owner_user_id: input.owner.owner_user_id.clone(),
        owner_source: input.owner.owner_source,
        caller_ura: input.caller_ura.clone(),
        principal_kind: input.principal_kind,
        principal_id: input.principal_id.clone(),
        token_id: input.token_id.clone(),
        callee_ura: input.callee_ura.clone(),
        subject_ura: input.subject_ura.clone(),
        ability_ura: input.ability_ura.clone(),
        action: input.action,
        rejector_ura: input.rejector_ura.clone(),
        policy_rule_id: grant_id.clone(),
        grant_id,
        prompt_request_id: None,
        canonical_hash: input.canonical_hash.clone(),
        signature_key_id: input.signature_key_id.clone(),
        authority_proof_id: input.verified_authority_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::admission::decision::{OwnerResolution, OwnerSource};
    use crate::daemon::invocation::admission::grant_matcher::{
        PermissionGrantLifetime, PermissionGrantState,
    };

    fn base_input() -> PolicyInput {
        PolicyInput {
            owner: OwnerResolution {
                owner_user_id: Some("alice".to_string()),
                owner_ura: Some("easynet:///r/test/user/alice".to_string()),
                owner_source: OwnerSource::Subject,
                audit_warnings: vec![],
            },
            caller_user_id: None,
            caller_ura: "easynet:///r/test/authority".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/device/dev".to_string(),
            ability_ura: "easynet:///r/test/ability/device.meta.list_resources".to_string(),
            action: AccessAction::Read,
            safe_read: true,
            authority_self_read: false,
            authority_self_manage: false,
            authority_self_stream: false,
            authority_peer_directory_stream: false,
            realm_authority_public_read: false,
            device_self_publication_manage: false,
            device_self_session_stream: false,
            interactive_context_available: false,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:key".to_string()),
            verified_authority_id: None,
            rejector_ura: Some("easynet:///r/test/device/dev".to_string()),
            now: Utc::now(),
            grants: vec![],
        }
    }

    #[test]
    fn owner_default_allows_before_missing_grant() {
        let mut input = base_input();
        input.caller_user_id = Some("alice".to_string());
        input.principal_kind = PrincipalKind::User;
        input.principal_id = "alice".to_string();
        input.token_id = None;
        input.token_class = None;
        input.action = AccessAction::Stream;
        input.safe_read = false;
        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerAllow);
    }

    #[test]
    fn hub_link_token_safe_read_allows_only_read() {
        let got = PolicyEngine::check(base_input());
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);

        let mut stream = base_input();
        stream.action = AccessAction::Stream;
        stream.safe_read = false;
        let got = PolicyEngine::check(stream);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::TokenScopeDenied);
    }

    #[test]
    fn unresolved_owner_denies() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_source = OwnerSource::Unresolved;
        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerUnresolved);
    }

    #[test]
    fn authority_self_read_allows_safe_hub_link_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.authority_self_read = true;
        input.authority_self_manage = false;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert!(got.owner_user_id.is_none());
    }

    #[test]
    fn authority_self_read_uses_gate_projection_not_token_class_duplication() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.token_class = None;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.authority_self_read = true;
        input.authority_self_manage = false;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
    }

    #[test]
    fn authority_self_read_does_not_allow_mutation_without_owner() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.action = AccessAction::Invoke;
        input.safe_read = false;
        input.authority_self_read = true;
        input.authority_self_manage = false;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerUnresolved);
    }

    #[test]
    fn realm_authority_public_read_allows_descriptor_safe_device_metadata_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/device/dev".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.realm_authority_public_read = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert!(got.owner_user_id.is_none());
    }

    #[test]
    fn realm_authority_public_read_does_not_allow_mutation_without_owner() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/device/dev".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.realm_authority_public_read = true;
        input.action = AccessAction::Invoke;
        input.safe_read = false;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerUnresolved);
    }

    #[test]
    fn authority_self_manage_allows_realm_authority_before_user_token_scope_rules() {
        let mut input = base_input();
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.principal_kind = PrincipalKind::Token;
        input.principal_id = "easynet:///r/test/authority".to_string();
        input.token_id = Some("easynet:///r/test/authority".to_string());
        input.token_class = Some(TokenClass::HubLink);
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/device/dev".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.revoke".to_string();
        input.action = AccessAction::Manage;
        input.safe_read = false;
        input.authority_self_manage = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert_eq!(got.owner_user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn authority_self_stream_allows_realm_authority_before_owner_resolution() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.principal_kind = PrincipalKind::Token;
        input.principal_id = "easynet:///r/test/authority".to_string();
        input.token_id = Some("easynet:///r/test/authority".to_string());
        input.token_class = Some(TokenClass::HubLink);
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura =
            "easynet:///r/test/resource/authority/invoke/federation.subscribe_directory_v2"
                .to_string();
        input.ability_ura =
            "easynet:///r/test/ability/authority.federation.subscribe_directory_v2".to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.authority_self_stream = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert!(got.owner_user_id.is_none());
    }

    #[test]
    fn authority_peer_directory_stream_allows_hub_link_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_id = None;
        input.owner.owner_ura = Some("easynet:///r/hub-b.local/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/hub-a.local/authority".to_string();
        input.principal_kind = PrincipalKind::Token;
        input.principal_id = "easynet:///r/hub-a.local/authority".to_string();
        input.token_id = Some("easynet:///r/hub-a.local/authority".to_string());
        input.token_class = Some(TokenClass::HubLink);
        input.callee_ura = "easynet:///r/hub-b.local/authority".to_string();
        input.subject_ura =
            "easynet:///r/hub-a.local/resource/hub.federation/directory/hub-b.local".to_string();
        input.ability_ura =
            "easynet:///r/hub-b.local/ability/authority.federation.subscribe_directory_v2"
                .to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.authority_peer_directory_stream = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert!(got.owner_user_id.is_none());
    }

    #[test]
    fn overdue_permanent_stream_grant_denies_with_reconfirmation_reason() {
        let mut input = base_input();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.ability_ura = "easynet:///r/test/ability/terminal.create".to_string();
        input.now = DateTime::parse_from_rfc3339("2026-07-09T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        input.grants = vec![PermissionGrant {
            grant_id: "grant-stream".to_string(),
            owner_user_id: "alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: Some(input.callee_ura.clone()),
            subject_ura_pattern: Some(input.subject_ura.clone()),
            ability_ura_pattern: Some(input.ability_ura.clone()),
            actions: vec![AccessAction::Stream],
            constraints: None,
            effect: PermissionEffect::Allow,
            lifetime: PermissionGrantLifetime::Permanent,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: Some("2026-07-01T00:00:00Z".to_string()),
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "easynet:///r/test/user/alice".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }];

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(
            got.reason,
            PolicyDecisionReason::GrantReconfirmationRequired
        );
        assert_eq!(got.grant_id.as_deref(), Some("grant-stream"));
    }

    #[test]
    fn verified_authority_proof_allows_without_durable_grant() {
        let mut input = base_input();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.verified_authority_id = Some("proof-1".to_string());

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert_eq!(got.authority_proof_id.as_deref(), Some("proof-1"));
        assert!(got.grant_id.is_none());
    }

    #[test]
    fn verified_authority_proof_allows_without_resolved_owner() {
        let mut input = base_input();
        input.owner = OwnerResolution {
            owner_user_id: None,
            owner_ura: None,
            owner_source: crate::daemon::invocation::admission::decision::OwnerSource::Unresolved,
            audit_warnings: vec![],
        };
        input.action = AccessAction::Invoke;
        input.safe_read = false;
        input.verified_authority_id = Some("proof-bootstrap".to_string());

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert_eq!(got.authority_proof_id.as_deref(), Some("proof-bootstrap"));
        assert!(got.grant_id.is_none());
    }

    #[test]
    fn explicit_deny_wins_over_verified_authority() {
        let mut input = base_input();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.verified_authority_id = Some("proof-1".to_string());
        input.grants = vec![PermissionGrant {
            grant_id: "deny-stream".to_string(),
            owner_user_id: "alice".to_string(),
            principal_kind: input.principal_kind,
            principal_id: input.principal_id.clone(),
            token_id: input.token_id.clone(),
            token_class: input.token_class,
            callee_ura: Some(input.callee_ura.clone()),
            subject_ura_pattern: Some(input.subject_ura.clone()),
            ability_ura_pattern: Some(input.ability_ura.clone()),
            actions: vec![AccessAction::Stream],
            constraints: None,
            effect: PermissionEffect::Deny,
            lifetime: PermissionGrantLifetime::Permanent,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: None,
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "easynet:///r/test/user/alice".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }];

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitDeny);
        assert_eq!(got.grant_id.as_deref(), Some("deny-stream"));
    }
}
