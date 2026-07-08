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
    pub interactive_context_available: bool,
    pub canonical_hash: Option<String>,
    pub signature_key_id: Option<String>,
    pub authority_proof_id: Option<String>,
    pub rejector_ura: Option<String>,
    pub now: DateTime<Utc>,
    pub grants: Vec<PermissionGrant>,
}

pub struct PolicyEngine;

impl PolicyEngine {
    #[must_use]
    pub fn check(input: PolicyInput) -> PolicyDecision {
        let Some(owner_user_id) = input
            .owner
            .owner_user_id
            .clone()
            .filter(|owner| !owner.trim().is_empty())
        else {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::OwnerUnresolved,
                None,
            );
        };

        let matcher = PermissionGrantMatcher::new(&input.grants);
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

        if let Some(grant) = matcher.find(&grant_input, PermissionEffect::Deny) {
            return decision(
                &input,
                PolicyDecisionOutcome::Deny,
                PolicyDecisionReason::ExplicitDeny,
                Some(grant.grant_id.clone()),
            );
        }

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
        authority_proof_id: input.authority_proof_id.clone(),
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
            caller_ura: "easynet:///r/test/hub".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/device/dev".to_string(),
            ability_ura: "easynet:///r/test/ability/device.meta.list_resources".to_string(),
            action: AccessAction::Read,
            safe_read: true,
            interactive_context_available: false,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:key".to_string()),
            authority_proof_id: None,
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
}
