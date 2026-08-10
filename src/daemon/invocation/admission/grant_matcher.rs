// EasyNet CLI — RFC-014 deterministic grant matcher
// ==================================================

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::decision::{AccessAction, PrincipalKind, TokenClass};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct PermissionGrant {
    pub grant_id: String,
    /// Runtime field is a canonical User URA. The serialized `owner_user_id`
    /// key is retained only for permission-grant durable/wire compatibility.
    #[serde(rename = "owner_user_id")]
    pub owner_user_ura: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_class: Option<TokenClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_expires_at: Option<String>,
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
        if self.lifetime == PermissionGrantLifetime::Once && self.last_used_at.is_some() {
            return false;
        }
        if self.lifetime == PermissionGrantLifetime::Session && !self.session_active_at(now) {
            return false;
        }
        match self.expires_at.as_deref() {
            Some(raw) => DateTime::parse_from_rfc3339(raw)
                .map(|expiry| expiry.with_timezone(&Utc) > now)
                .unwrap_or(false),
            None => true,
        }
    }

    #[must_use]
    pub fn session_matches(&self, session_id: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.lifetime != PermissionGrantLifetime::Session {
            return true;
        }
        self.active_at(now) && self.session_id.as_deref() == session_id
    }

    #[must_use]
    pub fn admissible_for(&self, action: AccessAction, now: DateTime<Utc>) -> bool {
        self.active_at(now) && !self.reconfirmation_required_for(action, now)
    }

    #[must_use]
    pub fn reconfirmation_required_for(&self, action: AccessAction, now: DateTime<Utc>) -> bool {
        if !self.active_at(now)
            || self.effect != PermissionEffect::Allow
            || !self.actions.contains(&action)
            || !self.requires_periodic_reconfirmation_for(action)
        {
            return false;
        }
        self.reconfirmation_deadline_for(action)
            .map(|deadline| deadline <= now)
            .unwrap_or(true)
    }

    #[must_use]
    pub fn default_reconfirmation_deadline(&self) -> Option<DateTime<Utc>> {
        if self.effect != PermissionEffect::Allow
            || self.lifetime != PermissionGrantLifetime::Permanent
        {
            return None;
        }
        let interval = review_interval_for_actions(&self.actions)?;
        let anchor = self
            .last_reviewed_at
            .as_deref()
            .filter(|raw| !raw.trim().is_empty())
            .unwrap_or(self.created_at.as_str());
        DateTime::parse_from_rfc3339(anchor)
            .ok()
            .map(|timestamp| timestamp.with_timezone(&Utc) + interval)
    }

    fn requires_periodic_reconfirmation_for(&self, action: AccessAction) -> bool {
        self.lifetime == PermissionGrantLifetime::Permanent
            && matches!(
                action,
                AccessAction::Stream | AccessAction::Manage | AccessAction::Grant
            )
    }

    fn reconfirmation_deadline_for(&self, action: AccessAction) -> Option<DateTime<Utc>> {
        if !self.requires_periodic_reconfirmation_for(action) {
            return None;
        }
        match self.review_required_after.as_deref() {
            Some(raw) if !raw.trim().is_empty() => DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|timestamp| timestamp.with_timezone(&Utc)),
            _ => self.default_reconfirmation_deadline(),
        }
    }

    fn session_active_at(&self, now: DateTime<Utc>) -> bool {
        self.session_expires_at
            .as_deref()
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|expiry| expiry.with_timezone(&Utc) > now)
            .unwrap_or(false)
    }
}

fn review_interval_for_actions(actions: &[AccessAction]) -> Option<Duration> {
    if actions
        .iter()
        .any(|action| matches!(action, AccessAction::Manage | AccessAction::Grant))
    {
        return Some(Duration::days(30));
    }
    actions
        .contains(&AccessAction::Stream)
        .then_some(Duration::days(90))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantMatchInput<'a> {
    pub owner_user_ura: &'a str,
    pub principal_kind: PrincipalKind,
    pub principal_id: &'a str,
    pub token_id: Option<&'a str>,
    pub token_class: Option<TokenClass>,
    pub session_id: Option<&'a str>,
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
        self.find_active(input, effect)
    }

    #[must_use]
    pub fn find_active(
        &self,
        input: &GrantMatchInput<'_>,
        effect: PermissionEffect,
    ) -> Option<&'a PermissionGrant> {
        self.matching_grants(input, effect)
            .into_iter()
            .find(|grant| grant.admissible_for(input.action, input.now))
    }

    #[must_use]
    pub fn find_reconfirmation_required(
        &self,
        input: &GrantMatchInput<'_>,
    ) -> Option<&'a PermissionGrant> {
        self.matching_grants(input, PermissionEffect::Allow)
            .into_iter()
            .find(|grant| grant.reconfirmation_required_for(input.action, input.now))
    }

    fn matching_grants(
        &self,
        input: &GrantMatchInput<'_>,
        effect: PermissionEffect,
    ) -> Vec<&'a PermissionGrant> {
        let mut matches: Vec<(GrantSpecificity, &PermissionGrant)> = self
            .grants
            .iter()
            .filter(|grant| grant.effect == effect)
            .filter(|grant| grant.active_at(input.now))
            .filter(|grant| grant.owner_user_ura == input.owner_user_ura)
            .filter(|grant| grant.principal_kind == input.principal_kind)
            .filter(|grant| grant.principal_id == input.principal_id)
            .filter(|grant| token_matches(grant.token_id.as_deref(), input.token_id))
            .filter(|grant| token_class_matches(grant.token_class, input.token_class))
            .filter(|grant| grant.session_matches(input.session_id, input.now))
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
        matches.into_iter().map(|(_, grant)| grant).collect()
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

fn token_class_matches(grant_class: Option<TokenClass>, input_class: Option<TokenClass>) -> bool {
    match grant_class {
        Some(expected) => input_class == Some(expected),
        None => true,
    }
}

fn specificity(grant: &PermissionGrant, input: &GrantMatchInput<'_>) -> Option<GrantSpecificity> {
    let subject =
        GrantSelector::parse(grant.subject_ura_pattern.as_deref())?.matches(input.subject_ura)?;
    let ability =
        GrantSelector::parse(grant.ability_ura_pattern.as_deref())?.matches(input.ability_ura)?;
    Some(match (subject, ability) {
        (GrantSelectorMatch::Exact, GrantSelectorMatch::Exact) => {
            GrantSpecificity::ExactSubjectExactAbility
        }
        (GrantSelectorMatch::Exact, GrantSelectorMatch::SegmentPrefix) => {
            GrantSpecificity::ExactSubjectAbilityPrefix
        }
        (GrantSelectorMatch::SegmentPrefix, GrantSelectorMatch::Exact) => {
            GrantSpecificity::SubjectPrefixExactAbility
        }
        (GrantSelectorMatch::SegmentPrefix, GrantSelectorMatch::SegmentPrefix) => {
            GrantSpecificity::SubjectPrefixAbilityPrefix
        }
        (GrantSelectorMatch::ClassScope, _) | (_, GrantSelectorMatch::ClassScope) => {
            GrantSpecificity::ClassLevel
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GrantSelector<'a> {
    Exact(&'a str),
    SegmentPrefix { prefix: &'a str },
    ClassScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrantSelectorMatch {
    Exact,
    SegmentPrefix,
    ClassScope,
}

impl<'a> GrantSelector<'a> {
    fn parse(raw: Option<&'a str>) -> Option<Self> {
        let Some(raw) = raw else {
            return Some(Self::ClassScope);
        };
        if raw.is_empty() {
            return Some(Self::ClassScope);
        }
        if raw.trim() != raw {
            return None;
        }
        for suffix in ["/*", ".*"] {
            if let Some(prefix) = raw.strip_suffix(suffix) {
                return (!prefix.is_empty()).then_some(Self::SegmentPrefix { prefix });
            }
        }
        if raw.ends_with('*') {
            return None;
        }
        Some(Self::Exact(raw))
    }

    fn matches(&self, value: &str) -> Option<GrantSelectorMatch> {
        match self {
            Self::Exact(expected) => (*expected == value).then_some(GrantSelectorMatch::Exact),
            Self::SegmentPrefix { prefix } => value
                .get(prefix.len()..)
                .and_then(|suffix| {
                    suffix
                        .strip_prefix('/')
                        .or_else(|| suffix.strip_prefix('.'))
                })
                .filter(|suffix| !suffix.is_empty())
                .map(|_| GrantSelectorMatch::SegmentPrefix),
            Self::ClassScope => Some(GrantSelectorMatch::ClassScope),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn grant(id: &str, effect: PermissionEffect, subject: &str, ability: &str) -> PermissionGrant {
        PermissionGrant {
            grant_id: id.to_string(),
            owner_user_ura: "easynet:///r/test/user/alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
            session_expires_at: None,
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

    fn grant_json() -> serde_json::Value {
        json!({
            "grant_id": "grant-1",
            "owner_user_id": "easynet:///r/test/user/alice",
            "principal_kind": "token",
            "principal_id": "token-principal",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": "easynet:///r/test/agent/device.dev.terminal",
            "subject_ura_pattern": "easynet:///r/test/resource/user.alice/session/s1",
            "ability_ura_pattern": "terminal.attach",
            "actions": ["read"],
            "constraints": {
                "resource_types": [],
                "network_scope": "local"
            },
            "effect": "allow",
            "lifetime": "permanent",
            "state": "active",
            "created_by": "easynet:///r/test/user/alice",
            "created_at": "2026-07-09T00:00:00Z"
        })
    }

    #[test]
    fn permission_grant_deserialization_rejects_unknown_fields() {
        let mut raw = grant_json();
        raw["legacy_scope"] = json!("compat-carrier");
        let error = serde_json::from_value::<PermissionGrant>(raw)
            .expect_err("PermissionGrant must reject unknown fields");
        assert!(
            error.to_string().contains("unknown field `legacy_scope`"),
            "error should name the noncanonical grant field: {error}"
        );
    }

    #[test]
    fn permission_grant_deserialization_requires_identity_fields() {
        let mut raw = grant_json();
        raw.as_object_mut().expect("object").remove("owner_user_id");
        let error = serde_json::from_value::<PermissionGrant>(raw)
            .expect_err("PermissionGrant must require owner_user_id");
        assert!(
            error.to_string().contains("missing field `owner_user_id`"),
            "error should name missing owner identity: {error}"
        );

        let mut raw = grant_json();
        raw.as_object_mut().expect("object").remove("principal_id");
        let error = serde_json::from_value::<PermissionGrant>(raw)
            .expect_err("PermissionGrant must require principal_id");
        assert!(
            error.to_string().contains("missing field `principal_id`"),
            "error should name missing principal identity: {error}"
        );
    }

    #[test]
    fn permission_constraints_deserialization_rejects_unknown_fields() {
        let raw = json!({
            "resource_types": [],
            "network_scope": "local",
            "legacy_filter": {"allow": true}
        });
        let error = serde_json::from_value::<PermissionConstraints>(raw)
            .expect_err("PermissionConstraints must reject unknown fields");
        assert!(
            error.to_string().contains("unknown field `legacy_filter`"),
            "error should name noncanonical constraint field: {error}"
        );
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
            owner_user_ura: "easynet:///r/test/user/alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
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

    #[test]
    fn bare_owner_user_id_does_not_match_realm_scoped_owner_key() {
        let mut grant = grant(
            "legacy-bare-owner",
            PermissionEffect::Allow,
            "easynet:///r/a/resource/x",
            "easynet:///r/a/ability/device.meta",
        );
        grant.owner_user_ura = "alice".to_string();
        let input = GrantMatchInput {
            owner_user_ura: "easynet:///r/a/user/alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
            callee_ura: "easynet:///r/a/device/dev",
            subject_ura: "easynet:///r/a/resource/x",
            ability_ura: "easynet:///r/a/ability/device.meta",
            action: AccessAction::Read,
            now: Utc::now(),
        };

        assert!(PermissionGrantMatcher::new(&[grant])
            .find(&input, PermissionEffect::Allow)
            .is_none());
    }

    #[test]
    fn overdue_permanent_stream_grant_requires_reconfirmation() {
        let mut grants = vec![grant(
            "stream-grant",
            PermissionEffect::Allow,
            "easynet:///r/a/resource/x",
            "easynet:///r/a/ability/device.stream",
        )];
        grants[0].actions = vec![AccessAction::Stream];
        grants[0].review_required_after = Some("2026-07-01T00:00:00Z".to_string());
        let input = GrantMatchInput {
            owner_user_ura: "easynet:///r/test/user/alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
            callee_ura: "easynet:///r/a/device/dev",
            subject_ura: "easynet:///r/a/resource/x",
            ability_ura: "easynet:///r/a/ability/device.stream",
            action: AccessAction::Stream,
            now: DateTime::parse_from_rfc3339("2026-07-09T00:00:00Z")
                .expect("timestamp")
                .with_timezone(&Utc),
        };
        let matcher = PermissionGrantMatcher::new(&grants);

        assert!(matcher
            .find_active(&input, PermissionEffect::Allow)
            .is_none());
        assert_eq!(
            matcher
                .find_reconfirmation_required(&input)
                .expect("reconfirmation match")
                .grant_id,
            "stream-grant"
        );
    }

    #[test]
    fn active_broader_grant_can_admit_when_specific_grant_is_overdue() {
        let mut specific = grant(
            "specific-overdue",
            PermissionEffect::Allow,
            "easynet:///r/a/resource/x",
            "easynet:///r/a/ability/device.stream",
        );
        specific.actions = vec![AccessAction::Stream];
        specific.review_required_after = Some("2026-07-01T00:00:00Z".to_string());
        let mut broader = grant(
            "broader-active",
            PermissionEffect::Allow,
            "easynet:///r/a/resource/*",
            "easynet:///r/a/ability/device.*",
        );
        broader.actions = vec![AccessAction::Stream];
        broader.review_required_after = Some("2026-08-01T00:00:00Z".to_string());
        let grants = vec![specific, broader];
        let input = GrantMatchInput {
            owner_user_ura: "easynet:///r/test/user/alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::HubLink),
            session_id: None,
            callee_ura: "easynet:///r/a/device/dev",
            subject_ura: "easynet:///r/a/resource/x",
            ability_ura: "easynet:///r/a/ability/device.stream",
            action: AccessAction::Stream,
            now: DateTime::parse_from_rfc3339("2026-07-09T00:00:00Z")
                .expect("timestamp")
                .with_timezone(&Utc),
        };

        let got = PermissionGrantMatcher::new(&grants)
            .find_active(&input, PermissionEffect::Allow)
            .expect("active broader grant");
        assert_eq!(got.grant_id, "broader-active");
    }

    #[test]
    fn token_class_constrained_grant_requires_matching_token_class() {
        let grants = vec![grant(
            "hub-link-only",
            PermissionEffect::Allow,
            "easynet:///r/a/resource/x",
            "easynet:///r/a/ability/device.meta",
        )];
        let input = GrantMatchInput {
            owner_user_ura: "easynet:///r/test/user/alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::BrowserSession),
            session_id: None,
            callee_ura: "easynet:///r/a/device/dev",
            subject_ura: "easynet:///r/a/resource/x",
            ability_ura: "easynet:///r/a/ability/device.meta",
            action: AccessAction::Read,
            now: Utc::now(),
        };
        let matcher = PermissionGrantMatcher::new(&grants);

        assert!(matcher.find(&input, PermissionEffect::Allow).is_none());

        let mut matching = input.clone();
        matching.token_class = Some(TokenClass::HubLink);
        assert_eq!(
            matcher
                .find(&matching, PermissionEffect::Allow)
                .expect("matching token class")
                .grant_id,
            "hub-link-only"
        );
    }
}
