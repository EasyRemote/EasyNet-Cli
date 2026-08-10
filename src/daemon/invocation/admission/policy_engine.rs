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
pub enum SystemPolicyRuleMatch {
    AuthoritySelfRead,
    AuthoritySelfManage,
    AuthoritySelfStream,
    AuthorityPeerDirectoryStream,
    RealmAuthorityPublicRead,
    DevicePublicationCustodyManage,
    DeviceSelfSessionStream,
    RemoteOwnerForward,
}

impl SystemPolicyRuleMatch {
    fn policy_rule_id(&self) -> &'static str {
        match self {
            Self::AuthoritySelfRead => "system.authority.self_read",
            Self::AuthoritySelfManage => "system.authority.self_manage",
            Self::AuthoritySelfStream => "system.authority.self_stream",
            Self::AuthorityPeerDirectoryStream => "system.authority.peer_directory_stream",
            Self::RealmAuthorityPublicRead => "system.realm_authority.public_read",
            Self::DevicePublicationCustodyManage => "system.device.publication_custody_manage",
            Self::DeviceSelfSessionStream => "system.device.self_session_stream",
            Self::RemoteOwnerForward => "system.federation.remote_owner_forward",
        }
    }

    fn allow_reason(&self, input: &PolicyInput) -> Option<PolicyDecisionReason> {
        match self {
            Self::AuthoritySelfRead | Self::RealmAuthorityPublicRead => {
                (input.action == AccessAction::Read && input.safe_read)
                    .then_some(PolicyDecisionReason::HubTokenReadAllow)
            }
            Self::AuthoritySelfManage | Self::DevicePublicationCustodyManage => (input.action
                == AccessAction::Manage)
                .then_some(PolicyDecisionReason::SystemRuleAllow),
            Self::AuthoritySelfStream
            | Self::AuthorityPeerDirectoryStream
            | Self::DeviceSelfSessionStream => (input.action == AccessAction::Stream)
                .then_some(PolicyDecisionReason::SystemRuleAllow),
            Self::RemoteOwnerForward => Some(PolicyDecisionReason::FederationForwardAllow),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyInput {
    pub owner: OwnerResolution,
    pub caller_user_ura: Option<String>,
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
    pub system_rule_matches: Vec<SystemPolicyRuleMatch>,
    /// Exact generic runtime lifecycle control. This admits the command to
    /// the lifecycle authority; it does not authorize a target. The target
    /// registry still binds caller, execution authority, and lifecycle hash.
    pub invocation_lifecycle_control: bool,
    pub interactive_context_available: bool,
    pub canonical_hash: Option<String>,
    pub signature_key_id: Option<String>,
    pub verified_authority_id: Option<String>,
    pub verified_session_id: Option<String>,
    pub rejector_ura: Option<String>,
    pub now: DateTime<Utc>,
    pub grants: Vec<PermissionGrant>,
}

pub struct PolicyEngine;

impl PolicyEngine {
    #[must_use]
    pub fn check(input: PolicyInput) -> PolicyDecision {
        let owner_user_ura = input
            .owner
            .owner_user_ura
            .clone()
            .filter(|owner| !owner.trim().is_empty());
        let matcher = PermissionGrantMatcher::new(&input.grants);

        if let Some(owner_user_ura) = owner_user_ura.as_deref() {
            let grant_input = GrantMatchInput {
                owner_user_ura: owner_user_ura,
                principal_kind: input.principal_kind,
                principal_id: &input.principal_id,
                token_id: input.token_id.as_deref(),
                token_class: input.token_class,
                session_id: input.verified_session_id.as_deref(),
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

        // Cleanup authority is inherited from an already-admitted Invocation,
        // not from the product grant that originally opened it. Admit only the
        // exact lifecycle-control classification here; the cancellation
        // registry performs the target ownership check before signalling Axon.
        if input.invocation_lifecycle_control && input.action == AccessAction::Manage {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::InvocationLifecycleControlAllow,
                None,
            );
        }

        if let Some((rule_match, reason)) =
            input.system_rule_matches.iter().find_map(|rule_match| {
                rule_match
                    .allow_reason(&input)
                    .map(|reason| (rule_match, reason))
            })
        {
            return system_rule_decision(&input, PolicyDecisionOutcome::Allow, reason, rule_match);
        }

        // Verified authority is bounded by its verifier, but it never
        // overrides an explicit owner deny. Bootstrap authority may be the
        // only source of owner truth during first publication, so it remains
        // valid when owner resolution is not yet available.
        if input.verified_authority_id.is_some() {
            return decision(
                &input,
                PolicyDecisionOutcome::Allow,
                PolicyDecisionReason::AuthorityProofAllow,
                None,
            );
        }

        let Some(owner_user_ura) = owner_user_ura else {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::OwnerUnresolved,
                None,
            );
        };

        let grant_input = GrantMatchInput {
            owner_user_ura: &owner_user_ura,
            principal_kind: input.principal_kind,
            principal_id: &input.principal_id,
            token_id: input.token_id.as_deref(),
            token_class: input.token_class,
            session_id: input.verified_session_id.as_deref(),
            callee_ura: &input.callee_ura,
            subject_ura: &input.subject_ura,
            ability_ura: &input.ability_ura,
            action: input.action,
            now: input.now,
        };

        if input
            .owner
            .owner_ura
            .as_deref()
            .is_some_and(|owner_ura| input.caller_user_ura.as_deref() == Some(owner_ura))
        {
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
            return explicit_grant_decision(&input, grant.grant_id.clone());
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

fn explicit_grant_decision(input: &PolicyInput, grant_id: String) -> PolicyDecision {
    let mut policy_decision = decision(
        input,
        PolicyDecisionOutcome::Allow,
        PolicyDecisionReason::ExplicitGrantAllow,
        None,
    );
    policy_decision.policy_rule_id = Some(grant_id.clone());
    policy_decision.grant_id = Some(grant_id);
    policy_decision
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
        owner_user_ura: input.owner.owner_user_ura.clone(),
        owner_source: input.owner.owner_source,
        caller_ura: input.caller_ura.clone(),
        principal_kind: input.principal_kind,
        principal_id: input.principal_id.clone(),
        token_id: input.token_id.clone(),
        token_class: input.token_class,
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

fn system_rule_decision(
    input: &PolicyInput,
    outcome: PolicyDecisionOutcome,
    reason: PolicyDecisionReason,
    rule_match: &SystemPolicyRuleMatch,
) -> PolicyDecision {
    let mut policy_decision = decision(input, outcome, reason, None);
    policy_decision.policy_rule_id = Some(rule_match.policy_rule_id().to_string());
    policy_decision
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
                owner_user_ura: Some("easynet:///r/test/user/alice".to_string()),
                owner_ura: Some("easynet:///r/test/user/alice".to_string()),
                owner_source: OwnerSource::Subject,
                audit_warnings: vec![],
            },
            caller_user_ura: None,
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
            system_rule_matches: Vec::new(),
            invocation_lifecycle_control: false,
            interactive_context_available: false,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:key".to_string()),
            verified_authority_id: None,
            verified_session_id: None,
            rejector_ura: Some("easynet:///r/test/device/dev".to_string()),
            now: Utc::now(),
            grants: vec![],
        }
    }

    fn deny_for_input(input: &PolicyInput, grant_id: &str) -> PermissionGrant {
        PermissionGrant {
            grant_id: grant_id.to_string(),
            owner_user_ura: input
                .owner
                .owner_user_ura
                .clone()
                .expect("resolved owner user URA"),
            principal_kind: input.principal_kind,
            principal_id: input.principal_id.clone(),
            token_id: input.token_id.clone(),
            token_class: input.token_class,
            session_id: None,
            session_expires_at: None,
            callee_ura: Some(input.callee_ura.clone()),
            subject_ura_pattern: Some(input.subject_ura.clone()),
            ability_ura_pattern: Some(input.ability_ura.clone()),
            actions: vec![input.action],
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
        }
    }

    fn assert_explicit_deny(input: PolicyInput, grant_id: &str) {
        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::ExplicitDeny);
        assert_eq!(got.grant_id.as_deref(), Some(grant_id));
    }

    #[test]
    fn owner_default_allows_before_missing_grant() {
        let mut input = base_input();
        input.caller_user_ura = Some("easynet:///r/test/user/alice".to_string());
        input.principal_kind = PrincipalKind::User;
        input.principal_id = "easynet:///r/test/user/alice".to_string();
        input.token_id = None;
        input.token_class = None;
        input.action = AccessAction::Stream;
        input.safe_read = false;
        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerAllow);
    }

    #[test]
    fn owner_default_rejects_same_user_id_from_different_realm() {
        let mut input = base_input();
        input.owner.owner_user_ura = Some("easynet:///r/realm-b/user/alice".to_string());
        input.owner.owner_ura = Some("easynet:///r/realm-b/user/alice".to_string());
        input.caller_user_ura = Some("easynet:///r/realm-a/user/alice".to_string());
        input.caller_ura = "easynet:///r/realm-a/user/alice".to_string();
        input.principal_kind = PrincipalKind::User;
        input.principal_id = "easynet:///r/realm-a/user/alice".to_string();
        input.token_id = None;
        input.token_class = None;
        input.action = AccessAction::Stream;
        input.safe_read = false;

        let got = PolicyEngine::check(input);

        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::NonInteractiveDeny);
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
    fn invocation_lifecycle_control_allows_hub_link_manage_without_product_grant() {
        let mut input = base_input();
        input.ability_ura = "easynet:///r/test/ability/device.dev.invocation.cancel".to_string();
        input.action = AccessAction::Manage;
        input.safe_read = false;
        input.invocation_lifecycle_control = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(
            got.reason,
            PolicyDecisionReason::InvocationLifecycleControlAllow
        );
    }

    #[test]
    fn lifecycle_control_classification_does_not_admit_non_manage_action() {
        let mut input = base_input();
        input.ability_ura = "easynet:///r/test/ability/device.dev.invocation.cancel".to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.invocation_lifecycle_control = true;

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::TokenScopeDenied);
    }

    #[test]
    fn unresolved_owner_denies() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_source = OwnerSource::Unresolved;
        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerUnresolved);
    }

    #[test]
    fn authority_self_read_allows_safe_hub_link_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfRead);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert!(got.owner_user_ura.is_none());
    }

    #[test]
    fn authority_self_read_uses_gate_projection_not_token_class_duplication() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.token_class = None;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfRead);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
    }

    #[test]
    fn authority_self_read_does_not_allow_mutation_without_owner() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_ura = Some("easynet:///r/test/authority".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.discover".to_string();
        input.action = AccessAction::Invoke;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfRead);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(got.reason, PolicyDecisionReason::OwnerUnresolved);
    }

    #[test]
    fn realm_authority_public_read_allows_descriptor_safe_device_metadata_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_ura = Some("easynet:///r/test/device/dev".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::RealmAuthorityPublicRead);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert!(got.owner_user_ura.is_none());
    }

    #[test]
    fn realm_authority_public_read_does_not_allow_mutation_without_owner() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
        input.owner.owner_ura = Some("easynet:///r/test/device/dev".to_string());
        input.owner.owner_source = OwnerSource::Unresolved;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::RealmAuthorityPublicRead);
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
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfManage);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::SystemRuleAllow);
        assert_eq!(
            got.policy_rule_id.as_deref(),
            Some("system.authority.self_manage")
        );
        assert!(got.grant_id.is_none());
        assert_eq!(
            got.owner_user_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
    }

    #[test]
    fn authority_self_stream_allows_realm_authority_before_owner_resolution() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
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
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfStream);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::SystemRuleAllow);
        assert_eq!(
            got.policy_rule_id.as_deref(),
            Some("system.authority.self_stream")
        );
        assert!(got.grant_id.is_none());
        assert!(got.owner_user_ura.is_none());
    }

    #[test]
    fn authority_peer_directory_stream_allows_hub_link_without_user_owner() {
        let mut input = base_input();
        input.owner.owner_user_ura = None;
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
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthorityPeerDirectoryStream);

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::SystemRuleAllow);
        assert_eq!(
            got.policy_rule_id.as_deref(),
            Some("system.authority.peer_directory_stream")
        );
        assert!(got.grant_id.is_none());
        assert!(got.owner_user_ura.is_none());
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
            owner_user_ura: "easynet:///r/test/user/alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
            session_expires_at: None,
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
    fn session_grant_requires_matching_verified_session_id() {
        let mut input = base_input();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.ability_ura = "easynet:///r/test/ability/terminal.create".to_string();
        input.now = DateTime::parse_from_rfc3339("2026-08-07T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        input.grants = vec![PermissionGrant {
            grant_id: "grant-session".to_string(),
            owner_user_ura: "easynet:///r/test/user/alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            session_id: Some("session-1".to_string()),
            session_expires_at: Some("2026-08-07T01:00:00Z".to_string()),
            callee_ura: Some(input.callee_ura.clone()),
            subject_ura_pattern: Some(input.subject_ura.clone()),
            ability_ura_pattern: Some(input.ability_ura.clone()),
            actions: vec![AccessAction::Stream],
            constraints: None,
            effect: PermissionEffect::Allow,
            lifetime: PermissionGrantLifetime::Session,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: None,
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "easynet:///r/test/user/alice".to_string(),
            created_at: "2026-08-07T00:00:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }];

        let unbound = PolicyEngine::check(input.clone());
        assert_eq!(unbound.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(unbound.reason, PolicyDecisionReason::TokenScopeDenied);

        input.verified_session_id = Some("session-2".to_string());
        let wrong_session = PolicyEngine::check(input.clone());
        assert_eq!(wrong_session.decision, PolicyDecisionOutcome::Deny);
        assert_eq!(wrong_session.reason, PolicyDecisionReason::TokenScopeDenied);

        input.verified_session_id = Some("session-1".to_string());
        let bound = PolicyEngine::check(input);
        assert_eq!(bound.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(bound.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert_eq!(bound.grant_id.as_deref(), Some("grant-session"));
    }

    #[test]
    fn explicit_deny_wins_over_authority_self_manage_allow() {
        let mut input = base_input();
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.principal_id = input.caller_ura.clone();
        input.token_id = Some(input.caller_ura.clone());
        input.callee_ura = "easynet:///r/test/authority".to_string();
        input.subject_ura = "easynet:///r/test/device/dev".to_string();
        input.ability_ura = "easynet:///r/test/ability/authority.federation.revoke".to_string();
        input.action = AccessAction::Manage;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfManage);
        input.grants = vec![deny_for_input(&input, "deny-authority-manage")];

        assert_explicit_deny(input, "deny-authority-manage");
    }

    #[test]
    fn explicit_deny_wins_over_device_publication_manage_allow() {
        let mut input = base_input();
        input.principal_kind = PrincipalKind::DeviceCustody;
        input.principal_id = "easynet:///r/test/device/dev".to_string();
        input.token_id = Some(input.principal_id.clone());
        input.token_class = Some(TokenClass::DevicePairing);
        input.caller_ura = input.principal_id.clone();
        input.callee_ura = input.principal_id.clone();
        input.subject_ura = input.principal_id.clone();
        input.ability_ura = "easynet:///r/test/ability/device.dev.ability.publish".to_string();
        input.action = AccessAction::Manage;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::DevicePublicationCustodyManage);
        input.grants = vec![deny_for_input(&input, "deny-device-publish")];

        assert_explicit_deny(input, "deny-device-publish");
    }

    #[test]
    fn explicit_deny_wins_over_device_session_stream_allow() {
        let mut input = base_input();
        input.principal_kind = PrincipalKind::DeviceCustody;
        input.principal_id = "easynet:///r/test/device/dev".to_string();
        input.token_id = Some(input.principal_id.clone());
        input.token_class = Some(TokenClass::DevicePairing);
        input.caller_ura = input.principal_id.clone();
        input.callee_ura = input.principal_id.clone();
        input.subject_ura = "easynet:///r/test/resource/session/terminal/session-1".to_string();
        input.ability_ura = "easynet:///r/test/ability/device.dev.terminal.stream".to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::DeviceSelfSessionStream);
        input.grants = vec![deny_for_input(&input, "deny-device-session")];

        assert_explicit_deny(input, "deny-device-session");
    }

    #[test]
    fn explicit_deny_wins_over_authority_self_stream_allow() {
        let mut input = base_input();
        input.caller_ura = "easynet:///r/test/authority".to_string();
        input.principal_id = input.caller_ura.clone();
        input.token_id = Some(input.caller_ura.clone());
        input.callee_ura = input.caller_ura.clone();
        input.subject_ura =
            "easynet:///r/test/resource/authority/invoke/federation.subscribe_directory_v2"
                .to_string();
        input.ability_ura =
            "easynet:///r/test/ability/authority.federation.subscribe_directory_v2".to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthoritySelfStream);
        input.grants = vec![deny_for_input(&input, "deny-authority-stream")];

        assert_explicit_deny(input, "deny-authority-stream");
    }

    #[test]
    fn explicit_deny_wins_over_authority_peer_directory_stream_allow() {
        let mut input = base_input();
        input.caller_ura = "easynet:///r/hub-a.local/authority".to_string();
        input.principal_id = input.caller_ura.clone();
        input.token_id = Some(input.caller_ura.clone());
        input.callee_ura = "easynet:///r/hub-b.local/authority".to_string();
        input.subject_ura =
            "easynet:///r/hub-a.local/resource/hub.federation/directory/hub-b.local".to_string();
        input.ability_ura =
            "easynet:///r/hub-b.local/ability/authority.federation.subscribe_directory_v2"
                .to_string();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::AuthorityPeerDirectoryStream);
        input.grants = vec![deny_for_input(&input, "deny-peer-directory-stream")];

        assert_explicit_deny(input, "deny-peer-directory-stream");
    }

    #[test]
    fn explicit_deny_wins_over_remote_owner_forward_allow() {
        let mut input = base_input();
        input.action = AccessAction::Invoke;
        input.safe_read = false;
        input
            .system_rule_matches
            .push(SystemPolicyRuleMatch::RemoteOwnerForward);
        input.grants = vec![deny_for_input(&input, "deny-federation-forward")];

        assert_explicit_deny(input, "deny-federation-forward");
    }

    #[test]
    fn verified_authority_proof_allows_without_durable_grant() {
        let mut input = base_input();
        input.action = AccessAction::Stream;
        input.safe_read = false;
        input.verified_authority_id = Some("proof-1".to_string());

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::AuthorityProofAllow);
        assert_eq!(got.authority_proof_id.as_deref(), Some("proof-1"));
        assert!(got.grant_id.is_none());
    }

    #[test]
    fn verified_authority_proof_allows_without_resolved_owner() {
        let mut input = base_input();
        input.owner = OwnerResolution {
            owner_user_ura: None,
            owner_ura: None,
            owner_source: crate::daemon::invocation::admission::decision::OwnerSource::Unresolved,
            audit_warnings: vec![],
        };
        input.action = AccessAction::Invoke;
        input.safe_read = false;
        input.verified_authority_id = Some("proof-bootstrap".to_string());

        let got = PolicyEngine::check(input);
        assert_eq!(got.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(got.reason, PolicyDecisionReason::AuthorityProofAllow);
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
            owner_user_ura: "easynet:///r/test/user/alice".to_string(),
            principal_kind: input.principal_kind,
            principal_id: input.principal_id.clone(),
            token_id: input.token_id.clone(),
            token_class: input.token_class,
            session_id: None,
            session_expires_at: None,
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
