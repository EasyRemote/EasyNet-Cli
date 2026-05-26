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
//     for built-in abilities (device.observe.*, device.agent.*, …).
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
//     name, owner_agent_ura, visibility, scope_subjects[],
//     scope_agents[], source, schema_summary{input,
//     output_receipt_body}, hints{read_only, destructive,
//     idempotent, streaming_only, bidi_only}
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
    /// scope_agents empty/Any). Default for `device.*`, `consent.*`,
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

/// Per AXON-RFC-006 §"transition receipts", an ability falls into
/// one of three execution shapes that determine receipt
/// machinery + audit timing:
///
///   * `Query`      — single-shot read. One request, one response,
///                    no state mutation. Receipt is signed at
///                    completion. Examples: `meta.list_abilities`,
///                    `fs.read`, `camera.snapshot`.
///
///   * `Stream`     — long-lived subscribe. One request, N response
///                    frames, an explicit terminal frame. Receipt
///                    covers the bracketed window; per-frame
///                    auditing lives in the stream itself.
///                    Examples: `mic.subscribe`, `camera.subscribe`,
///                    `discuss.subscribe`.
///
///   * `Transition` — state-mutating sequence. The ability hands
///                    back a receipt that itself carries the
///                    pre-/post-state digests so a future
///                    invocation can chain off it. Reserved for
///                    RFC-006 v2 abilities; no caller instantiates
///                    this variant in v1, but the enum carries it
///                    so descriptor renderers / discovery surfaces
///                    can match on it without a wildcard arm.
///
/// `Serialize` / `Deserialize` use the lowercased form so the
/// on-disk descriptor TOMLs and the wire envelope agree
/// byte-for-byte (`"class": "query"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AbilityClass {
    Query,
    Stream,
    Transition,
}

impl AbilityClass {
    /// Lowercase wire form, mirroring the serde rename. Pulled out
    /// as a method so non-serde call sites (descriptor table
    /// rendering, log lines) can stay drift-free with the wire
    /// shape — one rename in this enum updates every consumer.
    pub fn as_str(&self) -> &'static str {
        match self {
            AbilityClass::Query => "query",
            AbilityClass::Stream => "stream",
            AbilityClass::Transition => "transition",
        }
    }

    /// Derive the execution class from transport hints when an
    /// ability manifest has not declared a class explicitly.
    pub fn from_hints(hints: &AbilityHints) -> Self {
        if hints.streaming_only || hints.bidi_only {
            AbilityClass::Stream
        } else {
            AbilityClass::Query
        }
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
    /// `/` or end-of-string) so `easynet:///r/acme/user/alice`
    /// does NOT match `easynet:///r/acme/user/alice-evil`.
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
/// `dev-1` → `dev-1-attacker` confusion class without forcing
/// every caller to remember the trailing-slash convention.
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
    #[serde(default)]
    pub bidi_only: bool,
}

/// Canonical locator for an ability descriptor.
///
/// Descriptor names are presentation names scoped by an owner URA:
/// two hosted agents can both expose `chat`. The callable identity is
/// therefore the full ability URA, built through Axon's URA builders
/// from the descriptor's owner + public verb. Catalogues should key
/// on this value object instead of reconstructing `(owner, name)`
/// tuples at each call site.
///
/// Descriptors whose owner URA does not yet parse (e.g. a daemon
/// that hasn't joined a hub, so the static catalogue is anchored on
/// the literal `"self"` marker) still get a stable identity — a
/// synthetic `pseudo://<owner>/<name>` shape that preserves the
/// "distinct owner → distinct identity" invariant downstream catalog
/// merge depends on. The synthetic form is internal-only: it never
/// leaves this process because the wire `ability_ura` field is
/// recomputed from owner+name and skipped when empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AbilityIdentity {
    /// Either the canonical ability URA, or a `pseudo://...`
    /// fallback when no canonical URA can be derived. Both shapes
    /// are unique per `(owner, public verb)` pair.
    locator: String,
}

impl AbilityIdentity {
    /// Build an identity for `descriptor`. Returns `None` whenever
    /// either the owner URA or the name is blank — both halves are
    /// load-bearing for the `(owner, public verb)` uniqueness
    /// invariant the catalog merge relies on, so a half-filled
    /// descriptor MUST NOT mint a key. Every other case produces a
    /// stable key, falling back to a `pseudo://` form only when the
    /// owner URA does not yet parse (e.g. the daemon has not joined
    /// a hub, so the static catalogue is still anchored on the
    /// literal `"self"` marker).
    pub fn from_descriptor(descriptor: &AbilityDescriptor) -> Option<Self> {
        let owner = descriptor.owner_agent_ura.trim();
        let name = descriptor.name.trim();
        if owner.is_empty() || name.is_empty() {
            return None;
        }
        if let Some(canonical) = descriptor.canonical_ability_ura() {
            if crate::ura::parse_ura(&canonical).is_ok() {
                return Some(Self { locator: canonical });
            }
        }
        Some(Self {
            locator: format!("pseudo://{owner}/{name}"),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.locator
    }

    pub fn into_string(self) -> String {
        self.locator
    }
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
///
/// **Serialization is hand-written** ([`AbilityDescriptor`] impls
/// `Serialize` / `Deserialize` below) so two derived fields can
/// project from canonical state without becoming `pub` writeable
/// caches: `ability_ura` is always recomputed from `owner_agent_ura`
/// + public verb at serialize time and ignored on deserialize, and
/// `class` always emits an effective value even when no explicit
/// override was set.
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDescriptor {
    /// Callable ability name. Device and hub abilities use a
    /// namespaced name such as `device.agent.list`; agent-owned
    /// abilities use the local verb that is scoped by
    /// `owner_agent_ura`, such as `chat`.
    pub name: String,
    /// Canonical URA of the entity that publishes this ability — the
    /// `callee` in any Invoke targeting this name. Per AXON-RFC-001
    /// v4.1.5 §9 (AXIOM seven-tuple), `callee ∈ {hub, device, agent}`,
    /// and this field accepts any of those shapes:
    ///
    ///   * `agent/<user-uuid>.<agent-id>` — hosted user agent
    ///     (consent / policy / mcp / llm sub-agent abilities).
    ///   * `device/<device-uuid>`         — device-built-ins
    ///     (`shell.run`, `fs.read`, `device.agent.list`, …).
    ///   * `hub`                          — hub-published abilities
    ///     (`federation.advertise_*`, `voice.list_calls`, …).
    ///
    /// Field name kept as `owner_agent_ura` for wire-compat with
    /// every existing daemon. §A.URA-5's "agent owns the ability"
    /// rule applies to ABILITY URIs (`/ability/<...>`-shaped) — it
    /// does not constrain who may publish a descriptor for a
    /// device-built-in or hub-built-in verb. A device publishing
    /// `shell.run` is the canonical pattern, not a violation.
    pub owner_agent_ura: String,
    /// Explicit execution-class override. `None` means "derive from
    /// transport hints"; `Some(_)` means a builder called
    /// `with_class(...)` and the descriptor must honor that even if
    /// hints would have steered elsewhere. Internal — read through
    /// [`Self::ability_class`].
    class_override: Option<AbilityClass>,
    pub visibility: Visibility,
    pub scope_subjects: ScopeRule,
    pub scope_agents: ScopeRule,
    /// Human-readable description, surfaced as the MCP tool's
    /// `description` field and on `meta.list_abilities`. Empty
    /// when unknown — the projection layer falls back to the name
    /// in that case rather than fabricating one.
    pub description: String,
    /// Free-form provenance string, e.g.
    /// `skill_md:~/.claude/skills/alive-video/SKILL.md`,
    /// `manifest:<agent-root>/abilities/foo.ability.toml`, or
    /// `kernel:built-in`.
    pub source: String,
    pub schema_summary: AbilitySchemaSummary,
    pub hints: AbilityHints,
    /// Open-ended metadata bag. P4.4 stores `agent_type` here when
    /// the legacy `AgentType` enum is reshaped.
    pub metadata: HashMap<String, String>,
}

// Hand-written Serialize / Deserialize for AbilityDescriptor:
// the wire shape is a flat object with `ability_ura` and `class`
// as fields, but those fields are **derived** in code, not
// independent state. Hiding them behind a serde proxy (rather
// than `pub` cache fields) closes the door on any call site
// mutating them out of sync with the canonical inputs.
impl Serialize for AbilityDescriptor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AbilityDescriptorWire::from_descriptor(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AbilityDescriptor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        AbilityDescriptorWire::deserialize(deserializer).map(AbilityDescriptorWire::into_descriptor)
    }
}

/// Wire-only mirror of `AbilityDescriptor`. Lives next to the
/// canonical type so the on-the-wire field set stays under one
/// source of truth: any field added here must come with an explicit
/// projection from / to the canonical struct, which is what stops
/// `ability_ura` and `class` from drifting back into dual-source
/// fields.
#[derive(Serialize, Deserialize)]
struct AbilityDescriptorWire {
    name: String,
    /// Always populated by the projection layer; ignored on parse —
    /// the canonical value is recomputed from owner + public verb.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ability_ura: String,
    owner_agent_ura: String,
    /// Always emitted at serialize time with the descriptor's
    /// effective class, so consumers always see a concrete value.
    /// On parse, an absent field becomes `None` (the descriptor's
    /// `class_override` stays empty and `ability_class()` derives
    /// from hints); a present field is taken as an explicit
    /// override.
    #[serde(default)]
    class: Option<AbilityClass>,
    visibility: Visibility,
    scope_subjects: ScopeRule,
    scope_agents: ScopeRule,
    #[serde(default)]
    description: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    schema_summary: AbilitySchemaSummary,
    #[serde(default)]
    hints: AbilityHints,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl AbilityDescriptorWire {
    fn from_descriptor(d: &AbilityDescriptor) -> Self {
        let ability_ura = d.canonical_ability_ura().unwrap_or_default();
        // Wire always carries a concrete class. Pinned overrides
        // pass through; unpinned descriptors emit the derived value
        // so a remote consumer (or a re-deserialize round-trip) sees
        // what we would have computed locally.
        let class = Some(d.ability_class());
        Self {
            name: d.name.clone(),
            ability_ura,
            owner_agent_ura: d.owner_agent_ura.clone(),
            class,
            visibility: d.visibility,
            scope_subjects: d.scope_subjects.clone(),
            scope_agents: d.scope_agents.clone(),
            description: d.description.clone(),
            source: d.source.clone(),
            schema_summary: d.schema_summary.clone(),
            hints: d.hints.clone(),
            metadata: d.metadata.clone(),
        }
    }

    fn into_descriptor(self) -> AbilityDescriptor {
        // Wire `ability_ura` is intentionally discarded; the
        // canonical URA is rebuilt from `owner_agent_ura` + name at
        // every read via `canonical_ability_ura()`. Wire `class`,
        // when present, becomes the explicit override.
        AbilityDescriptor {
            name: self.name,
            owner_agent_ura: self.owner_agent_ura,
            class_override: self.class,
            visibility: self.visibility,
            scope_subjects: self.scope_subjects,
            scope_agents: self.scope_agents,
            description: self.description,
            source: self.source,
            schema_summary: self.schema_summary,
            hints: self.hints,
            metadata: self.metadata,
        }
    }
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
                write!(f, "owner_agent_ura must not be empty")
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
        owner_agent_ura: impl Into<String>,
        visibility: Visibility,
    ) -> Result<Self, DescriptorError> {
        let name = name.into();
        let owner_agent_ura = owner_agent_ura.into();
        if name.trim().is_empty() {
            return Err(DescriptorError::EmptyName);
        }
        if owner_agent_ura.trim().is_empty() {
            return Err(DescriptorError::EmptyOwner);
        }
        if !name.contains('.') {
            let agent_owned = crate::ura::parse_ura(&owner_agent_ura)
                .map(|parsed| parsed.kind == crate::ura::URAKind::Agent)
                .unwrap_or(false);
            if !agent_owned {
                return Err(DescriptorError::UnnamespacedName);
            }
        }
        Ok(Self {
            name,
            owner_agent_ura,
            class_override: None,
            visibility,
            // Sensible defaults for SCOPED's two axes: any caller
            // from any subject. Builders narrow as needed.
            scope_subjects: ScopeRule::Any,
            scope_agents: ScopeRule::Any,
            description: String::new(),
            source: String::new(),
            schema_summary: AbilitySchemaSummary::default(),
            hints: AbilityHints::default(),
            metadata: HashMap::new(),
        })
    }

    /// Builder: set the human-readable description in one call.
    /// Trims surrounding whitespace; the projection layer treats
    /// an empty string as "fall back to name".
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into().trim().to_string();
        self
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

    /// Attach transport hints. Does **not** mutate the explicit
    /// class override: `ability_class()` derives from hints only
    /// when `with_class(...)` has not been called. This makes the
    /// builder commutative in (`with_class`, `with_hints`) — call
    /// them in any order and the result is the same.
    pub fn with_hints(mut self, hints: AbilityHints) -> Self {
        self.hints = hints;
        self
    }

    /// Pin the execution class explicitly. Once set, later
    /// `with_hints(...)` calls cannot silently flip it.
    pub fn with_class(mut self, class: AbilityClass) -> Self {
        self.class_override = Some(class);
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

    /// The public verb under this descriptor's owner. Agent-owned
    /// registry entries may arrive as `<agent-id>.<verb>` internally;
    /// this method is the only place that projection is applied.
    pub fn public_name(&self) -> String {
        crate::ura::public_ability_name_for_owner(&self.owner_agent_ura, &self.name)
    }

    /// Canonical ability URA for this descriptor. Always recomputed
    /// from `owner_agent_ura` + `public_name()`; the `ability_ura`
    /// wire field is a one-way serialize-time projection of this
    /// method, not a separate source of truth.
    pub fn canonical_ability_ura(&self) -> Option<String> {
        canonical_ability_ura_for_owner(&self.owner_agent_ura, &self.public_name())
    }

    pub fn identity(&self) -> Option<AbilityIdentity> {
        AbilityIdentity::from_descriptor(self)
    }

    /// Effective execution shape. An explicit `with_class(...)` wins;
    /// otherwise the class is derived from transport hints. Callers
    /// that need to know whether the class was pinned can use
    /// [`Self::class_was_pinned`] instead.
    pub fn ability_class(&self) -> AbilityClass {
        self.class_override
            .unwrap_or_else(|| AbilityClass::from_hints(&self.hints))
    }

    /// `true` iff a builder pinned the class via `with_class(...)`.
    /// Mostly useful for tests and for downstream tooling that must
    /// distinguish "derived from hints" from "operator-asserted".
    pub fn class_was_pinned(&self) -> bool {
        self.class_override.is_some()
    }

    /// Per RFC §1.6, decide whether this descriptor should be
    /// included in a `federation.resolve` / `meta.list_abilities`
    /// response for the given caller + subject.
    ///
    /// Centralised so a future caller cannot drift the rule.
    pub fn is_visible_to(&self, caller_ura: &str, subject_ura: &str) -> bool {
        match self.visibility {
            Visibility::Public => true,
            Visibility::Scoped => {
                self.scope_subjects.admits(subject_ura) && self.scope_agents.admits(caller_ura)
            }
            Visibility::Private => {
                // Owner's own signing authority can list — that's
                // the host device-profile when the owner is hosted,
                // or the owner itself when self-signed. Until P4.3
                // wires hosted vs self-signed signaling, we accept
                // exact owner match, which is the conservative case
                // (host=owner for self-signed Agents).
                caller_ura == self.owner_agent_ura || subject_ura == self.owner_agent_ura
            }
        }
    }
}

fn canonical_ability_ura_for_owner(owner_ura: &str, public_name: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(owner_ura).ok()?;
    match parsed.kind {
        crate::ura::URAKind::Agent => Some(crate::ura::ability_ura(
            &parsed.realm,
            &parsed.user_id,
            &parsed.agent_id,
            public_name,
        )),
        crate::ura::URAKind::Hub if public_name.contains('.') => {
            Some(crate::ura::hub_ability_ura(&parsed.realm, public_name))
        }
        crate::ura::URAKind::Device => Some(crate::ura::ability_ura(
            &parsed.realm,
            "device",
            &parsed.device_id,
            public_name,
        )),
        crate::ura::URAKind::User => Some(crate::ura::ability_ura(
            &parsed.realm,
            "user",
            &parsed.user_id,
            public_name,
        )),
        _ => None,
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
        let d = must(
            "observe.health",
            "easynet:///r/acme/device/dev-1",
            Visibility::Public,
        );
        assert!(d.is_visible_to("anybody", "anybody"));
        assert!(d.is_visible_to("", ""));
    }

    #[test]
    fn private_visibility_only_visible_to_owner_axis() {
        let owner = "easynet:///r/acme/agent/alice.claude";
        let d = must("skill.design", owner, Visibility::Private);
        assert!(d.is_visible_to(owner, "stranger"));
        assert!(d.is_visible_to("stranger", owner));
        assert!(!d.is_visible_to("stranger", "stranger"));
    }

    #[test]
    fn scoped_default_is_any_any_so_admits_everyone() {
        let d = must(
            "conversation.send",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        // Defaults set scope_subjects=Any, scope_agents=Any, so until
        // a builder narrows them, SCOPED behaves like PUBLIC. We test
        // this explicitly so the default is documented in code.
        assert!(d.is_visible_to("anybody", "anybody"));
    }

    #[test]
    fn scoped_with_only_matching_subjects_filters_strangers() {
        let owner = "easynet:///r/acme/agent/alice.claude";
        let operator = "easynet:///r/acme/user/alice";
        let d = must("conversation.send", owner, Visibility::Scoped)
            .with_scope_subjects(ScopeRule::OnlyMatching(vec![operator.into()]));
        assert!(d.is_visible_to("anybody", operator));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/user/mallory"));
    }

    #[test]
    fn scoped_both_axes_filtered_requires_both_matches() {
        let backend = "easynet:///r/acme/hub";
        let operator = "easynet:///r/acme/user/alice";
        let d = must(
            "device.agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        )
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
        let d = must(
            "admin.failover",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        )
        .with_scope_subjects(ScopeRule::None);
        assert!(!d.is_visible_to("anybody", "anybody"));
    }

    #[test]
    fn scope_rule_prefix_match_respects_path_boundary() {
        // §1.6 path-boundary rule: the matched prefix must be
        // followed by `/` or end-of-string. So
        // `easynet:///r/acme/device/dev-1` matches itself AND
        // `easynet:///r/acme/device/dev-1/sub` (sub-resource of
        // the same device), but NOT
        // `easynet:///r/acme/device/dev-1-attacker` — that would
        // let attacker URAs masquerade as authorised ones by
        // sharing a prefix.
        let d = must(
            "device.agent.list",
            "easynet:///r/acme/hub",
            Visibility::Scoped,
        )
        .with_scope_subjects(ScopeRule::OnlyMatching(vec![
            "easynet:///r/acme/device/dev-1".into(),
        ]));
        assert!(d.is_visible_to("anybody", "easynet:///r/acme/device/dev-1"));
        assert!(d.is_visible_to("anybody", "easynet:///r/acme/device/dev-1/sub"));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/device/dev-1-attacker"));
        assert!(!d.is_visible_to("anybody", "easynet:///r/acme/device/dev-"));
    }

    #[test]
    fn identity_rejects_half_filled_descriptor() {
        // The constructor enforces non-empty name + owner, but a
        // caller that mutates the struct after construction must not
        // be able to mint a ghost identity (`pseudo://owner/` or
        // `pseudo:///name`) that the catalog dedup would treat as
        // unique-per-blank. Both halves are load-bearing for the
        // `(owner, public verb)` uniqueness invariant — either being
        // blank means there is nothing to identify.
        let mut blank_name = must(
            "device.agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        blank_name.name = "   ".into();
        assert!(blank_name.identity().is_none());

        let mut blank_owner = must(
            "device.agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        blank_owner.owner_agent_ura = "  ".into();
        assert!(blank_owner.identity().is_none());
    }

    #[test]
    fn descriptor_exposes_canonical_agent_ability_identity() {
        let d = must(
            "backend-engineer.chat",
            "easynet:///r/acme/agent/alice.backend-engineer",
            Visibility::Scoped,
        );
        assert_eq!(d.public_name(), "chat");
        assert_eq!(
            d.canonical_ability_ura().as_deref(),
            Some("easynet:///r/acme/ability/alice.backend-engineer.chat")
        );
        assert_eq!(
            d.identity().map(|id| id.into_string()),
            Some("easynet:///r/acme/ability/alice.backend-engineer.chat".to_string())
        );
    }

    #[test]
    fn descriptor_identity_keeps_same_public_name_distinct_per_owner() {
        let anthropic = must(
            "chat",
            "easynet:///r/acme/agent/alice.anthropic",
            Visibility::Scoped,
        );
        let backend = must(
            "chat",
            "easynet:///r/acme/agent/alice.backend-engineer",
            Visibility::Scoped,
        );
        assert_eq!(anthropic.public_name(), "chat");
        assert_eq!(backend.public_name(), "chat");
        assert_ne!(anthropic.identity(), backend.identity());
    }

    #[test]
    fn ability_class_is_derived_from_transport_hints() {
        let query = must(
            "device.agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        assert_eq!(query.ability_class(), AbilityClass::Query);
        assert!(!query.class_was_pinned());

        let stream = query.clone().with_hints(AbilityHints {
            streaming_only: true,
            ..Default::default()
        });
        assert_eq!(stream.ability_class(), AbilityClass::Stream);
        assert!(!stream.class_was_pinned());

        let transition = query.with_class(AbilityClass::Transition);
        assert_eq!(transition.ability_class(), AbilityClass::Transition);
        assert!(transition.class_was_pinned());
        // Hints arriving AFTER an explicit override do not flip the
        // pinned class — this is the regression the audit caught.
        let transition_after_hints = transition.with_hints(AbilityHints {
            streaming_only: true,
            ..Default::default()
        });
        assert_eq!(
            transition_after_hints.ability_class(),
            AbilityClass::Transition,
            "with_hints must not silently overwrite a class pinned by with_class"
        );
    }

    #[test]
    fn descriptor_round_trips_through_serde() {
        // Wire round-trip equivalence: serialize → deserialize →
        // serialize again must produce byte-identical JSON. We assert
        // wire-equivalence (not full struct equality) because the
        // wire `class` field cannot distinguish an explicit override
        // of `Query` from "no override + derived Query", so the
        // `class_override` internal field is intentionally one-way
        // lossy across the wire. Every other field round-trips
        // exactly.
        let d = must(
            "skill.alive-video",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        )
        .with_scope_subjects(ScopeRule::OnlyMatching(vec!["operator".into()]))
        .with_description("render alive video clips")
        .with_source("skill_md:/path/to/SKILL.md")
        .with_input_schema(serde_json::json!({"type":"object"}))
        .with_output_schema(serde_json::json!({"type":"object"}))
        .with_hints(AbilityHints {
            read_only: true,
            ..Default::default()
        })
        .with_metadata_entry("agent_type", "claude-code");
        let first = serde_json::to_string(&d).unwrap();
        let back: AbilityDescriptor = serde_json::from_str(&first).unwrap();
        let second = serde_json::to_string(&back).unwrap();
        assert_eq!(first, second, "wire form must be stable under round-trip");
        assert_eq!(back.name, d.name);
        assert_eq!(back.owner_agent_ura, d.owner_agent_ura);
        assert_eq!(back.visibility, d.visibility);
        assert_eq!(back.scope_subjects, d.scope_subjects);
        assert_eq!(back.scope_agents, d.scope_agents);
        assert_eq!(back.description, d.description);
        assert_eq!(back.source, d.source);
        assert_eq!(back.schema_summary, d.schema_summary);
        assert_eq!(back.hints, d.hints);
        assert_eq!(back.metadata, d.metadata);
        assert_eq!(back.ability_class(), d.ability_class());
        assert_eq!(back.canonical_ability_ura(), d.canonical_ability_ura());
    }

    #[test]
    fn serialized_ability_ura_is_always_derived_not_round_tripped() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let mut json: serde_json::Value = serde_json::to_value(&d).unwrap();
        // Wire MUST carry the canonical URA regardless of how the
        // descriptor was built.
        assert_eq!(
            json["ability_ura"],
            serde_json::json!("easynet:///r/acme/ability/alice.claude.chat")
        );
        // Tamper with the wire form: a malicious / buggy upstream
        // sends a URA that does not match owner+name. Deserialization
        // must ignore it and recompute.
        json["ability_ura"] = serde_json::json!("easynet:///r/acme/ability/mallory.evil.chat");
        let back: AbilityDescriptor = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.canonical_ability_ura().as_deref(),
            Some("easynet:///r/acme/ability/alice.claude.chat"),
            "wire ability_ura must never override the locally-derived value"
        );
    }

    #[test]
    fn wire_class_field_is_always_present_with_effective_value() {
        // Unpinned + streaming hints → emit "stream" on the wire even
        // though `class_override` is None internally.
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        )
        .with_hints(AbilityHints {
            streaming_only: true,
            ..Default::default()
        });
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["class"], serde_json::json!("stream"));
        assert!(!d.class_was_pinned());
    }

    #[test]
    fn with_class_and_with_hints_are_commutative() {
        let owner = "easynet:///r/acme/device/dev-1";
        // Order A: pin class, then add streaming hints.
        let a = must("device.agent.list", owner, Visibility::Scoped)
            .with_class(AbilityClass::Query)
            .with_hints(AbilityHints {
                streaming_only: true,
                ..Default::default()
            });
        // Order B: same calls reversed.
        let b = must("device.agent.list", owner, Visibility::Scoped)
            .with_hints(AbilityHints {
                streaming_only: true,
                ..Default::default()
            })
            .with_class(AbilityClass::Query);
        assert_eq!(a.ability_class(), AbilityClass::Query);
        assert_eq!(b.ability_class(), AbilityClass::Query);
        assert_eq!(a, b);
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
