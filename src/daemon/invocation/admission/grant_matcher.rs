// EasyNet CLI — RFC-014 deterministic grant matcher
// ==================================================

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::decision::{AccessAction, PrincipalKind, TokenClass};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PermissionConstraints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args_schema_filter: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_scope: Option<NetworkScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_invocations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_user_present: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkScope {
    Local,
    Paired,
    Federated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionEffect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantLifetime {
    Once,
    Session,
    Ttl,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantState {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PermissionGrant {
    pub grant_id: String,
    pub owner_user_id: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_class: Option<TokenClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callee_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_ura_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ability_ura_pattern: Option<String>,
    pub actions: Vec<AccessAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<PermissionConstraints>,
    pub effect: PermissionEffect,
    pub lifetime: PermissionGrantLifetime,
    pub state: PermissionGrantState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_required_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reviewed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionGrant {
    #[must_use]
    pub fn active_at(&self, now: DateTime<Utc>) -> bool {
        if self.state != PermissionGrantState::Active {
            return false;
        }
        match self.expires_at.as_deref() {
            Some(raw) => DateTime::parse_from_rfc3339(raw)
                .map(|expiry| expiry.with_timezone(&Utc) > now)
                .unwrap_or(false),
            None => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantMatchInput<'a> {
    pub owner_user_id: &'a str,
    pub principal_kind: PrincipalKind,
    pub principal_id: &'a str,
    pub token_id: Option<&'a str>,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub ability_ura: &'a str,
    pub action: AccessAction,
    pub now: DateTime<Utc>,
}

pub struct PermissionGrantMatcher<'a> {
    grants: &'a [PermissionGrant],
}

impl<'a> PermissionGrantMatcher<'a> {
    #[must_use]
    pub fn new(grants: &'a [PermissionGrant]) -> Self {
        Self { grants }
    }

    #[must_use]
    pub fn find(
        &self,
        input: &GrantMatchInput<'_>,
        effect: PermissionEffect,
    ) -> Option<&'a PermissionGrant> {
        let mut matches: Vec<(GrantSpecificity, &PermissionGrant)> = self
            .grants
            .iter()
            .filter(|grant| grant.effect == effect)
            .filter(|grant| grant.active_at(input.now))
            .filter(|grant| grant.owner_user_id == input.owner_user_id)
            .filter(|grant| grant.principal_kind == input.principal_kind)
            .filter(|grant| grant.principal_id == input.principal_id)
            .filter(|grant| token_matches(grant.token_id.as_deref(), input.token_id))
            .filter(|grant| grant.actions.contains(&input.action))
            .filter(|grant| {
                grant
                    .callee_ura
                    .as_deref()
                    .map(|callee| callee == input.callee_ura)
                    .unwrap_or(true)
            })
            .filter_map(|grant| specificity(grant, input).map(|s| (s, grant)))
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.grant_id.cmp(&right.grant_id))
        });
        matches.into_iter().map(|(_, grant)| grant).next()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GrantSpecificity {
    ClassLevel = 1,
    SubjectPrefixAbilityPrefix = 2,
    SubjectPrefixExactAbility = 3,
    ExactSubjectAbilityPrefix = 4,
    ExactSubjectExactAbility = 5,
}

fn token_matches(grant_token: Option<&str>, input_token: Option<&str>) -> bool {
    match grant_token {
        Some(expected) => input_token == Some(expected),
        None => true,
    }
}

fn specificity(grant: &PermissionGrant, input: &GrantMatchInput<'_>) -> Option<GrantSpecificity> {
    let subject = pattern_match(grant.subject_ura_pattern.as_deref(), input.subject_ura)?;
    let ability = pattern_match(grant.ability_ura_pattern.as_deref(), input.ability_ura)?;
    Some(match (subject, ability) {
        (PatternMatch::Exact, PatternMatch::Exact) => GrantSpecificity::ExactSubjectExactAbility,
        (PatternMatch::Exact, PatternMatch::Prefix) => GrantSpecificity::ExactSubjectAbilityPrefix,
        (PatternMatch::Prefix, PatternMatch::Exact) => GrantSpecificity::SubjectPrefixExactAbility,
        (PatternMatch::Prefix, PatternMatch::Prefix) => {
            GrantSpecificity::SubjectPrefixAbilityPrefix
        }
        (PatternMatch::Class, _) | (_, PatternMatch::Class) => GrantSpecificity::ClassLevel,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternMatch {
    Exact,
    Prefix,
    Class,
}

fn pattern_match(pattern: Option<&str>, value: &str) -> Option<PatternMatch> {
    let Some(pattern) = pattern.map(str::trim).filter(|p| !p.is_empty()) else {
        return Some(PatternMatch::Class);
    };
    if let Some(prefix) = pattern.strip_suffix('*') {
        return (!prefix.is_empty() && value.starts_with(prefix)).then_some(PatternMatch::Prefix);
    }
    (pattern == value).then_some(PatternMatch::Exact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(id: &str, effect: PermissionEffect, subject: &str, ability: &str) -> PermissionGrant {
        PermissionGrant {
            grant_id: id.to_string(),
            owner_user_id: "alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: None,
            subject_ura_pattern: Some(subject.to_string()),
            ability_ura_pattern: Some(ability.to_string()),
            actions: vec![AccessAction::Read],
            constraints: None,
            effect,
            lifetime: PermissionGrantLifetime::Permanent,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: None,
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "owner".to_string(),
            created_at: "2026-07-09T00:00:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }
    }

    #[test]
    fn deterministic_specificity_beats_storage_order() {
        let grants = vec![
            grant(
                "b-prefix",
                PermissionEffect::Allow,
                "easynet:///r/a/resource/*",
                "easynet:///r/a/ability/device.*",
            ),
            grant(
                "a-exact",
                PermissionEffect::Allow,
                "easynet:///r/a/resource/x",
                "easynet:///r/a/ability/device.meta",
            ),
        ];
        let input = GrantMatchInput {
            owner_user_id: "alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            callee_ura: "easynet:///r/a/device/dev",
            subject_ura: "easynet:///r/a/resource/x",
            ability_ura: "easynet:///r/a/ability/device.meta",
            action: AccessAction::Read,
            now: Utc::now(),
        };
        let got = PermissionGrantMatcher::new(&grants)
            .find(&input, PermissionEffect::Allow)
            .expect("allow match");
        assert_eq!(got.grant_id, "a-exact");
    }
}
