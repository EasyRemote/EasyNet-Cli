// EasyNet CLI — AbilityDescriptor (RFC-001 §1.6 / §A15)
// =======================================================
//
// File: src/runtime/ability_descriptor.rs
//
// Per AXON-RFC-001 plan v4.1.2 §1.6, every advertised Ability is
// described by an AbilityDescriptor. This is the Rust home for
// that schema. Today's CLI carries two ad-hoc precursors:
//
//   * `runtime::abilities::AgentAbilitySpec` — per-agent on-disk
//     manifest shape, used for chat / skill manifests.
//   * `runtime::agents::SystemAbilityMetadata` — in-memory shape
//     for built-in abilities (observe.health, fleet.*, …).
//
// Both pre-date RFC-001 and lack visibility/scope. AbilityDescriptor
// supersedes them at the protocol-facing edge: anything that goes to
// `federation.advertise_abilities` (P4.6) or back to a caller via
// `meta.list_abilities` MUST flow through this struct.
//
// Why a fresh module instead of mutating the existing types
// ---------------------------------------------------------
// The existing types still have non-trivial in-tree consumers
// (manifest readers, schema synthesizers, the legacy MCP catalog).
// Bolting visibility + scope onto them would force a single
// commit to touch every consumer. P4.1 introduces the schema in
// isolation; P4.2 wires profile registration to it; P4.4 reshapes
// AgentType into descriptor metadata; and only after all of those
// land does the legacy SystemAbilityMetadata get retired.
//
// The minimum viable scope for P4.1
// ---------------------------------
// Per the plan §1.6 schema:
//
//   AbilityDescriptor {
//     name, owner_agent_uri, visibility, scope_subjects[],
//     scope_agents[], source, schema_summary{input,
//     output_receipt_body}, hints{read_only, destructive,
//     idempotent, streaming_only}
//   }
//
// We model all fields. The visibility filter logic (PUBLIC always /
// SCOPED checked / PRIVATE owner-only) lives here too because it is
// a protocol invariant: a centrally-defined `is_visible_to(...)`
// closes the door on every future caller writing its own slightly-
// wrong filter.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Per RFC §1.6, an Ability has one of three visibility levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Visibility {
    /// Returned by every `federation.resolve` / `meta.list_abilities`
    /// regardless of caller. Default for `observe.*`, `meta.describe`.
    Public,
    /// Returned only when both axes match: subject ∈ scope_subjects
    /// (or scope_subjects empty/Any) AND caller ∈ scope_agents (or
    /// scope_agents empty/Any). Default for `fleet.*`, `consent.*`,
    /// most `meta.*`. Per [P8] also default for `conversation.*`.
    Scoped,
    /// Returned only when caller is the owner Agent's signing
    /// authority (its hosting device-profile) OR subject is the
    /// host operator. PRIVATE is a degenerate SCOPED case.
    Private,
}

impl Default for Visibility {
    fn default() -> Self {
        // Defaulting to PRIVATE is the safe choice for any descriptor
        // built by a forgetful caller — over-restrictive, never
        // over-permissive. The wire decoder can still parse explicit
        // PUBLIC/SCOPED entries; this only affects code-side
        // construction with fields omitted.
        Visibility::Private
    }
}

/// Per RFC §1.6, each scope axis (subject vs caller) is a rule.
/// Modeled as an enum so an empty `Vec` cannot accidentally mean
/// "no restriction" — it means "explicit deny-all" (None).
///
/// Uses serde `tag = "kind", content = "uris"` so the wire shape is
/// unambiguous: `{"kind":"any"}`, `{"kind":"none"}`, or
/// `{"kind":"only_matching","uris":["…"]}`. Adjacent-tagged because
/// internally-tagged would conflict with the named-content shape
/// for the OnlyMatching variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "uris", rename_all = "snake_case")]
pub enum ScopeRule {
    /// No restriction on this axis. Used together with the visibility
    /// gate; SCOPED + Any on both axes degrades to "any caller, any
    /// subject" which is functionally PUBLIC — callers should pick
    /// PUBLIC instead in that case.
    Any,
    /// Allow listed canonical URAs. Prefix matching uses a strict
    /// path-boundary rule (the matched prefix must be followed by
    /// `/` or end-of-string) so `easynet:///r/acme/agent/01` does
    /// NOT match `easynet:///r/acme/agent/01ATTACKER`.
    OnlyMatching(Vec<String>),
    /// Explicit deny-all. Use when a SCOPED ability is intentionally
    /// off-limits on this axis pending an operator gesture.
    None,
}

impl Default for ScopeRule {
    fn default() -> Self {
        // Same rationale as Visibility: deny-by-default beats
        // permit-by-default for forgotten fields.
        ScopeRule::None
    }
}

impl ScopeRule {
    /// `true` iff this rule admits the given URA.
    pub fn admits(&self, candidate_uri: &str) -> bool {
        match self {
            ScopeRule::Any => true,
            ScopeRule::None => false,
            ScopeRule::OnlyMatching(allowed) => allowed
                .iter()
                .any(|allow| uri_matches_with_path_boundary(allow, candidate_uri)),
        }
    }
}

/// Path-boundary URI matcher. A bare equality match passes; a
/// prefix match requires the next character after the prefix to
/// be `/` or end-of-string. This blocks the
/// `01` → `01ATTACKER` confusion class without forcing every
/// caller to remember the trailing-slash convention.
fn uri_matches_with_path_boundary(allow: &str, candidate: &str) -> bool {
    if allow == candidate {
        return true;
    }
    if let Some(rest) = candidate.strip_prefix(allow) {
        return rest.starts_with('/');
    }
    false
}

/// Per RFC §1.6, advisory hints about an ability's behavior. Not
/// authoritative — admission policies must not rely on these alone
/// — but useful for UI presentation and for the consent profile
/// when classifying risk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityHints {
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub streaming_only: bool,
}

/// Per RFC §1.6, the JSON Schemas describing an ability's input
/// and the body shape of its receipt. We carry them as
/// `serde_json::Value` so callers can attach existing schemas
/// without forcing a JSON-Schema-typed Rust intermediate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AbilitySchemaSummary {
    /// JSON Schema for the ability's `arguments` field.
    #[serde(default)]
    pub input: Value,
    /// JSON Schema for the receipt body returned on success.
    #[serde(default)]
    pub output_receipt_body: Value,
}

/// The ability descriptor advertised over `federation.advertise_abilities`
/// and returned via `meta.list_abilities`. Built locally by the
/// hosting profile module (P4.2); never mutated after construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbilityDescriptor {
    /// Fully-qualified ability name, e.g. `fleet.list_agents`,
    /// `skill.alive-video`. Per `AgentAbilitySpec::new` validation,
    /// must contain at least one `.` (no namespace = no descriptor).
    pub name: String,
    /// Canonical URA of the Agent that hosts this ability — the
    /// callee in any Invoke targeting this name.
    pub owner_agent_uri: String,
    pub visibility: Visibility,
    pub scope_subjects: ScopeRule,
    pub scope_agents: ScopeRule,
    /// Free-form provenance string, e.g.
    /// `skill_md:~/.claude/skills/alive-video/SKILL.md`,
    /// `manifest:<agent-root>/abilities/foo.ability.toml`, or
    /// `kernel:built-in`.
    pub source: String,
    pub schema_summary: AbilitySchemaSummary,
    pub hints: AbilityHints,
    /// Open-ended metadata bag. P4.4 stores `agent_type` here when
    /// the legacy `AgentType` enum is reshaped.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Construction error. Shape mirrors `AgentAbilitySpec::new`: a
/// short reason string suitable for logging and surfacing back to
/// the operator on a misconfigured manifest.
#[derive(Debug, PartialEq)]
pub enum DescriptorError {
    EmptyName,
    UnnamespacedName,
    EmptyOwner,
}

impl std::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DescriptorError::EmptyName => write!(f, "ability name must not be empty"),
            DescriptorError::UnnamespacedName => {
                write!(f, "ability name must use the `<namespace>.<verb>` shape")
            }
            DescriptorError::EmptyOwner => {
                write!(f, "owner_agent_uri must not be empty")
            }
        }
    }
}

impl std::error::Error for DescriptorError {}

impl AbilityDescriptor {
    /// Construct a descriptor with the protocol-required invariants
    /// validated up-front. Callers building from on-disk manifests
    /// or from the per-profile registration tables should prefer
    /// this constructor over field-wise struct literals so a new
    /// call site cannot ship a malformed descriptor.
    pub fn new(
        name: impl Into<String>,
        owner_agent_uri: impl Into<String>,
        visibility: Visibility,
    ) -> Result<Self, DescriptorError> {
        let name = name.into();
        let owner_agent_uri = owner_agent_uri.into();
        if name.trim().is_empty() {
            return Err(DescriptorError::EmptyName);
        }
        if !name.contains('.') {
            return Err(DescriptorError::UnnamespacedName);
        }
        if owner_agent_uri.trim().is_empty() {
            return Err(DescriptorError::EmptyOwner);
        }
        Ok(Self {
            name,
            owner_agent_uri,
            visibility,
            // Sensible defaults for SCOPED's two axes: any caller
            // from any subject. Builders narrow as needed.
            scope_subjects: ScopeRule::Any,
            scope_agents: ScopeRule::Any,
            source: String::new(),
            schema_summary: AbilitySchemaSummary::default(),
            hints: AbilityHints::default(),
            metadata: HashMap::new(),
        })
    }

    /// Builder: set the source string in one call without exposing
    /// public field mutation to every consumer.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    pub fn with_input_schema(mut self, schema: Value) -> Self {
        self.schema_summary.input = schema;
        self
    }

    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.schema_summary.output_receipt_body = schema;
        self
    }

    pub fn with_hints(mut self, hints: AbilityHints) -> Self {
        self.hints = hints;
        self
    }

    pub fn with_scope_subjects(mut self, rule: ScopeRule) -> Self {
        self.scope_subjects = rule;
        self
    }

    pub fn with_scope_agents(mut self, rule: ScopeRule) -> Self {
        self.scope_agents = rule;
        self
    }

    pub fn with_metadata_entry(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Per RFC §1.6, decide whether this descriptor should be
    /// included in a `federation.resolve` / `meta.list_abilities`
    /// response for the given caller + subject.
    ///
    /// Centralised so a future caller cannot drift the rule.
    pub fn is_visible_to(&self, caller_uri: &str, subject_uri: &str) -> bool {
        match self.visibility {
            Visibility::Public => true,
            Visibility::Scoped => {
                self.scope_subjects.admits(subject_uri)
                    && self.scope_agents.admits(caller_uri)
            }
            Visibility::Private => {
                // Owner's own signing authority can list — that's
                // the host device-profile when the owner is hosted,
                // or the owner itself when self-signed. Until P4.3
                // wires hosted vs self-signed signaling, we accept
                // exact owner match, which is the conservative case
                // (host=owner for self-signed Agents).
                caller_uri == self.owner_agent_uri || subject_uri == self.owner_agent_uri
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must(name: &str, owner: &str, vis: Visibility) -> AbilityDescriptor {
        AbilityDescriptor::new(name, owner, vis).expect("descriptor must construct")
    }

    #[test]
    fn descriptor_constructor_rejects_empty_name() {
        assert_eq!(
            AbilityDescriptor::new("", "u", Visibility::Public).unwrap_err(),
            DescriptorError::EmptyName,
        );
        assert_eq!(
            AbilityDescriptor::new("   ", "u", Visibility::Public).unwrap_err(),
            DescriptorError::EmptyName,
        );
    }

    #[test]
    fn descriptor_constructor_rejects_unnamespaced_name() {
        assert_eq!(
            AbilityDescriptor::new("nodot", "u", Visibility::Public).unwrap_err(),
            DescriptorError::UnnamespacedName,
        );
    }

    #[test]
    fn descriptor_constructor_rejects_empty_owner() {
        assert_eq!(
            AbilityDescriptor::new("a.b", "", Visibility::Public).unwrap_err(),
            DescriptorError::EmptyOwner,
        );
    }

    #[test]
    fn public_visibility_is_visible_to_anyone() {
        let d = must("observe.health", "easynet:///r/acme/agent/01DEV", Visibility::Public);
        assert!(d.is_visible_to("anybody", "anybody"));
        assert!(d.is_visible_to("", ""));
    }

    #[test]
    fn private_visibility_only_visible_to_owner_axis() {
        let owner = "easynet:///r/acme/agent/01LLM";
        let d = must("skill.design", owner, Visibility::Private);
        assert!(d.is_visible_to(owner, "stranger"));
        assert!(d.is_visible_to("stranger", owner));
        assert!(!d.is_visible_to("stranger", "stranger"));
    }

    #[test]
    fn scoped_default_is_any_any_so_admits_everyone() {
        let d = must("conversation.send", "easynet:///r/acme/agent/01LLM", Visibility::Scoped);
        // Defaults set scope_subjects=Any, scope_agents=Any, so until
        // a builder narrows them, SCOPED behaves like PUBLIC. We test
        // this explicitly so the default is documented in code.
        assert!(d.is_visible_to("anybody", "anybody"));
    }

    #[test]
    fn scoped_with_only_matching_subjects_filters_strangers() {
        let owner = "easynet:///r/acme/agent/01LLM";
        let operator = "easynet:///r/acme/agent/01USR-alice";
        let d = must("conversation.send", owner, Visibility::Scoped)
            .with_scope_subjects(ScopeRule::OnlyMatching(vec![operator.into()]));
        assert!(d.is_visible_to("anybody", operator));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/agent/01USR-mallory"));
    }

    #[test]
    fn scoped_both_axes_filtered_requires_both_matches() {
        let backend = "easynet:///r/acme/agent/01BAK";
        let operator = "easynet:///r/acme/agent/01USR-alice";
        let d = must("fleet.list_agents", "easynet:///r/acme/agent/01DEV", Visibility::Scoped)
            .with_scope_subjects(ScopeRule::OnlyMatching(vec![operator.into()]))
            .with_scope_agents(ScopeRule::OnlyMatching(vec![backend.into()]));
        assert!(d.is_visible_to(backend, operator));
        // Right subject, wrong caller — denied.
        assert!(!d.is_visible_to("rogue-caller", operator));
        // Right caller, wrong subject — denied.
        assert!(!d.is_visible_to(backend, "rogue-subject"));
    }

    #[test]
    fn scope_rule_none_denies_everything_on_that_axis() {
        let d = must("admin.failover", "easynet:///r/acme/agent/01DEV", Visibility::Scoped)
            .with_scope_subjects(ScopeRule::None);
        assert!(!d.is_visible_to("anybody", "anybody"));
    }

    #[test]
    fn scope_rule_prefix_match_respects_path_boundary() {
        // §1.6 path-boundary rule: the matched prefix must be
        // followed by `/` or end-of-string. So
        // `easynet:///r/acme/agent/01DEV` matches itself AND
        // `easynet:///r/acme/agent/01DEV/sub` (sub-resource of the
        // same Agent), but NOT `easynet:///r/acme/agent/01DEVATTACKER`
        // — that would let attacker URAs masquerade as authorised
        // ones by sharing a prefix.
        let d = must("fleet.list_agents", "easynet:///r/acme/agent/01HUB", Visibility::Scoped)
            .with_scope_subjects(ScopeRule::OnlyMatching(vec![
                "easynet:///r/acme/agent/01DEV".into(),
            ]));
        assert!(d.is_visible_to("anybody", "easynet:///r/acme/agent/01DEV"));
        assert!(d.is_visible_to("anybody", "easynet:///r/acme/agent/01DEV/sub"));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/agent/01DEVATTACKER"));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/agent/01D"));
    }

    #[test]
    fn descriptor_round_trips_through_serde() {
        let mut metadata = HashMap::new();
        metadata.insert("agent_type".into(), "claude-code".into());
        let d = AbilityDescriptor {
            name: "skill.alive-video".into(),
            owner_agent_uri: "easynet:///r/acme/agent/01LLM".into(),
            visibility: Visibility::Scoped,
            scope_subjects: ScopeRule::OnlyMatching(vec!["operator".into()]),
            scope_agents: ScopeRule::Any,
            source: "skill_md:/path/to/SKILL.md".into(),
            schema_summary: AbilitySchemaSummary {
                input: serde_json::json!({"type":"object"}),
                output_receipt_body: serde_json::json!({"type":"object"}),
            },
            hints: AbilityHints {
                read_only: true,
                ..Default::default()
            },
            metadata,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: AbilityDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn visibility_serde_uses_uppercase_form() {
        let d = must("a.b", "u", Visibility::Public);
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["visibility"], "PUBLIC");
    }

    #[test]
    fn scope_rule_any_serializes_with_kind_tag() {
        let rule = ScopeRule::Any;
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["kind"], "any");
        assert!(json.get("OnlyMatching").is_none());
    }

    #[test]
    fn scope_rule_only_matching_serializes_with_uri_list() {
        let rule = ScopeRule::OnlyMatching(vec!["a".into(), "b".into()]);
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["kind"], "only_matching");
        assert_eq!(json["uris"], serde_json::json!(["a", "b"]));
        let back: ScopeRule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }
}
