// EasyNet CLI — AbilityDescriptor (RFC-001 §1.6 / §A15)
// =======================================================
//
// File: src/daemon/ability/descriptors/surface.rs
//
// Per AXON-RFC-001 plan v4.1.2 §1.6, every advertised Ability is
// described by an AbilityDescriptor. This is the Rust home for
// that schema. AbilityDescriptor supersedes earlier metadata projections at
// the protocol-facing edge: anything that goes to
// `federation.advertise_abilities` (P4.6) or back to a caller via
// `meta.list_abilities` MUST flow through this struct.
//
// Why a fresh module instead of mutating the existing types
// ---------------------------------------------------------
// Persistence manifests remain import DTOs. Catalogues, profiles, discovery,
// and publication exchange this governed aggregate directly so no reduced
// metadata object can silently drop policy, receipt, version, or hash inputs.
//
// The minimum viable scope for P4.1
// ---------------------------------
// Per the plan §1.6 schema:
//
//   AbilityDescriptor {
//     name, owner_ura, visibility, scope_subjects[],
//     scope_agents[], source, schema_summary{input,
//     output_receipt_body}, hints{read_only, destructive,
//     idempotent}
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
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::CallMode;

/// Authorization semantics bound to the exact governed descriptor.
/// Transport geometry alone cannot distinguish a read RPC from a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionAction {
    Read,
    Invoke,
    Stream,
    Manage,
    Grant,
}

impl AdmissionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Invoke => "invoke",
            Self::Stream => "stream",
            Self::Manage => "manage",
            Self::Grant => "grant",
        }
    }
}

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

/// Receipt class declared by an RFC-006 state transition.
///
/// This is deliberately independent of [`CallMode`]. A canonical transition
/// may be unary, streaming, or bidirectional; transport shape cannot prove
/// whether a receipt advances canonical state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionClass {
    /// Runtime/observability transition. Its receipt never advances the
    /// canonical state chain.
    Operational,
    /// Canonical state transition. Its terminal receipt binds pre/post state
    /// hashes and advances the state object's canonical version.
    Canonical,
}

impl TransitionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Canonical => "canonical",
        }
    }
}

/// Validated RFC-006 state-transition declaration.
///
/// `transition_id` is schema identity, not an invocation transport selector.
/// It has the stable `<ability-qualified-name>@v<integer>` form required by
/// RFC-006 and therefore remains meaningful across descriptor revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateTransition {
    transition_id: String,
    transition_class: TransitionClass,
}

impl<'de> Deserialize<'de> for StateTransition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            transition_id: String,
            transition_class: TransitionClass,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.transition_id, wire.transition_class).map_err(serde::de::Error::custom)
    }
}

impl StateTransition {
    pub fn new(
        transition_id: impl Into<String>,
        transition_class: TransitionClass,
    ) -> Result<Self, StateTransitionError> {
        let transition_id = transition_id.into();
        validate_transition_id(&transition_id)?;
        Ok(Self {
            transition_id,
            transition_class,
        })
    }

    pub fn transition_id(&self) -> &str {
        &self.transition_id
    }

    pub fn transition_class(&self) -> TransitionClass {
        self.transition_class
    }

    fn validate(&self) -> Result<(), StateTransitionError> {
        validate_transition_id(&self.transition_id)
    }
}

/// Receipt/state-machine semantics of an ability descriptor.
///
/// Ordinary calls emit operational invocation receipts. A state transition
/// additionally declares the transition schema identity and whether the
/// terminal receipt is canonical or operational. Neither variant implies a
/// transport mode; [`AbilityDescriptor::call_mode`] is the sole transport
/// source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "transition", rename_all = "snake_case")]
pub enum ReceiptSemantics {
    Operational,
    StateTransition(StateTransition),
}

impl ReceiptSemantics {
    pub fn state_transition(
        transition_id: impl Into<String>,
        transition_class: TransitionClass,
    ) -> Result<Self, StateTransitionError> {
        StateTransition::new(transition_id, transition_class).map(Self::StateTransition)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::StateTransition(_) => "state_transition",
        }
    }

    pub fn transition(&self) -> Option<&StateTransition> {
        match self {
            Self::Operational => None,
            Self::StateTransition(transition) => Some(transition),
        }
    }

    fn validate(&self) -> Result<(), StateTransitionError> {
        match self {
            Self::Operational => Ok(()),
            Self::StateTransition(transition) => transition.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionError {
    transition_id: String,
}

impl StateTransitionError {
    pub fn transition_id(&self) -> &str {
        &self.transition_id
    }
}

impl std::fmt::Display for StateTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "transition_id {:?} must use `<ability-qualified-name>@v<positive-integer>`",
            self.transition_id
        )
    }
}

impl std::error::Error for StateTransitionError {}

fn validate_transition_id(transition_id: &str) -> Result<(), StateTransitionError> {
    let valid = transition_id
        .rsplit_once("@v")
        .is_some_and(|(ability, version)| {
            super::is_valid_ability_name(ability)
                && !version.is_empty()
                && version.bytes().all(|byte| byte.is_ascii_digit())
                && version.parse::<u64>().is_ok_and(|version| version > 0)
        });
    if valid {
        Ok(())
    } else {
        Err(StateTransitionError {
            transition_id: transition_id.to_string(),
        })
    }
}

/// Per RFC §1.6, each scope axis (subject vs caller) is a rule.
/// Modeled as an enum so an empty `Vec` cannot accidentally mean
/// "no restriction" — it means "explicit deny-all" (None).
///
/// Uses serde `tag = "kind", content = "uras"` so the wire shape is
/// unambiguous: `{"kind":"any"}`, `{"kind":"none"}`, or
/// `{"kind":"only_matching","uras":["…"]}`. Adjacent-tagged because
/// internally-tagged would conflict with the named-content shape
/// for the OnlyMatching variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "uras", rename_all = "snake_case")]
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

impl ScopeRule {
    /// `true` iff this rule admits the given URA.
    pub fn admits(&self, candidate_ura: &str) -> bool {
        match self {
            ScopeRule::Any => true,
            ScopeRule::None => false,
            ScopeRule::OnlyMatching(allowed) => allowed
                .iter()
                .any(|allow| ura_matches_with_path_boundary(allow, candidate_ura)),
        }
    }

    fn admits_agent(&self, candidate: &str) -> bool {
        match self {
            ScopeRule::Any => true,
            ScopeRule::None => false,
            ScopeRule::OnlyMatching(allowed) => allowed
                .iter()
                .any(|allow| agent_policy_identity_matches(allow, candidate)),
        }
    }
}

/// Path-boundary URA matcher. A bare equality match passes; a
/// prefix match requires the next character after the prefix to
/// be `/` or end-of-string. This blocks the
/// `dev-1` → `dev-1-attacker` confusion class without forcing
/// every caller to remember the trailing-slash convention.
fn ura_matches_with_path_boundary(allow: &str, candidate: &str) -> bool {
    if allow == candidate {
        return true;
    }
    if let Some(rest) = candidate.strip_prefix(allow) {
        return rest.starts_with('/');
    }
    false
}

fn agent_policy_identity_matches(allowed: &str, candidate: &str) -> bool {
    if ura_matches_with_path_boundary(allowed, candidate) {
        return true;
    }
    let Ok(parsed) = crate::core::ura::parse_ura(candidate) else {
        return false;
    };
    let candidate_agent = parsed
        .agent_ids()
        .map(|(_, agent_id)| agent_id)
        .or_else(|| parsed.device_agent_ids().map(|(_, agent_id)| agent_id));
    candidate_agent.is_some_and(|agent_id| allowed == agent_id)
}

fn normalized_policy_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AbilityDescriptorWireHints {
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    destructive: bool,
    #[serde(default)]
    idempotent: bool,
    #[serde(default)]
    streaming_only: bool,
    #[serde(default)]
    bidi_only: bool,
}

impl AbilityDescriptorWireHints {
    fn from_hints_and_call_mode(hints: &AbilityHints, call_mode: CallMode) -> Self {
        Self {
            read_only: hints.read_only,
            destructive: hints.destructive,
            idempotent: hints.idempotent,
            streaming_only: call_mode == CallMode::Stream,
            bidi_only: call_mode == CallMode::Bidi,
        }
    }

    fn into_canonical_hints(self, call_mode: CallMode) -> Result<AbilityHints, String> {
        if self.streaming_only != (call_mode == CallMode::Stream)
            || self.bidi_only != (call_mode == CallMode::Bidi)
        {
            return Err(format!(
                "wire transport hints conflict with canonical call_mode {:?}",
                call_mode
            ));
        }
        Ok(AbilityHints {
            read_only: self.read_only,
            destructive: self.destructive,
            idempotent: self.idempotent,
        })
    }
}

pub(crate) fn ability_hints_wire_value(hints: &AbilityHints, call_mode: CallMode) -> Value {
    serde_json::to_value(AbilityDescriptorWireHints::from_hints_and_call_mode(
        hints, call_mode,
    ))
    .expect("ability descriptor wire hints serialize")
}

pub(crate) fn ability_hints_wire_json(hints: &AbilityHints, call_mode: CallMode) -> String {
    serde_json::to_string(&AbilityDescriptorWireHints::from_hints_and_call_mode(
        hints, call_mode,
    ))
    .expect("ability descriptor wire hints serialize")
}

pub(crate) fn ability_hints_from_wire_json(
    raw: &str,
    call_mode: CallMode,
) -> Result<AbilityHints, String> {
    serde_json::from_str::<AbilityDescriptorWireHints>(raw)
        .map_err(|error| format!("invalid hints_json: {error}"))?
        .into_canonical_hints(call_mode)
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
/// The locator is always a canonical Ability URA. There is no synthetic
/// pre-join identity: a descriptor without a canonical authority owner is
/// not routable and therefore has no identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AbilityIdentity {
    /// Canonical Ability URA, unique per `(authority owner, public verb)`.
    locator: String,
}

impl AbilityIdentity {
    /// Build an identity for `descriptor`. Returns `None` if public field
    /// mutation has broken the descriptor invariant after construction.
    pub fn from_descriptor(descriptor: &AbilityDescriptor) -> Option<Self> {
        let canonical = descriptor.canonical_ability_ura()?;
        crate::core::ura::AbilitySelector::parse(&canonical).ok()?;
        Some(Self { locator: canonical })
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
/// `Serialize` / `Deserialize` below) so derived wire fields can
/// project from canonical state without becoming `pub` writeable
/// caches: wire `name` is the owner-local public verb,
/// `ability_ura` is always recomputed from `owner_ura` +
/// public verb at serialize time and ignored on deserialize, and
/// `call_mode` and `receipt_semantics` are emitted as orthogonal governed
/// facts: transport selection never implies a state transition.
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDescriptor {
    /// Callable owner-local public ability name. Device and realm Authority abilities
    /// use a namespaced public name such as `agent.list` or
    /// `federation.resolve`; agent-owned abilities use the local verb scoped
    /// by `owner_ura`, such as `chat`.
    pub name: String,
    /// Canonical URA of the entity that publishes this ability — the
    /// `callee` in any Invoke targeting this name. Per AXON-RFC-001
    /// v4.1.5 §9 (AXIOM seven-tuple), `callee ∈ {authority, device, agent}`,
    /// and this field accepts any of those shapes:
    ///
    ///   * `agent/<user-uuid>.<agent-id>` — hosted user agent
    ///     (consent / policy / mcp / llm sub-agent abilities).
    ///   * `device/<device-uuid>`         — device-built-ins
    ///     (`shell.run`, `fs.read`, `agent.list`, …).
    ///   * `authority`                    — realm Authority-published abilities
    ///     (`federation.advertise_*`, `voice.list_calls`, …).
    ///
    /// Field name kept as `owner_ura` for wire-compat with
    /// every existing daemon. §A.URA-5's agent-scoped ability URA
    /// rule applies to `/ability/<...>`-shaped URAs — it does not
    /// constrain who may publish a descriptor for a
    /// device-built-in or realm Authority-built-in verb. A device publishing
    /// `shell.run` is the canonical pattern, not a violation.
    pub owner_ura: String,
    /// Governed interface version. This is distinct from
    /// `AbilityManifest.schema_version`, which versions the TOML file
    /// format rather than the callable interface contract.
    pub version: String,
    /// Canonical invocation transport. This is the only transport authority
    /// used by descriptor hashing, routing, and public projection.
    call_mode: CallMode,
    admission_action: AdmissionAction,
    /// Receipt/state-machine classification, independent of transport.
    receipt_semantics: ReceiptSemantics,
    pub visibility: Visibility,
    pub scope_subjects: ScopeRule,
    pub scope_agents: ScopeRule,
    /// Canonical deny set for caller Agents. Values are sorted, deduplicated,
    /// and evaluated before visibility or allow scope; deny always wins.
    denied_agents: Vec<String>,
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
// the wire shape is a flat object with `ability_ura`, `call_mode`, and
// `receipt_semantics`
// as fields, but those fields are **derived** in code, not
// independent state. Hiding them behind a serde proxy (rather
// than `pub` cache fields) closes the door on any call site
// mutating them out of sync with the canonical inputs.
impl Serialize for AbilityDescriptor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AbilityDescriptorWire::try_from_descriptor(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AbilityDescriptor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        AbilityDescriptorWire::deserialize(deserializer)?
            .try_into_descriptor()
            .map_err(serde::de::Error::custom)
    }
}

/// Wire-only mirror of `AbilityDescriptor`. Lives next to the
/// canonical type so the on-the-wire field set stays under one
/// source of truth: any field added here must come with an explicit
/// projection from / to the canonical struct, which is what stops
/// derived identity fields from drifting back into dual-source fields.
#[derive(Serialize, Deserialize)]
struct AbilityDescriptorWire {
    /// Owner-local public ability name. Local registry prefixes
    /// such as `device.` or `<agent-id>.` are not serialized.
    name: String,
    /// Always populated by the projection layer; validated on parse
    /// against owner + public verb so wire state cannot drift from the
    /// canonical descriptor identity.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ability_ura: String,
    owner_ura: String,
    version: String,
    #[serde(default)]
    schema_hash: String,
    #[serde(default)]
    descriptor_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    descriptor_ref: String,
    call_mode: CallMode,
    admission_action: AdmissionAction,
    receipt_semantics: ReceiptSemantics,
    visibility: Visibility,
    scope_subjects: ScopeRule,
    scope_agents: ScopeRule,
    #[serde(default)]
    denied_agents: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    schema_summary: AbilitySchemaSummary,
    /// Canonical public SDK projection of `schema_summary.input`.
    ///
    /// This is a serialize-only field: descriptor governance and hashing own
    /// schema state through `schema_summary`, while SDK consumers read the
    /// product-facing `input_schema` field without reverse-mapping retired
    /// aliases. Deserialization deliberately ignores this projection so a
    /// caller cannot create a second schema authority.
    #[serde(default, skip_deserializing)]
    input_schema: Value,
    #[serde(default)]
    hints: AbilityDescriptorWireHints,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl AbilityDescriptorWire {
    fn try_from_descriptor(d: &AbilityDescriptor) -> Result<Self, String> {
        AbilityDescriptor::validate_owner_ura(&d.owner_ura).map_err(|error| error.to_string())?;
        let name = d.public_name();
        let ability_ura =
            crate::core::ura::owner_ability_ura(&d.owner_ura, &name).ok_or_else(|| {
                format!(
                    "descriptor owner {:?} and name {name:?} do not derive a canonical Ability URA",
                    d.owner_ura
                )
            })?;
        crate::core::ura::AbilitySelector::parse(&ability_ura)
            .map_err(|error| format!("invalid derived Ability URA {ability_ura:?}: {error}"))?;
        let descriptor_hash = d.descriptor_hash_prefixed();
        let descriptor_hash_hex = descriptor_hash
            .strip_prefix("sha256:")
            .ok_or_else(|| format!("descriptor_hash {descriptor_hash:?} is not sha256-prefixed"))?;
        let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{}@{}#{}!{}",
            ability_ura,
            d.version,
            descriptor_hash_hex,
            d.admission_action.as_str()
        ))
        .map_err(|error| format!("invalid derived descriptor_ref: {error}"))?;
        Ok(Self {
            name,
            ability_ura,
            owner_ura: d.owner_ura.clone(),
            version: d.version.clone(),
            schema_hash: d.schema_hash_prefixed(),
            descriptor_hash,
            descriptor_ref,
            call_mode: d.call_mode,
            admission_action: d.admission_action,
            receipt_semantics: d.receipt_semantics.clone(),
            visibility: d.visibility,
            scope_subjects: d.scope_subjects.clone(),
            scope_agents: d.scope_agents.clone(),
            denied_agents: d.denied_agents.clone(),
            description: d.description.clone(),
            source: d.source.clone(),
            schema_summary: d.schema_summary.clone(),
            input_schema: d.schema_summary.input.clone(),
            hints: AbilityDescriptorWireHints::from_hints_and_call_mode(&d.hints, d.call_mode),
            metadata: d.metadata.clone(),
        })
    }

    fn try_into_descriptor(self) -> Result<AbilityDescriptor, String> {
        let wire_ability_ura = self.ability_ura.trim().to_string();
        let wire_schema_hash = self.schema_hash.trim().to_string();
        let wire_descriptor_hash = self.descriptor_hash.trim().to_string();
        let wire_descriptor_ref = self.descriptor_ref.trim().to_string();
        let version = self.version.trim().to_string();
        crate::daemon::ability::descriptors::AbilityDescriptorVersion::new(version.clone())
            .map_err(|err| format!("invalid descriptor version {version:?}: {err}"))?;

        self.receipt_semantics
            .validate()
            .map_err(|err| err.to_string())?;
        AbilityDescriptor::new(
            &self.name,
            &self.owner_ura,
            self.visibility,
            self.admission_action,
        )
        .map_err(|error| error.to_string())?;
        let normalized_denied_agents = normalized_policy_values(self.denied_agents.clone());
        if normalized_denied_agents != self.denied_agents {
            return Err("wire denied_agents must be sorted, deduplicated, and non-empty".into());
        }
        let hints = self.hints.into_canonical_hints(self.call_mode)?;

        // Wire identity/hash fields are independently validated below.
        let descriptor = AbilityDescriptor {
            name: self.name,
            owner_ura: self.owner_ura,
            version,
            call_mode: self.call_mode,
            admission_action: self.admission_action,
            receipt_semantics: self.receipt_semantics,
            visibility: self.visibility,
            scope_subjects: self.scope_subjects,
            scope_agents: self.scope_agents,
            denied_agents: self.denied_agents,
            description: self.description,
            source: self.source,
            schema_summary: self.schema_summary,
            hints,
            metadata: self.metadata,
        };

        if !wire_ability_ura.is_empty() {
            let Some(canonical_ability_ura) = descriptor.canonical_ability_ura() else {
                return Err(format!(
                    "wire ability_ura {wire_ability_ura:?} cannot be validated for owner {:?} \
                     and name {:?}",
                    descriptor.owner_ura,
                    descriptor.public_name()
                ));
            };
            if wire_ability_ura != canonical_ability_ura {
                return Err(format!(
                    "wire ability_ura {wire_ability_ura:?} does not match canonical \
                     ability_ura {canonical_ability_ura:?}"
                ));
            }
        }
        if !wire_schema_hash.is_empty() {
            let actual = descriptor.schema_hash_prefixed();
            if wire_schema_hash != actual {
                return Err(format!(
                    "wire schema_hash {wire_schema_hash:?} does not match computed {actual:?}"
                ));
            }
        }
        if !wire_descriptor_hash.is_empty() {
            let actual = descriptor.descriptor_hash_prefixed();
            if wire_descriptor_hash != actual {
                return Err(format!(
                    "wire descriptor_hash {wire_descriptor_hash:?} does not match computed {actual:?}"
                ));
            }
        }
        if !wire_descriptor_ref.is_empty() {
            let actual = descriptor
                .descriptor_ref()
                .map_err(|error| format!("derive canonical descriptor_ref: {error}"))?;
            if wire_descriptor_ref != actual {
                return Err(format!(
                    "wire descriptor_ref {wire_descriptor_ref:?} does not match computed {actual:?}"
                ));
            }
        }
        Ok(descriptor)
    }
}

fn default_descriptor_version() -> String {
    crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION.to_string()
}

/// Construction error. Shape mirrors `AgentAbilitySpec::new`: a
/// short reason string suitable for logging and surfacing back to
/// the operator on a misconfigured manifest.
#[derive(Debug, PartialEq)]
pub enum DescriptorError {
    EmptyName,
    UnnamespacedName,
    EmptyOwner,
    InvalidOwnerUra { owner_ura: String },
    InvalidVersion { version: String },
    InvalidDescriptorIdentity { detail: String },
}

impl std::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DescriptorError::EmptyName => write!(f, "ability name must not be empty"),
            DescriptorError::UnnamespacedName => {
                write!(f, "ability name must use the `<namespace>.<verb>` shape")
            }
            DescriptorError::EmptyOwner => {
                write!(f, "owner_ura must not be empty")
            }
            DescriptorError::InvalidOwnerUra { owner_ura } => write!(
                f,
                "owner_ura {owner_ura:?} must be a canonical Agent, Device, or Authority URA"
            ),
            DescriptorError::InvalidVersion { version } => {
                write!(f, "descriptor version {version:?} is not a valid semver")
            }
            DescriptorError::InvalidDescriptorIdentity { detail } => {
                write!(f, "invalid descriptor identity: {detail}")
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
        owner_ura: impl Into<String>,
        visibility: Visibility,
        admission_action: AdmissionAction,
    ) -> Result<Self, DescriptorError> {
        let name = name.into();
        let owner_ura = owner_ura.into();
        if name.trim().is_empty() {
            return Err(DescriptorError::EmptyName);
        }
        if owner_ura.trim().is_empty() {
            return Err(DescriptorError::EmptyOwner);
        }
        Self::validate_owner_ura(&owner_ura)?;
        if !name.contains('.') {
            let agent_owned = crate::core::ura::parse_ura(&owner_ura)
                .is_ok_and(|parsed| parsed.kind == crate::core::ura::URAKind::Agent);
            if !agent_owned {
                return Err(DescriptorError::UnnamespacedName);
            }
        }
        Ok(Self {
            name,
            owner_ura,
            version: default_descriptor_version(),
            call_mode: CallMode::Rpc,
            admission_action,
            receipt_semantics: ReceiptSemantics::Operational,
            visibility,
            // Sensible defaults for SCOPED's two axes: any caller
            // from any subject. Builders narrow as needed.
            scope_subjects: ScopeRule::Any,
            scope_agents: ScopeRule::Any,
            denied_agents: Vec::new(),
            description: String::new(),
            source: String::new(),
            schema_summary: AbilitySchemaSummary::default(),
            hints: AbilityHints::default(),
            metadata: HashMap::new(),
        })
    }

    /// Validate the authority root that owns an ability descriptor.
    ///
    /// User, Ability, Resource, legacy `self`, and ad-hoc URA schemes are not
    /// ability-publishing authorities. Comparing against the Axon builder's
    /// reconstruction also prevents a merely parseable non-canonical spelling
    /// from becoming a second locator for the same authority.
    pub fn validate_owner_ura(owner_ura: &str) -> Result<(), DescriptorError> {
        let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|_| {
            DescriptorError::InvalidOwnerUra {
                owner_ura: owner_ura.to_string(),
            }
        })?;
        let canonical = match parsed.kind {
            crate::core::ura::URAKind::Agent => parsed
                .device_agent_ids()
                .map(|(device_id, agent_id)| {
                    crate::core::ura::device_agent_ura(&parsed.realm, device_id, agent_id)
                })
                .or_else(|| {
                    parsed.agent_ids().map(|(user_id, agent_id)| {
                        crate::core::ura::agent_ura(&parsed.realm, user_id, agent_id)
                    })
                }),
            crate::core::ura::URAKind::Device => parsed
                .device_id()
                .map(|device_id| crate::core::ura::device_ura(&parsed.realm, device_id)),
            crate::core::ura::URAKind::Authority => Some(crate::core::ura::hub_ura(&parsed.realm)),
            _ => None,
        };
        if canonical.as_deref() == Some(owner_ura) {
            Ok(())
        } else {
            Err(DescriptorError::InvalidOwnerUra {
                owner_ura: owner_ura.to_string(),
            })
        }
    }

    /// Normalize one daemon registry row and its persistence manifest into the
    /// governed descriptor aggregate.
    ///
    /// The manifest is mandatory at this boundary: descriptor publication is
    /// provider-backed and must not synthesize a schema-less owner-only
    /// descriptor from execution rows alone. The manifest contributes version,
    /// schemas, access allow/deny rules, and description exactly once at this
    /// import boundary.
    pub fn from_registry_manifest(
        registry_ability: impl Into<String>,
        owner_ura: impl Into<String>,
        call_mode: CallMode,
        admission_action: AdmissionAction,
        manifest: &crate::daemon::ability::manifest::AbilityManifest,
    ) -> Result<Self, DescriptorError> {
        let registry_ability = registry_ability.into();
        let owner_ura = owner_ura.into();
        let public_name =
            crate::core::ura::descriptor_public_ability_name(&owner_ura, &registry_ability);

        let access = manifest.access();
        let visibility = visibility_from_manifest(access.visibility);
        let mut descriptor =
            Self::new(public_name, owner_ura.clone(), visibility, admission_action)?
                .with_call_mode(call_mode);
        descriptor = descriptor
            .with_version(manifest.descriptor_version())?
            .with_description(manifest.description())
            .with_input_schema(manifest.input_schema().clone())
            .with_scope_subjects(ScopeRule::Any)
            .with_scope_agents(scope_agents_from_manifest(&access))
            .with_denied_agents(access.deny_callers.unwrap_or_default());
        if let Some(output_schema) = manifest.output_schema() {
            descriptor = descriptor.with_output_schema(output_schema.clone());
        }
        Ok(descriptor)
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

    pub fn with_version(mut self, version: impl Into<String>) -> Result<Self, DescriptorError> {
        let version = version.into();
        let version = version.trim().to_string();
        if !crate::daemon::ability::descriptors::is_valid_descriptor_version(&version) {
            return Err(DescriptorError::InvalidVersion { version });
        }
        self.version = version;
        Ok(self)
    }

    pub fn with_input_schema(mut self, schema: Value) -> Self {
        self.schema_summary.input = schema;
        self
    }

    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.schema_summary.output_receipt_body = schema;
        self
    }

    /// Attach advisory behavior hints. Transport is not represented here;
    /// `call_mode` is the sole routing and descriptor-hash authority.
    pub fn with_hints(mut self, hints: AbilityHints) -> Self {
        self.hints = hints;
        self
    }

    /// Select the invocation transport. Public transport hints are derived
    /// from this value at projection boundaries.
    pub fn with_call_mode(mut self, call_mode: CallMode) -> Self {
        self.call_mode = call_mode;
        self
    }

    /// Declare receipt/state-machine semantics without changing transport.
    pub fn with_receipt_semantics(mut self, receipt_semantics: ReceiptSemantics) -> Self {
        receipt_semantics
            .validate()
            .expect("ReceiptSemantics must be constructed with a valid transition_id");
        self.receipt_semantics = receipt_semantics;
        self
    }

    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    /// Re-anchor this governed descriptor on another canonical owner while
    /// preserving every non-identity facet.
    ///
    /// Profile projection uses this instead of reconstructing a descriptor
    /// field-by-field. Self-referential scope entries are rebound to the new
    /// owner; external allow/deny identities, schemas, metadata, transport,
    /// receipt semantics, and version remain unchanged. Hashes are always
    /// derived on demand, so the returned descriptor has the canonical hash
    /// for its new identity and policy without a stale cache.
    pub fn rebind_owner_ura(
        mut self,
        owner_ura: impl Into<String>,
    ) -> Result<Self, DescriptorError> {
        let owner_ura = owner_ura.into();
        let public_name = self.public_name();
        Self::new(
            public_name.clone(),
            owner_ura.clone(),
            self.visibility,
            self.admission_action,
        )?;

        let previous_owner_ura = std::mem::replace(&mut self.owner_ura, owner_ura.clone());
        self.name = public_name;
        self.scope_subjects =
            rebind_owner_scope_rule(self.scope_subjects, &previous_owner_ura, &owner_ura);
        self.scope_agents =
            rebind_owner_scope_rule(self.scope_agents, &previous_owner_ura, &owner_ura);
        Ok(self)
    }

    pub fn with_scope_subjects(mut self, rule: ScopeRule) -> Self {
        self.scope_subjects = rule;
        self
    }

    pub fn with_scope_agents(mut self, rule: ScopeRule) -> Self {
        self.scope_agents = rule;
        self
    }

    pub fn with_denied_agents(mut self, denied_agents: impl IntoIterator<Item = String>) -> Self {
        self.denied_agents = normalized_policy_values(denied_agents);
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
        crate::core::ura::descriptor_public_ability_name(&self.owner_ura, &self.name)
    }

    /// Canonical ability URA for this descriptor. Always recomputed
    /// from `owner_ura` + `public_name()`; the `ability_ura`
    /// wire field is a one-way serialize-time projection of this
    /// method, not a separate source of truth.
    pub fn canonical_ability_ura(&self) -> Option<String> {
        crate::core::ura::owner_ability_ura(&self.owner_ura, &self.public_name())
    }

    pub fn schema_hash_bytes(&self) -> [u8; 32] {
        let governed = self.governed_schema_summary();
        crate::daemon::ability::descriptors::schema_hash_for_governed_summary(&governed).0
    }

    pub fn input_schema(&self) -> &Value {
        &self.schema_summary.input
    }

    pub fn output_receipt_schema(&self) -> &Value {
        &self.schema_summary.output_receipt_body
    }

    fn governed_schema_summary(&self) -> Value {
        crate::daemon::ability::descriptors::governed_schema_summary(
            crate::daemon::ability::descriptors::GovernedSchemaProjection {
                input: &self.schema_summary.input,
                output: &self.schema_summary.output_receipt_body,
                access_policy: self.governed_access_policy_summary(),
                hints: ability_hints_wire_value(&self.hints, self.call_mode),
                receipt_semantics: serde_json::to_value(&self.receipt_semantics)
                    .expect("receipt semantics serialize"),
                admission_action: serde_json::to_value(self.admission_action)
                    .expect("admission action serializes"),
                description: &self.description,
                source: &self.source,
                metadata: serde_json::to_value(&self.metadata)
                    .expect("descriptor metadata serializes"),
            },
        )
    }

    pub fn schema_hash_prefixed(&self) -> String {
        crate::daemon::ability::descriptors::SchemaHash(self.schema_hash_bytes()).prefixed_hex()
    }

    /// Canonical governed access-policy projection used by schema hashing,
    /// authority binding, and diagnostics. Callers receive a value snapshot;
    /// no mutable policy cache exists outside the descriptor.
    pub fn governed_access_policy_summary(&self) -> Value {
        crate::daemon::ability::descriptors::governed_access_policy_summary(
            serde_json::to_value(self.visibility).expect("visibility serializes"),
            governed_scope_rule_value(&self.scope_subjects),
            governed_scope_rule_value(&self.scope_agents),
            serde_json::to_value(&self.denied_agents).expect("denied_agents serialize"),
        )
    }

    pub fn access_policy_hash_bytes(&self) -> [u8; 32] {
        let canonical = crate::daemon::ability::descriptors::canonical_json_bytes(
            &self.governed_access_policy_summary(),
        );
        Sha256::digest(canonical).into()
    }

    pub fn access_policy_hash_prefixed(&self) -> String {
        format!("sha256:{}", hex::encode(self.access_policy_hash_bytes()))
    }

    pub fn descriptor_hash_bytes(&self) -> [u8; 32] {
        let ability_ura = self
            .canonical_ability_ura()
            .expect("validated AbilityDescriptor must have a canonical Ability URA");
        crate::daemon::ability::descriptors::descriptor_hash_for_ability_ura_parts(
            &ability_ura,
            &self.public_name(),
            &self.version,
            self.call_mode,
            crate::daemon::ability::descriptors::SchemaHash(self.schema_hash_bytes()),
        )
        .0
    }

    pub fn descriptor_hash_prefixed(&self) -> String {
        crate::daemon::ability::descriptors::DescriptorHash(self.descriptor_hash_bytes())
            .prefixed_hex()
    }

    pub fn descriptor_ref(&self) -> Result<String, DescriptorError> {
        let public_name = self.public_name();
        let ability_ura = self.canonical_ability_ura().ok_or_else(|| {
            DescriptorError::InvalidDescriptorIdentity {
                detail: format!(
                    "owner {:?} and name {public_name:?} do not derive a canonical Ability URA",
                    self.owner_ura
                ),
            }
        })?;
        let descriptor_hash =
            crate::daemon::ability::descriptors::descriptor_hash_for_ability_ura_parts(
                &ability_ura,
                &public_name,
                &self.version,
                self.call_mode,
                crate::daemon::ability::descriptors::SchemaHash(self.schema_hash_bytes()),
            )
            .prefixed_hex();
        let descriptor_hash_hex = descriptor_hash.strip_prefix("sha256:").ok_or_else(|| {
            DescriptorError::InvalidDescriptorIdentity {
                detail: format!("descriptor_hash {descriptor_hash:?} is not sha256-prefixed"),
            }
        })?;
        axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{}@{}#{}!{}",
            ability_ura,
            self.version,
            descriptor_hash_hex,
            self.admission_action.as_str()
        ))
        .map_err(|error| DescriptorError::InvalidDescriptorIdentity {
            detail: format!("invalid derived descriptor_ref: {error}"),
        })
    }

    pub fn identity(&self) -> Option<AbilityIdentity> {
        AbilityIdentity::from_descriptor(self)
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn admission_action(&self) -> AdmissionAction {
        self.admission_action
    }

    pub fn receipt_semantics(&self) -> &ReceiptSemantics {
        &self.receipt_semantics
    }

    pub fn denied_agents(&self) -> &[String] {
        &self.denied_agents
    }

    /// Per RFC §1.6, decide whether this descriptor should be
    /// included in a `federation.resolve` / `meta.list_abilities`
    /// response for the given caller + subject.
    ///
    /// Centralised so a future caller cannot drift the rule.
    pub fn is_visible_to(&self, caller_ura: &str, subject_ura: &str) -> bool {
        if self
            .denied_agents
            .iter()
            .any(|denied| agent_policy_identity_matches(denied, caller_ura))
        {
            return false;
        }
        match self.visibility {
            Visibility::Public => true,
            Visibility::Scoped => {
                self.scope_subjects.admits(subject_ura)
                    && self.scope_agents.admits_agent(caller_ura)
            }
            Visibility::Private => {
                // Owner's own signing authority can list — that's
                // the host device-profile when the owner is hosted,
                // or the owner itself when self-signed. Until P4.3
                // wires hosted vs self-signed signaling, we accept
                // exact owner match, which is the conservative case
                // (host=owner for self-signed Agents).
                caller_ura == self.owner_ura || subject_ura == self.owner_ura
            }
        }
    }
}

fn visibility_from_manifest(
    visibility: crate::daemon::ability::manifest::ManifestAccessScope,
) -> Visibility {
    match visibility {
        crate::daemon::ability::manifest::ManifestAccessScope::Selfish => Visibility::Private,
        crate::daemon::ability::manifest::ManifestAccessScope::Device => Visibility::Scoped,
        crate::daemon::ability::manifest::ManifestAccessScope::Public => Visibility::Public,
    }
}

fn rebind_owner_scope_rule(
    rule: ScopeRule,
    previous_owner_ura: &str,
    owner_ura: &str,
) -> ScopeRule {
    let ScopeRule::OnlyMatching(values) = rule else {
        return rule;
    };
    ScopeRule::OnlyMatching(
        values
            .into_iter()
            .map(|value| {
                if value == previous_owner_ura {
                    return owner_ura.to_string();
                }
                crate::core::ura::public_ability_name_from_ability_ura(previous_owner_ura, &value)
                    .and_then(|name| crate::core::ura::owner_ability_ura(owner_ura, &name))
                    .unwrap_or(value)
            })
            .collect(),
    )
}

fn scope_agents_from_manifest(
    access: &crate::daemon::ability::manifest::AccessPolicy,
) -> ScopeRule {
    let allowed = normalized_policy_values(access.allow_callers.clone().unwrap_or_default());
    if allowed.is_empty() {
        ScopeRule::Any
    } else {
        ScopeRule::OnlyMatching(allowed)
    }
}

fn governed_scope_rule_value(rule: &ScopeRule) -> Value {
    match rule {
        ScopeRule::OnlyMatching(values) => {
            serde_json::to_value(ScopeRule::OnlyMatching(sorted_scope_values(values)))
                .expect("scope rule serializes")
        }
        other => serde_json::to_value(other).expect("scope rule serializes"),
    }
}

fn sorted_scope_values(values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_OWNER: &str = "easynet:///r/acme/device/dev-1";

    fn must(name: &str, owner: &str, vis: Visibility) -> AbilityDescriptor {
        AbilityDescriptor::new(name, owner, vis, AdmissionAction::Invoke)
            .expect("descriptor must construct")
    }

    #[test]
    fn descriptor_constructor_rejects_empty_name() {
        assert_eq!(
            AbilityDescriptor::new(
                "",
                DEVICE_OWNER,
                Visibility::Public,
                AdmissionAction::Invoke
            )
            .unwrap_err(),
            DescriptorError::EmptyName,
        );
        assert_eq!(
            AbilityDescriptor::new(
                "   ",
                DEVICE_OWNER,
                Visibility::Public,
                AdmissionAction::Invoke
            )
            .unwrap_err(),
            DescriptorError::EmptyName,
        );
    }

    #[test]
    fn descriptor_constructor_rejects_unnamespaced_name() {
        assert_eq!(
            AbilityDescriptor::new(
                "nodot",
                DEVICE_OWNER,
                Visibility::Public,
                AdmissionAction::Invoke,
            )
            .unwrap_err(),
            DescriptorError::UnnamespacedName,
        );
    }

    #[test]
    fn descriptor_constructor_rejects_empty_owner() {
        assert_eq!(
            AbilityDescriptor::new("a.b", "", Visibility::Public, AdmissionAction::Invoke)
                .unwrap_err(),
            DescriptorError::EmptyOwner,
        );
    }

    #[test]
    fn descriptor_constructor_rejects_non_authority_and_legacy_owner_locators() {
        for owner_ura in [
            "self",
            "agent://claude",
            "easynet:///r/acme/user/alice",
            "easynet:///r/acme/ability/device.dev-1.fs.read",
            "easynet:///r/acme/resource/device.dev-1/fs/file",
        ] {
            assert_eq!(
                AbilityDescriptor::new(
                    "fs.read",
                    owner_ura,
                    Visibility::Public,
                    AdmissionAction::Invoke,
                )
                .unwrap_err(),
                DescriptorError::InvalidOwnerUra {
                    owner_ura: owner_ura.to_string()
                },
                "owner {owner_ura:?} must fail closed"
            );
        }
    }

    #[test]
    fn owner_rebind_preserves_governed_facets_and_rebinds_self_scope() {
        let previous_owner = "easynet:///r/acme/device/dev-a";
        let owner = "easynet:///r/acme/device/dev-b";
        let previous_ability =
            crate::core::ura::owner_ability_ura(previous_owner, "fs.read").unwrap();
        let external = "easynet:///r/acme/agent/user.external".to_string();
        let descriptor = must("fs.read", previous_owner, Visibility::Scoped)
            .with_version("2.0.0")
            .unwrap()
            .with_description("Read one file")
            .with_source("daemon:control-plane")
            .with_input_schema(serde_json::json!({"type": "object"}))
            .with_output_schema(serde_json::json!({"type": "string"}))
            .with_call_mode(CallMode::Stream)
            .with_receipt_semantics(
                ReceiptSemantics::state_transition("fs.read@v1", TransitionClass::Operational)
                    .unwrap(),
            )
            .with_scope_subjects(ScopeRule::OnlyMatching(vec![
                previous_owner.to_string(),
                previous_ability,
                external.clone(),
            ]))
            .with_scope_agents(ScopeRule::OnlyMatching(vec![
                previous_owner.to_string(),
                external.clone(),
            ]))
            .with_denied_agents(["mallory".to_string()])
            .with_metadata_entry("runtime", "native");

        let previous_descriptor_hash = descriptor.descriptor_hash_prefixed();
        let rebound = descriptor.rebind_owner_ura(owner).unwrap();

        assert_eq!(rebound.owner_ura, owner);
        assert_eq!(rebound.name, "fs.read");
        assert_eq!(rebound.version, "2.0.0");
        assert_eq!(rebound.call_mode(), CallMode::Stream);
        assert_eq!(
            rebound
                .receipt_semantics()
                .transition()
                .unwrap()
                .transition_id(),
            "fs.read@v1"
        );
        assert_eq!(rebound.description, "Read one file");
        assert_eq!(rebound.source, "daemon:control-plane");
        assert_eq!(
            rebound.input_schema(),
            &serde_json::json!({"type": "object"})
        );
        assert_eq!(
            rebound.output_receipt_schema(),
            &serde_json::json!({"type": "string"})
        );
        assert_eq!(
            serde_json::to_value(&rebound).unwrap()["hints"]["streaming_only"],
            serde_json::json!(true)
        );
        assert_eq!(rebound.denied_agents(), &["mallory"]);
        assert_eq!(
            rebound.metadata.get("runtime").map(String::as_str),
            Some("native")
        );
        assert!(rebound.scope_subjects.admits(owner));
        assert!(rebound.scope_subjects.admits(&external));
        assert!(rebound.scope_agents.admits(owner));
        assert!(rebound.scope_agents.admits(&external));
        assert_ne!(rebound.descriptor_hash_prefixed(), previous_descriptor_hash);

        let wire = serde_json::to_value(&rebound).unwrap();
        let round_trip: AbilityDescriptor = serde_json::from_value(wire).unwrap();
        assert_eq!(round_trip, rebound);
        assert_eq!(
            round_trip.descriptor_hash_prefixed(),
            rebound.descriptor_hash_prefixed()
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
    fn denied_agents_are_canonical_and_override_visibility() {
        let descriptor = must("agent.list", DEVICE_OWNER, Visibility::Public).with_denied_agents([
            " mallory ".to_string(),
            "alice".to_string(),
            "mallory".to_string(),
            "".to_string(),
        ]);
        assert_eq!(descriptor.denied_agents(), &["alice", "mallory"]);
        assert!(!descriptor.is_visible_to("mallory", "anything"));
        assert!(descriptor.is_visible_to("bob", "anything"));

        let wire = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            wire["denied_agents"],
            serde_json::json!(["alice", "mallory"])
        );
    }

    #[test]
    fn denied_agent_name_matches_canonical_agent_ura() {
        let descriptor = must("agent.list", DEVICE_OWNER, Visibility::Public)
            .with_denied_agents(["mallory".to_string()]);
        assert!(!descriptor.is_visible_to(
            "easynet:///r/acme/agent/alice.mallory",
            "easynet:///r/acme/device/dev-1"
        ));
    }

    #[test]
    fn policy_hash_binds_deny_rules() {
        let base = must("agent.list", DEVICE_OWNER, Visibility::Scoped);
        let denied = base.clone().with_denied_agents(["mallory".to_string()]);
        assert_ne!(
            base.access_policy_hash_bytes(),
            denied.access_policy_hash_bytes()
        );
        assert_ne!(base.schema_hash_bytes(), denied.schema_hash_bytes());
        assert_eq!(
            denied.governed_access_policy_summary()["deny_callers"],
            serde_json::json!(["mallory"])
        );
    }

    #[test]
    fn manifest_normalization_projects_schema_mode_and_access_once() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "quote",
            "emit a quote",
            serde_json::json!({"type":"object","required":["topic"]}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap()
        .with_output_schema(serde_json::json!({"type":"object"}))
        .unwrap()
        .with_access(crate::daemon::ability::manifest::AccessPolicy {
            visibility: crate::daemon::ability::manifest::ManifestAccessScope::Public,
            allow_callers: Some(vec!["bob".into(), " alice ".into(), "bob".into()]),
            deny_callers: Some(vec!["mallory".into(), " mallory ".into()]),
        })
        .unwrap();
        let descriptor = AbilityDescriptor::from_registry_manifest(
            "mentor.quote",
            "easynet:///r/acme/agent/alice.mentor",
            CallMode::Stream,
            AdmissionAction::Invoke,
            &manifest,
        )
        .unwrap();

        assert_eq!(descriptor.public_name(), "quote");
        assert_eq!(descriptor.version, "2.0.0");
        assert_eq!(descriptor.call_mode(), CallMode::Stream);
        assert_eq!(descriptor.description, "emit a quote");
        assert_eq!(descriptor.denied_agents(), &["mallory"]);
        assert_eq!(
            descriptor.scope_agents,
            ScopeRule::OnlyMatching(vec!["alice".into(), "bob".into()])
        );
        assert!(descriptor.is_visible_to("easynet:///r/acme/agent/alice.bob", "any-subject"));
        assert!(!descriptor.is_visible_to("easynet:///r/acme/agent/alice.mallory", "any-subject"));
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
        let backend = crate::core::ura::hub_ura("acme");
        let operator = "easynet:///r/acme/user/alice";
        let d = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        )
        .with_scope_subjects(ScopeRule::OnlyMatching(vec![operator.into()]))
        .with_scope_agents(ScopeRule::OnlyMatching(vec![backend.clone()]));
        assert!(d.is_visible_to(&backend, operator));
        // Right subject, wrong caller — denied.
        assert!(!d.is_visible_to("rogue-caller", operator));
        // Right caller, wrong subject — denied.
        assert!(!d.is_visible_to(&backend, "rogue-subject"));
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
            "agent.list",
            &crate::core::ura::hub_ura("acme"),
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
        // be able to mint a ghost identity. Both halves are load-bearing for the
        // `(owner, public verb)` uniqueness invariant — either being
        // blank means there is nothing to identify.
        let mut blank_name = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        blank_name.name = "   ".into();
        assert!(blank_name.identity().is_none());

        let mut blank_owner = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        blank_owner.owner_ura = "  ".into();
        assert!(blank_owner.identity().is_none());
    }

    #[test]
    fn descriptor_ref_derivation_fails_closed_for_corrupt_identity() {
        let mut descriptor = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        descriptor.owner_ura = "not-a-ura".into();
        let error = descriptor
            .descriptor_ref()
            .expect_err("descriptor_ref derivation must fail closed");
        assert!(
            error
                .to_string()
                .contains("do not derive a canonical Ability URA"),
            "unexpected descriptor_ref error: {error}"
        );
    }

    #[test]
    fn descriptor_wire_projection_fails_closed_for_corrupt_identity() {
        let mut descriptor = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        descriptor.name = "   ".into();
        let error =
            serde_json::to_value(&descriptor).expect_err("wire projection must fail closed");
        assert!(
            error
                .to_string()
                .contains("do not derive a canonical Ability URA"),
            "unexpected descriptor wire projection error: {error}"
        );
    }

    #[test]
    fn descriptor_exposes_canonical_agent_ability_identity() {
        let d = must(
            "chat",
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
    fn descriptor_exposes_canonical_device_owned_ability_identity() {
        let d = must(
            "fs.read",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        assert_eq!(d.public_name(), "fs.read");
        assert_eq!(
            d.canonical_ability_ura().as_deref(),
            Some("easynet:///r/acme/ability/device.dev-1.fs.read")
        );
        assert_eq!(
            d.identity().map(|id| id.into_string()),
            Some("easynet:///r/acme/ability/device.dev-1.fs.read".to_string())
        );
    }

    #[test]
    fn descriptor_wire_exposes_canonical_descriptor_ref() {
        let descriptor = must(
            "meta.list_abilities",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        )
        .with_description("List abilities");

        let value = serde_json::to_value(&descriptor).expect("descriptor serializes");
        let descriptor_ref = value["descriptor_ref"]
            .as_str()
            .expect("wire descriptor_ref");

        assert_eq!(
            descriptor_ref.to_string(),
            descriptor
                .descriptor_ref()
                .expect("descriptor derives canonical descriptor_ref")
        );
        let expected_ref_prefix = format!(
            "{}@1.0.0#",
            crate::core::ura::device_ability_ura("acme", "dev-1", "meta.list_abilities")
        );
        assert!(descriptor_ref.starts_with(&expected_ref_prefix));
        assert!(descriptor_ref.ends_with("!invoke"));
    }

    #[test]
    fn descriptor_wire_name_is_owner_local_public_name() {
        let device = must(
            "fs.read",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        let device_json = serde_json::to_value(&device).unwrap();
        assert_eq!(device_json["name"], "fs.read");
        assert_eq!(
            device_json["ability_ura"],
            "easynet:///r/acme/ability/device.dev-1.fs.read"
        );

        let agent = must(
            "chat",
            "easynet:///r/acme/agent/alice.backend-engineer",
            Visibility::Scoped,
        );
        let agent_json = serde_json::to_value(&agent).unwrap();
        assert_eq!(agent_json["name"], "chat");
        assert_eq!(
            agent_json["ability_ura"],
            "easynet:///r/acme/ability/alice.backend-engineer.chat"
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
    fn behavior_hints_do_not_select_call_mode() {
        let rpc = must(
            "agent.list",
            "easynet:///r/acme/device/dev-1",
            Visibility::Scoped,
        );
        assert_eq!(rpc.call_mode(), CallMode::Rpc);

        let hinted = rpc.clone().with_hints(AbilityHints {
            read_only: true,
            ..Default::default()
        });
        assert_eq!(hinted.call_mode(), CallMode::Rpc);
        assert!(hinted.hints.read_only);
        let wire = serde_json::to_value(&hinted).unwrap();
        assert_eq!(wire["hints"]["streaming_only"], serde_json::json!(false));
        assert_eq!(wire["hints"]["bidi_only"], serde_json::json!(false));
    }

    #[test]
    fn state_transition_semantics_are_independent_of_call_mode() {
        let transition =
            ReceiptSemantics::state_transition("page.publish@v2", TransitionClass::Canonical)
                .unwrap();
        let descriptor = must(
            "page.publish",
            "easynet:///r/acme/agent/alice.pages",
            Visibility::Scoped,
        )
        .with_call_mode(CallMode::Stream)
        .with_receipt_semantics(transition);

        assert_eq!(descriptor.call_mode(), CallMode::Stream);
        assert_eq!(
            descriptor
                .receipt_semantics()
                .transition()
                .unwrap()
                .transition_id(),
            "page.publish@v2"
        );
        assert_eq!(
            descriptor
                .receipt_semantics()
                .transition()
                .unwrap()
                .transition_class(),
            TransitionClass::Canonical
        );
        let wire = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(wire["hints"]["streaming_only"], serde_json::json!(true));
        assert_eq!(wire["hints"]["bidi_only"], serde_json::json!(false));
    }

    #[test]
    fn transition_id_is_validated_at_construction() {
        let error = ReceiptSemantics::state_transition(
            "page.publish@not-a-version",
            TransitionClass::Canonical,
        )
        .unwrap_err();
        assert_eq!(error.transition_id(), "page.publish@not-a-version");
    }

    #[test]
    fn descriptor_round_trips_through_serde() {
        // Wire round-trip equivalence: serialize → deserialize → serialize
        // again must produce byte-identical JSON. Call mode and receipt
        // semantics are concrete fields, so the full aggregate round-trips.
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
        assert_eq!(back.owner_ura, d.owner_ura);
        assert_eq!(back.visibility, d.visibility);
        assert_eq!(back.scope_subjects, d.scope_subjects);
        assert_eq!(back.scope_agents, d.scope_agents);
        assert_eq!(back.denied_agents, d.denied_agents);
        assert_eq!(back.description, d.description);
        assert_eq!(back.source, d.source);
        assert_eq!(back.schema_summary, d.schema_summary);
        assert_eq!(back.hints, d.hints);
        assert_eq!(back.metadata, d.metadata);
        assert_eq!(back.call_mode(), d.call_mode());
        assert_eq!(back.receipt_semantics(), d.receipt_semantics());
        assert_eq!(back.canonical_ability_ura(), d.canonical_ability_ura());
    }

    #[test]
    fn wire_projects_input_schema_without_making_it_deserialization_authority() {
        let descriptor = must(
            "admission.explain",
            "easynet:///r/acme/device/device-a",
            Visibility::Scoped,
        )
        .with_input_schema(serde_json::json!({
            "type": "object",
            "required": ["observer_ura"],
            "properties": {"observer_ura": {"type": "string"}},
            "additionalProperties": false
        }));
        let mut wire = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            wire["input_schema"], descriptor.schema_summary.input,
            "public descriptor wire must expose the SDK-owned input_schema projection"
        );

        wire["input_schema"] = serde_json::json!({"type": "string"});
        let parsed: AbilityDescriptor = serde_json::from_value(wire).unwrap();
        assert_eq!(
            parsed.schema_summary.input,
            descriptor.schema_summary.input,
            "input_schema is a serialize-only projection; schema_summary remains the sole descriptor authority"
        );
    }

    #[test]
    fn serialized_ability_ura_is_derived_and_validated_on_parse() {
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
        // must fail closed rather than silently accepting a false
        // identity projection.
        json["ability_ura"] = serde_json::json!("easynet:///r/acme/ability/mallory.evil.chat");
        let err = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not match canonical ability_ura"),
            "{err}"
        );
    }

    #[test]
    fn descriptor_wire_rejects_hash_tampering() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let mut schema_json: serde_json::Value = serde_json::to_value(&d).unwrap();
        schema_json["schema_hash"] = serde_json::json!("sha256:bad");
        let schema_err = serde_json::from_value::<AbilityDescriptor>(schema_json).unwrap_err();
        assert!(
            schema_err.to_string().contains("wire schema_hash"),
            "{schema_err}"
        );

        let mut descriptor_json: serde_json::Value = serde_json::to_value(&d).unwrap();
        descriptor_json["descriptor_hash"] = serde_json::json!("sha256:bad");
        let descriptor_err =
            serde_json::from_value::<AbilityDescriptor>(descriptor_json).unwrap_err();
        assert!(
            descriptor_err.to_string().contains("wire descriptor_hash"),
            "{descriptor_err}"
        );
    }

    #[test]
    fn descriptor_wire_rejects_transport_hint_call_mode_conflicts() {
        let descriptor = must("fs.read", DEVICE_OWNER, Visibility::Scoped);
        let mut json = serde_json::to_value(descriptor).unwrap();
        json["hints"]["streaming_only"] = serde_json::json!(true);

        let err = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();

        assert!(
            err.to_string()
                .contains("transport hints conflict with canonical call_mode"),
            "{err}"
        );
    }

    #[test]
    fn descriptor_wire_rejects_non_canonical_owner_even_without_identity_fields() {
        let descriptor = must("fs.read", DEVICE_OWNER, Visibility::Scoped);
        let mut json = serde_json::to_value(descriptor).unwrap();
        json["owner_ura"] = serde_json::json!("self");
        json.as_object_mut().unwrap().remove("ability_ura");
        json.as_object_mut().unwrap().remove("schema_hash");
        json.as_object_mut().unwrap().remove("descriptor_hash");

        let error = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("canonical Agent, Device, or Authority URA"),
            "{error}"
        );
    }

    #[test]
    fn descriptor_hash_binds_governance_fields() {
        let public = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Public,
        )
        .with_input_schema(serde_json::json!({"type":"object"}));
        let scoped = public
            .clone()
            .with_visibility(Visibility::Scoped)
            .with_scope_agents(ScopeRule::OnlyMatching(vec![
                "easynet:///r/acme/agent/bob".to_string()
            ]));

        assert_ne!(
            public.schema_hash_prefixed(),
            scoped.schema_hash_prefixed(),
            "schema hash must bind governance fields, not only input/output JSON"
        );
        assert_ne!(
            public.descriptor_hash_prefixed(),
            scoped.descriptor_hash_prefixed(),
            "descriptor revision must change when governance changes"
        );

        let base = public
            .clone()
            .with_description("canonical description")
            .with_source("kernel:canonical")
            .with_output_schema(serde_json::json!({"type":"object"}));
        let digest = base.descriptor_hash_bytes();
        for changed in [
            base.clone().with_description("changed description"),
            base.clone().with_source("changed:source"),
            base.clone().with_metadata_entry("provider", "changed"),
            base.clone()
                .with_output_schema(serde_json::json!({"type":"array"})),
            base.clone().with_hints(AbilityHints {
                idempotent: true,
                ..AbilityHints::default()
            }),
        ] {
            assert_ne!(changed.descriptor_hash_bytes(), digest);
        }

        let changed_action = AbilityDescriptor::new(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Public,
            AdmissionAction::Read,
        )
        .unwrap()
        .with_input_schema(serde_json::json!({"type":"object"}))
        .with_description("canonical description")
        .with_source("kernel:canonical")
        .with_output_schema(serde_json::json!({"type":"object"}));
        assert_ne!(changed_action.descriptor_hash_bytes(), digest);
    }

    #[test]
    fn descriptor_wire_rejects_invalid_version() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let mut json: serde_json::Value = serde_json::to_value(&d).unwrap();
        json["version"] = serde_json::json!("not-semver");
        let err = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();
        assert!(
            err.to_string().contains("invalid descriptor version"),
            "{err}"
        );
    }

    #[test]
    fn descriptor_wire_rejects_missing_version() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let mut json: serde_json::Value = serde_json::to_value(&d).unwrap();
        json.as_object_mut()
            .expect("descriptor JSON object")
            .remove("version");
        let err = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();
        assert!(
            err.to_string().contains("missing field `version`"),
            "wire descriptor must not default a missing version: {err}"
        );
    }

    #[test]
    fn descriptor_wire_rejects_blank_version() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let mut json: serde_json::Value = serde_json::to_value(&d).unwrap();
        json["version"] = serde_json::json!(" ");
        let err = serde_json::from_value::<AbilityDescriptor>(json).unwrap_err();
        assert!(
            err.to_string().contains("invalid descriptor version")
                && err
                    .to_string()
                    .contains("ability descriptor version must be non-empty"),
            "wire descriptor must not default a blank version: {err}"
        );
    }

    #[test]
    fn with_version_validates_the_governed_interface_version() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        );
        let versioned = d.clone().with_version("2.0.0").unwrap();
        assert_eq!(versioned.version, "2.0.0");
        assert_eq!(
            d.with_version("not-semver").unwrap_err(),
            DescriptorError::InvalidVersion {
                version: "not-semver".to_string()
            }
        );
    }

    #[test]
    fn wire_separates_call_mode_from_receipt_semantics() {
        let d = must(
            "chat",
            "easynet:///r/acme/agent/alice.claude",
            Visibility::Scoped,
        )
        .with_call_mode(CallMode::Stream);
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["call_mode"], serde_json::json!("stream"));
        assert_eq!(json["receipt_semantics"]["kind"], "operational");
        assert!(json.get("class").is_none());
    }

    #[test]
    fn bidi_descriptor_hash_uses_bidi_call_mode() {
        let owner = "easynet:///r/acme/agent/alice.claude";
        let stream = must("chat", owner, Visibility::Scoped).with_call_mode(CallMode::Stream);
        let bidi = must("chat", owner, Visibility::Scoped).with_call_mode(CallMode::Bidi);

        assert_eq!(bidi.call_mode(), CallMode::Bidi);
        assert_eq!(serde_json::to_value(&bidi).unwrap()["call_mode"], "bidi");
        assert_ne!(
            stream.descriptor_hash_prefixed(),
            bidi.descriptor_hash_prefixed(),
            "bidi descriptors must hash with Axon CallMode::Bidi, not Stream"
        );
    }

    #[test]
    fn with_call_mode_derives_wire_transport_hints() {
        let owner = "easynet:///r/acme/device/dev-1";
        let descriptor = must("agent.list", owner, Visibility::Scoped)
            .with_hints(AbilityHints {
                read_only: true,
                ..Default::default()
            })
            .with_call_mode(CallMode::Bidi);
        assert_eq!(descriptor.call_mode(), CallMode::Bidi);
        assert!(descriptor.hints.read_only);
        let wire = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(wire["hints"]["streaming_only"], serde_json::json!(false));
        assert_eq!(wire["hints"]["bidi_only"], serde_json::json!(true));
    }

    #[test]
    fn visibility_serde_uses_uppercase_form() {
        let d = must("a.b", DEVICE_OWNER, Visibility::Public);
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
    fn scope_rule_only_matching_serializes_with_ura_list() {
        let rule = ScopeRule::OnlyMatching(vec!["a".into(), "b".into()]);
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["kind"], "only_matching");
        assert_eq!(json["uras"], serde_json::json!(["a", "b"]));
        let back: ScopeRule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }
}
