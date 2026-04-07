// EasyNet CLI — Agent Identity Layer (L2)
// =========================================
//
// File: src/shared/agent_id.rs
// Description: Logical agent identity types. This is the L2 identity
//              layer specified in `docs/AGENT_IDENTITY.md`. It is the
//              type-safe replacement for the previous stringly-typed
//              `target_node_id: String` field.
//
// What this file contains:
//   - `AgentId`        — full identity (`<tenant>/<name>`).
//   - `AbilityName`    — typed method name. NOT a peer of `AgentId`.
//                        See AGENT_IDENTITY.md §10 before doing anything
//                        with this type.
//
// What this file is NOT:
//   - Not a URI parser. URI shapes (`easynet://...`) belong to URA L3,
//     a separate (future) layer. See `../URA/README.md`.
//   - Not a registry directory. Resolution lives in
//     `src/shared/agents.rs`.
//   - Not a routing decision. The dispatcher in
//     `src/eal/interpreter.rs` matches on `IrTarget`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The default tenant for shorthand `AgentId` values.
///
/// `"claude"` parses as `AgentId { tenant: "default", name: "claude" }`.
/// This matches `cli/agent.rs::run_send` and `cli/groups/mission.rs`'s
/// fall-through behaviour, and matches the existing `RuntimeState`
/// default tenant in `src/shared/config.rs`.
pub const DEFAULT_TENANT: &str = "default";

/// Maximum length of a single segment (`tenant` or `name`) in characters.
/// Chosen to match DNS-style limits — long enough for any reasonable
/// identifier, short enough that storage and display assumptions hold.
const MAX_SEGMENT_LEN: usize = 63;

// ─── AgentId ─────────────────────────────────────────────────────────────────

/// Logical agent identity. The L2 identity layer specified in
/// `docs/AGENT_IDENTITY.md`.
///
/// Surface forms (parsed by `AgentId::parse` / `FromStr`):
///
/// - Shorthand: `"claude"` → `AgentId { tenant: "default", name: "claude" }`
/// - Full form: `"silan/claude"` → `AgentId { tenant: "silan", name: "claude" }`
///
/// Both segments must match `[a-z0-9_-]+` (ASCII lowercase, alnum, `_`,
/// `-`), each non-empty and ≤63 characters. There is no Unicode, no
/// case-folding, no implicit normalization. Wrong input → hard error.
/// See `docs/AGENT_IDENTITY.md` §3 for the full rejection table.
///
/// `Display` always emits the **full form** (`"<tenant>/<name>"`),
/// never the shorthand. Storage and runtime see the same canonical form.
///
/// Equality is on the resolved `(tenant, name)` pair: shorthand and
/// full form parse to equal values.
///
/// ```ignore
/// assert_eq!(AgentId::parse("claude")?, AgentId::parse("default/claude")?);
/// ```
///
/// **Do not extend this struct.** It contains exactly the two fields
/// shown. Adding `node_id`, `endpoint`, `public_key`, etc. is forbidden
/// by `docs/AGENT_IDENTITY.md` §2 Constraint 2 ("identity, not locator").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId {
    pub tenant: String,
    pub name: String,
}

impl AgentId {
    /// Parse a surface form into an `AgentId`. Accepts shorthand
    /// (`"claude"`) or full form (`"silan/claude"`).
    ///
    /// Hard-rejects:
    /// - empty input
    /// - empty segments (`/`, `claude/`, `/claude`)
    /// - more than two segments (`a/b/c` — multi-level reserved for v2)
    /// - instance ids (`claude#42` — reserved for v2 per ontology §6.4)
    /// - URI-shaped strings (`easynet://...` — wrong layer)
    /// - non-ASCII characters
    /// - uppercase letters
    /// - characters outside `[a-z0-9_-]`
    /// - any segment longer than 63 characters
    pub fn parse(s: &str) -> Result<Self, AgentIdError> {
        // Reject reserved-for-v2 forms with specific error messages so
        // future v2 enablement only has to remove the explicit rejection.
        if s.contains('#') {
            return Err(AgentIdError::ReservedV2 {
                feature: "instance id (`#<id>`)",
            });
        }
        if s.contains("://") {
            return Err(AgentIdError::WrongLayer);
        }
        if s.is_empty() {
            return Err(AgentIdError::Empty);
        }

        // Split into at most 2 segments. More = multi-level namespace,
        // reserved for v2.
        let segments: Vec<&str> = s.split('/').collect();
        let (tenant, name) = match segments.as_slice() {
            [name] => (DEFAULT_TENANT.to_string(), (*name).to_string()),
            [t, n] => ((*t).to_string(), (*n).to_string()),
            _ => {
                return Err(AgentIdError::ReservedV2 {
                    feature: "multi-level namespace (`a/b/c`)",
                });
            }
        };

        validate_segment(&tenant, "tenant")?;
        validate_segment(&name, "name")?;

        Ok(Self { tenant, name })
    }

    /// Construct an `AgentId` from already-validated parts. Useful when
    /// the caller knows the parts came from a trusted source. Both
    /// segments are still validated — there is no unchecked path.
    /// Used by tests today; production code constructs `AgentId` via
    /// `parse` from EAL surface forms.
    #[allow(dead_code)]
    pub fn new(tenant: impl Into<String>, name: impl Into<String>) -> Result<Self, AgentIdError> {
        let tenant = tenant.into();
        let name = name.into();
        validate_segment(&tenant, "tenant")?;
        validate_segment(&name, "name")?;
        Ok(Self { tenant, name })
    }
}

impl fmt::Display for AgentId {
    /// Always emits the full form `<tenant>/<name>`. The shorthand is a
    /// parse-time convenience only — Display is canonical.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.tenant, self.name)
    }
}

impl FromStr for AgentId {
    type Err = AgentIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// ─── AbilityName ─────────────────────────────────────────────────────────────

/// A method name on an agent. **NOT a peer of `AgentId`. NOT an
/// identity type.**
///
/// Before doing anything with this type, read
/// `docs/AGENT_IDENTITY.md` §10 ("Why `AbilityName` is not a peer of
/// `AgentId`"). The asymmetry between this type and `AgentId` is
/// intentional and load-bearing.
///
/// Specifically:
///
/// - `AbilityName` has no `tenant`. Method names live in the namespace
///   of their owning agent, not in a global ability namespace.
/// - `AbilityName` has no canonical equality across owners. Two
///   `AbilityName("chat")` values on different agents are different
///   methods that share a name — they are not the same ability.
/// - `AbilityName` has no `Display` form richer than the underlying
///   string. There is nothing to canonicalize.
/// - `AbilityName` is not a registry key. The agent registry keys on
///   `AgentId`, not on ability.
///
/// If you find yourself wanting to write
/// `struct AbilityRef { agent: AgentId, name: AbilityName }` and use
/// it as a routing target, **stop and read AGENT_IDENTITY.md §10
/// prohibition 3.** That design is explicitly forbidden because it
/// turns the system into a service-registry model.
///
/// `AbilityName` validates its input on construction:
/// `[a-z0-9_-]+`, non-empty, ≤63 characters. That is the entire
/// contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AbilityName(String);

impl AbilityName {
    /// Construct an `AbilityName` from a string. Validates that the
    /// input is non-empty, ASCII lowercase, and matches `[a-z0-9_.-]+`
    /// (≤63 chars).
    ///
    /// Note: ability names allow `.` because the existing EAL convention
    /// uses dotted namespacing (`photo.capture`, `health.check`,
    /// `slow.op`). This is **not** the same character class as
    /// `AgentId` segments — agent names are stricter (no `.`) because
    /// `.` is the EAL member-call separator (`agent.ability`). In EAL
    /// member-call surface form, the ability name is a single
    /// identifier (no `.`); abilities containing `.` must be invoked
    /// via the traditional form `call "photo.capture" on "node"`.
    pub fn parse(s: &str) -> Result<Self, AgentIdError> {
        validate_ability_segment(s)?;
        Ok(Self(s.to_string()))
    }

    /// Borrow the underlying string. There is no `to_string` method
    /// distinct from `Display`, no canonical form distinct from the
    /// stored bytes, and no comparison form. The underlying string
    /// is the entire value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AbilityName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AbilityName {
    type Err = AgentIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// ─── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentIdError {
    /// Input was empty.
    Empty,
    /// A segment (tenant or name) was empty.
    EmptySegment { which: &'static str },
    /// A segment exceeded `MAX_SEGMENT_LEN` characters.
    SegmentTooLong { which: &'static str, len: usize, max: usize },
    /// A segment contained a character outside `[a-z0-9_-]`.
    InvalidChar { which: &'static str, ch: char },
    /// A segment contained an uppercase ASCII letter. Reported
    /// separately from `InvalidChar` so the error message can suggest
    /// lowercasing instead of generically "invalid character".
    UppercaseRejected { which: &'static str },
    /// Input used a feature reserved for a future version of the
    /// identity model. Distinct error so future code can flip the
    /// rejection without changing the error vocabulary.
    ReservedV2 { feature: &'static str },
    /// Input looked like a URI from L3 (URA), which belongs to a
    /// different addressing layer entirely.
    WrongLayer,
}

impl fmt::Display for AgentIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "agent id is empty"),
            Self::EmptySegment { which } => {
                write!(f, "agent id {which} segment is empty")
            }
            Self::SegmentTooLong { which, len, max } => write!(
                f,
                "agent id {which} segment is {len} chars (max {max})"
            ),
            Self::InvalidChar { which, ch } => write!(
                f,
                "agent id {which} segment contains invalid character {ch:?} \
                 (allowed: [a-z0-9_-])"
            ),
            Self::UppercaseRejected { which } => write!(
                f,
                "agent id {which} segment contains uppercase letters; \
                 use lowercase only (allowed: [a-z0-9_-])"
            ),
            Self::ReservedV2 { feature } => write!(
                f,
                "agent id uses {feature}, which is reserved for a \
                 future version of the identity model and is not \
                 accepted by v1"
            ),
            Self::WrongLayer => write!(
                f,
                "agent id looks like a URI; URI addressing belongs to \
                 the URA L3 layer (../URA/README.md), not to AgentId"
            ),
        }
    }
}

impl std::error::Error for AgentIdError {}

// ─── Validation ──────────────────────────────────────────────────────────────

fn validate_segment(s: &str, which: &'static str) -> Result<(), AgentIdError> {
    if s.is_empty() {
        return Err(AgentIdError::EmptySegment { which });
    }
    if s.chars().count() > MAX_SEGMENT_LEN {
        return Err(AgentIdError::SegmentTooLong {
            which,
            len: s.chars().count(),
            max: MAX_SEGMENT_LEN,
        });
    }
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            return Err(AgentIdError::UppercaseRejected { which });
        }
        let ok = ch.is_ascii_lowercase()
            || ch.is_ascii_digit()
            || ch == '_'
            || ch == '-';
        if !ok {
            return Err(AgentIdError::InvalidChar { which, ch });
        }
    }
    Ok(())
}

/// Ability name validator. Allows everything `validate_segment` allows
/// PLUS `.` (for the existing `category.action` convention used across
/// the EAL examples and tests). The `.` is safe in `AbilityName` because
/// ability names only appear as **values** — never inside the EAL
/// `agent.ability` member-call surface, where the lexer would split on
/// `.`. Member-call abilities must be a single identifier; dotted
/// abilities are reachable only via the traditional `call "..." on "..."`
/// form.
fn validate_ability_segment(s: &str) -> Result<(), AgentIdError> {
    let which = "ability";
    if s.is_empty() {
        return Err(AgentIdError::EmptySegment { which });
    }
    if s.chars().count() > MAX_SEGMENT_LEN {
        return Err(AgentIdError::SegmentTooLong {
            which,
            len: s.chars().count(),
            max: MAX_SEGMENT_LEN,
        });
    }
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            return Err(AgentIdError::UppercaseRejected { which });
        }
        let ok = ch.is_ascii_lowercase()
            || ch.is_ascii_digit()
            || ch == '_'
            || ch == '-'
            || ch == '.';
        if !ok {
            return Err(AgentIdError::InvalidChar { which, ch });
        }
    }
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── AgentId — happy paths ────────────────────────────────────────────

    #[test]
    fn shorthand_parses_to_default_tenant() {
        let id = AgentId::parse("claude").unwrap();
        assert_eq!(id.tenant, "default");
        assert_eq!(id.name, "claude");
    }

    #[test]
    fn full_form_parses_unchanged() {
        let id = AgentId::parse("silan/claude").unwrap();
        assert_eq!(id.tenant, "silan");
        assert_eq!(id.name, "claude");
    }

    #[test]
    fn alphanum_underscore_dash_accepted() {
        let id = AgentId::parse("team-1/code_reviewer-v2").unwrap();
        assert_eq!(id.tenant, "team-1");
        assert_eq!(id.name, "code_reviewer-v2");
    }

    // ── AgentId — equality across forms (load-bearing) ───────────────────

    #[test]
    fn shorthand_equals_full_form_in_default_tenant() {
        let a = AgentId::parse("claude").unwrap();
        let b = AgentId::parse("default/claude").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn equality_implies_hash_equality() {
        // HashMap lookup must succeed regardless of which surface form
        // was used to insert vs lookup.
        let mut m: HashMap<AgentId, &'static str> = HashMap::new();
        m.insert(AgentId::parse("claude").unwrap(), "via shorthand");
        let v = m.get(&AgentId::parse("default/claude").unwrap());
        assert_eq!(v, Some(&"via shorthand"));

        // And the reverse direction.
        let mut m: HashMap<AgentId, &'static str> = HashMap::new();
        m.insert(AgentId::parse("default/claude").unwrap(), "via full");
        let v = m.get(&AgentId::parse("claude").unwrap());
        assert_eq!(v, Some(&"via full"));
    }

    #[test]
    fn different_tenants_are_not_equal() {
        let a = AgentId::parse("silan/claude").unwrap();
        let b = AgentId::parse("acme/claude").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_names_are_not_equal() {
        let a = AgentId::parse("default/claude").unwrap();
        let b = AgentId::parse("default/codex").unwrap();
        assert_ne!(a, b);
    }

    // ── AgentId — Display always emits full form ─────────────────────────

    #[test]
    fn display_emits_full_form_for_shorthand() {
        let id = AgentId::parse("claude").unwrap();
        assert_eq!(format!("{id}"), "default/claude");
    }

    #[test]
    fn display_emits_full_form_for_explicit_tenant() {
        let id = AgentId::parse("silan/claude").unwrap();
        assert_eq!(format!("{id}"), "silan/claude");
    }

    #[test]
    fn display_round_trip() {
        // parse(display(parse(s)?)?)? == parse(s)?
        for input in ["claude", "silan/claude", "team-1/code_reviewer-v2"] {
            let parsed = AgentId::parse(input).unwrap();
            let displayed = format!("{parsed}");
            let reparsed = AgentId::parse(&displayed).unwrap();
            assert_eq!(parsed, reparsed, "round-trip failed for {input:?}");
        }
    }

    // ── AgentId — rejection rules from AGENT_IDENTITY.md §3.2 ────────────

    #[test]
    fn rejects_empty() {
        assert_eq!(AgentId::parse(""), Err(AgentIdError::Empty));
    }

    #[test]
    fn rejects_just_separator() {
        assert!(matches!(
            AgentId::parse("/"),
            Err(AgentIdError::EmptySegment { .. })
        ));
    }

    #[test]
    fn rejects_empty_name() {
        assert!(matches!(
            AgentId::parse("silan/"),
            Err(AgentIdError::EmptySegment { which: "name" })
        ));
    }

    #[test]
    fn rejects_empty_tenant() {
        assert!(matches!(
            AgentId::parse("/claude"),
            Err(AgentIdError::EmptySegment { which: "tenant" })
        ));
    }

    #[test]
    fn rejects_multi_level_with_v2_message() {
        let err = AgentId::parse("a/b/c").unwrap_err();
        assert!(matches!(err, AgentIdError::ReservedV2 { .. }));
        assert!(format!("{err}").contains("reserved for a future version"));
    }

    #[test]
    fn rejects_instance_id_with_v2_message() {
        let err = AgentId::parse("claude#42").unwrap_err();
        assert!(matches!(err, AgentIdError::ReservedV2 { .. }));
        let msg = format!("{err}");
        assert!(msg.contains("instance id"));
        assert!(msg.contains("reserved"));
    }

    #[test]
    fn rejects_full_form_with_instance_id() {
        let err = AgentId::parse("silan/claude#42").unwrap_err();
        assert!(matches!(err, AgentIdError::ReservedV2 { .. }));
    }

    #[test]
    fn rejects_uppercase() {
        let err = AgentId::parse("Claude").unwrap_err();
        assert!(matches!(
            err,
            AgentIdError::UppercaseRejected { which: "name" }
        ));
        assert!(format!("{err}").contains("lowercase"));
    }

    #[test]
    fn rejects_uppercase_in_tenant() {
        let err = AgentId::parse("Silan/claude").unwrap_err();
        assert!(matches!(
            err,
            AgentIdError::UppercaseRejected { which: "tenant" }
        ));
    }

    #[test]
    fn rejects_dot_in_name() {
        // `.` is reserved for EAL member-call syntax (`agent.ability`)
        // and would create grammar ambiguity if allowed in agent names.
        assert!(matches!(
            AgentId::parse("claude.chat"),
            Err(AgentIdError::InvalidChar { ch: '.', .. })
        ));
    }

    #[test]
    fn rejects_unicode() {
        assert!(matches!(
            AgentId::parse("审查员"),
            Err(AgentIdError::InvalidChar { .. })
        ));
    }

    #[test]
    fn rejects_whitespace() {
        assert!(matches!(
            AgentId::parse("claude bot"),
            Err(AgentIdError::InvalidChar { ch: ' ', .. })
        ));
    }

    #[test]
    fn rejects_uri_shaped_input_as_wrong_layer() {
        let err =
            AgentId::parse("easynet://r/agent.claude/abilities/chat").unwrap_err();
        assert!(matches!(err, AgentIdError::WrongLayer));
        assert!(format!("{err}").contains("URA"));
    }

    #[test]
    fn rejects_oversized_segment() {
        let long_name: String = "a".repeat(MAX_SEGMENT_LEN + 1);
        assert!(matches!(
            AgentId::parse(&long_name),
            Err(AgentIdError::SegmentTooLong { which: "name", .. })
        ));
    }

    #[test]
    fn accepts_max_length_segment() {
        let just_under: String = "a".repeat(MAX_SEGMENT_LEN);
        assert!(AgentId::parse(&just_under).is_ok());
    }

    // ── AgentId — `new` constructor validates ────────────────────────────

    #[test]
    fn new_validates_both_segments() {
        assert!(AgentId::new("silan", "claude").is_ok());
        assert!(AgentId::new("Silan", "claude").is_err()); // uppercase
        assert!(AgentId::new("silan", "").is_err()); // empty name
        assert!(AgentId::new("", "claude").is_err()); // empty tenant
    }

    // ── AgentId — FromStr ────────────────────────────────────────────────

    #[test]
    fn from_str_works() {
        let id: AgentId = "silan/claude".parse().unwrap();
        assert_eq!(id, AgentId::new("silan", "claude").unwrap());
    }

    // ── AbilityName ──────────────────────────────────────────────────────

    #[test]
    fn ability_name_accepts_alphanum() {
        let a = AbilityName::parse("chat").unwrap();
        assert_eq!(a.as_str(), "chat");
        assert_eq!(format!("{a}"), "chat");
    }

    #[test]
    fn ability_name_accepts_dash_underscore_digits() {
        assert!(AbilityName::parse("review-pr").is_ok());
        assert!(AbilityName::parse("code_review").is_ok());
        assert!(AbilityName::parse("v2").is_ok());
        assert!(AbilityName::parse("a-b_c-d").is_ok());
    }

    #[test]
    fn ability_name_rejects_empty() {
        assert!(matches!(
            AbilityName::parse(""),
            Err(AgentIdError::EmptySegment { .. })
        ));
    }

    #[test]
    fn ability_name_rejects_uppercase() {
        assert!(matches!(
            AbilityName::parse("Chat"),
            Err(AgentIdError::UppercaseRejected { .. })
        ));
    }

    #[test]
    fn ability_name_allows_dot() {
        // Dotted ability names (`category.action`) are the existing EAL
        // convention. They are safe in `AbilityName` because ability
        // names only appear as values; the EAL member-call surface
        // (`agent.ability(...)`) requires a single identifier as the
        // ability part, so dotted abilities are reachable only via
        // the traditional `call "photo.capture" on "node"` form.
        assert!(AbilityName::parse("photo.capture").is_ok());
        assert!(AbilityName::parse("health.check").is_ok());
        assert!(AbilityName::parse("slow.op").is_ok());
        assert!(AbilityName::parse("a.b.c").is_ok());
    }

    #[test]
    fn ability_name_dot_is_safe_with_member_call() {
        // The asymmetry: AgentId rejects `.` (because tenant/name
        // segments can't contain it), but AbilityName accepts it
        // (because the EAL convention uses dotted namespaces).
        // Verify the asymmetry is real.
        assert!(AgentId::parse("claude.chat").is_err());
        assert!(AbilityName::parse("photo.capture").is_ok());
    }

    #[test]
    fn ability_name_rejects_slash() {
        // `/` is the AgentId tenant separator. Allowing it would let
        // ability names look like agent ids.
        assert!(matches!(
            AbilityName::parse("a/b"),
            Err(AgentIdError::InvalidChar { ch: '/', .. })
        ));
    }

    // ── AbilityName — non-peer assertions (anti-regression) ──────────────
    //
    // These tests document properties that AbilityName INTENTIONALLY does
    // not have. They exist so the test corpus visibly encodes "this is
    // not an identity type". If a future contributor adds any of these
    // capabilities, they have to delete an explicit test, which forces
    // them to read AGENT_IDENTITY.md §10.

    #[test]
    fn ability_name_has_no_tenant_field() {
        // Compile-time evidence that AbilityName carries no namespace.
        // If you find yourself wanting to add `.tenant()` here, read
        // AGENT_IDENTITY.md §10 first.
        let a = AbilityName::parse("chat").unwrap();
        let _: &str = a.as_str();
        // No `a.tenant()` and no `a.qualified_with(...)`. By design.
    }

    #[test]
    fn two_ability_names_with_same_string_are_equal_but_only_in_isolation() {
        // `AbilityName("chat") == AbilityName("chat")` is true as a
        // value comparison. But this equality has NO meaning across
        // owning agents — see AGENT_IDENTITY.md §10. The test exists to
        // document that the equality is on the underlying string,
        // nothing more.
        let a = AbilityName::parse("chat").unwrap();
        let b = AbilityName::parse("chat").unwrap();
        assert_eq!(a, b);
    }

    // ── Serde round-trip (registry storage will rely on this) ────────────

    #[test]
    fn agent_id_serializes_as_struct() {
        let id = AgentId::new("silan", "claude").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        // We deliberately serialize as a struct, not as a string.
        // Storage uses the canonical form via Display when keying maps,
        // but the value type itself is structural.
        assert!(json.contains("\"tenant\":\"silan\""));
        assert!(json.contains("\"name\":\"claude\""));

        let back: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn ability_name_serializes_transparently_as_string() {
        let a = AbilityName::parse("chat").unwrap();
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, "\"chat\"");

        let back: AbilityName = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
