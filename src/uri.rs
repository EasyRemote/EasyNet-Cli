// EasyNet-Cli — canonical URA builders + parser (RFC-001 v4.1.4)
// =================================================================
//
// File: src/uri.rs
//
// Single source of truth for v4.1.4 URI strings on the Rust side.
// v4.1.4 ontology, ratified by CTO 2026-05-03:
//
//   realm    owns hub
//   user     owns agent
//   device   hosts agent
//   agent    exposes ability
//   resource is user-owned concrete substrate
//
// Six URI shapes
// --------------
//   user      easynet:///r/<realm>/user/<user-uuid>
//   device    easynet:///r/<realm>/device/<device-uuid>
//   agent     easynet:///r/<realm>/agent/<user-uuid>.<agent-id>
//   ability   easynet:///r/<realm>/ability/<user-uuid>.<agent-id>.<ability-id>
//   hub       easynet:///r/<realm>/hub                           (singleton)
//   resource  easynet:///r/<realm>/resource/<user-uuid>/<ns>/<path>
//
// Why these shapes
// ----------------
// agent / ability / resource are user-anchored: ownership belongs to
// the user; the host device is a runtime placement detail (Directory
// metadata), NOT in the URI. When a user migrates between devices,
// the agent/resource URA stays stable.
//
// hub is a realm-singleton with no sub-id (`r/<realm>/hub`). The
// backend Go process IS the hub of its realm — there is no separate
// "backend" identity in v4.1.4. The legacy `01BAK` / `01HUB` tail is
// retired.
//
// device-id is a UUID (no `en-` prefix, no user prefix). The
// device → user mapping lives in `device_pairings.user_id`.
//
// Namespace is the typed access channel into the substrate (fs /
// process / pty / shell / http). The set is closed: `parse_ura`
// rejects unknown namespaces.

#![allow(dead_code)]

use std::fmt;

/// Production realm — DNS-aligned federation namespace.
pub const REALM_EASYNET: &str = "easynet.run";

const URI_SCHEME: &str = "easynet:///r/";

/// Subject kind discriminator. Maps the `<role>` segment of a URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum URAKind {
    Hub,
    Device,
    User,
    Agent,
    Ability,
    Resource,
    Unknown,
}

impl fmt::Display for URAKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            URAKind::Hub => "hub",
            URAKind::Device => "device",
            URAKind::User => "user",
            URAKind::Agent => "agent",
            URAKind::Ability => "ability",
            URAKind::Resource => "resource",
            URAKind::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

/// Resource namespace — closed set; the parser rejects unknown values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceNamespace {
    Fs,
    Process,
    Pty,
    Shell,
    Http,
}

impl ResourceNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceNamespace::Fs => "fs",
            ResourceNamespace::Process => "process",
            ResourceNamespace::Pty => "pty",
            ResourceNamespace::Shell => "shell",
            ResourceNamespace::Http => "http",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "fs" => Some(ResourceNamespace::Fs),
            "process" => Some(ResourceNamespace::Process),
            "pty" => Some(ResourceNamespace::Pty),
            "shell" => Some(ResourceNamespace::Shell),
            "http" => Some(ResourceNamespace::Http),
            _ => None,
        }
    }
}

impl fmt::Display for ResourceNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed URA. Field population by Kind:
///   User     → user_id
///   Device   → device_id
///   Agent    → user_id, agent_id
///   Ability  → user_id, agent_id, ability_id
///   Hub      → (no further fields; realm-singleton)
///   Resource → user_id, namespace, path
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedURA {
    pub realm: String,
    pub kind: URAKind,
    pub user_id: String,
    pub device_id: String,
    pub agent_id: String,
    pub ability_id: String,
    pub namespace: Option<ResourceNamespace>,
    pub path: String,
    pub raw: String,
}

impl Default for URAKind {
    fn default() -> Self {
        URAKind::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    BadScheme,
    EmptyRealm,
    EmptyRole,
    UserMissingTail,
    UserBadShape,
    DeviceMissingTail,
    DeviceBadShape,
    AgentMissingTail,
    AgentBadShape,
    AbilityMissingTail,
    AbilityBadShape,
    HubUnexpectedTail(String),
    ResourceMissingTail,
    ResourceMissingNs,
    ResourceEmptyUser,
    ResourceEmptyNs,
    ResourceUnknownNs(String),
    UnknownRole(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::BadScheme => f.write_str("URI must start with easynet:///r/"),
            ParseError::EmptyRealm => f.write_str("URI missing <realm> segment"),
            ParseError::EmptyRole => f.write_str("URI missing <role> segment"),
            ParseError::UserMissingTail => f.write_str("user URI requires <user-uuid> tail"),
            ParseError::UserBadShape => {
                f.write_str("user-id must be bare (no dots/slashes)")
            }
            ParseError::DeviceMissingTail => {
                f.write_str("device URI requires <device-uuid> tail")
            }
            ParseError::DeviceBadShape => {
                f.write_str("device-id must be bare (no dots/slashes)")
            }
            ParseError::AgentMissingTail => {
                f.write_str("agent URI requires <user>.<agent> tail")
            }
            ParseError::AgentBadShape => {
                f.write_str("agent tail must be <user>.<agent>; agent-id must not contain '.'")
            }
            ParseError::AbilityMissingTail => {
                f.write_str("ability URI requires <user>.<agent>.<ability> tail")
            }
            ParseError::AbilityBadShape => f.write_str(
                "ability tail must be <user>.<agent>.<ability> with three non-empty parts",
            ),
            ParseError::HubUnexpectedTail(t) => {
                write!(f, "hub is realm-singleton; unexpected tail {t:?}")
            }
            ParseError::ResourceMissingTail => {
                f.write_str("resource URI requires <user>/<ns>/<path> tail")
            }
            ParseError::ResourceMissingNs => f.write_str("resource tail missing /<namespace>"),
            ParseError::ResourceEmptyUser => f.write_str("resource user-id empty"),
            ParseError::ResourceEmptyNs => f.write_str("resource namespace empty"),
            ParseError::ResourceUnknownNs(ns) => {
                write!(
                    f,
                    "unknown resource namespace {ns:?} (allowed: fs/process/pty/shell/http)"
                )
            }
            ParseError::UnknownRole(r) => write!(
                f,
                "unknown role {r:?} (allowed: user/device/agent/ability/hub/resource)"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Builders ────────────────────────────────────────────────────

pub fn user_uri(realm: &str, user_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/user/{user_id}")
}

pub fn device_uri(realm: &str, device_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/device/{device_id}")
}

pub fn agent_uri(realm: &str, user_id: &str, agent_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/agent/{user_id}.{agent_id}")
}

pub fn ability_uri(realm: &str, user_id: &str, agent_id: &str, ability_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/ability/{user_id}.{agent_id}.{ability_id}")
}

/// Hub is a realm-singleton: no sub-id, no tail. v4.1.4 retires the
/// `01HUB` / `01BAK` agent-id distinction.
pub fn hub_uri(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/hub")
}

/// Resource is user-anchored. `path` may have a leading slash; it is
/// stripped so `/Users/...` and `Users/...` produce the same URI.
pub fn resource_uri(realm: &str, user_id: &str, ns: ResourceNamespace, path: &str) -> String {
    let clean = path.strip_prefix('/').unwrap_or(path);
    format!("{URI_SCHEME}{realm}/resource/{user_id}/{}/{clean}", ns.as_str())
}

// ── Realm-scoped prefixes (federation.resolve filters) ─────────

pub fn realm_user_prefix(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/user/")
}

pub fn realm_device_prefix(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/device/")
}

pub fn realm_agent_prefix(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/agent/")
}

pub fn realm_ability_prefix(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/ability/")
}

pub fn realm_resource_prefix(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/resource/")
}

// ── Parser ─────────────────────────────────────────────────────

/// Parse a v4.1.4 URI into a [`ParsedURA`]. Strict: each role
/// enforces its own dot-to-thing shape; unknown roles or unknown
/// resource namespaces yield a [`ParseError`].
pub fn parse_ura(uri: &str) -> Result<ParsedURA, ParseError> {
    let rest = uri.strip_prefix(URI_SCHEME).ok_or(ParseError::BadScheme)?;
    let (realm, after_realm) = rest.split_once('/').ok_or(ParseError::EmptyRealm)?;
    if realm.is_empty() {
        return Err(ParseError::EmptyRealm);
    }
    // tail may be absent (hub realm-singleton).
    let (role, tail) = match after_realm.split_once('/') {
        Some((r, t)) => (r, t),
        None => (after_realm, ""),
    };
    if role.is_empty() {
        return Err(ParseError::EmptyRole);
    }

    let mut out = ParsedURA {
        realm: realm.to_string(),
        raw: uri.to_string(),
        ..Default::default()
    };

    match role {
        "user" => {
            if tail.is_empty() {
                return Err(ParseError::UserMissingTail);
            }
            if tail.contains('.') || tail.contains('/') {
                return Err(ParseError::UserBadShape);
            }
            out.kind = URAKind::User;
            out.user_id = tail.to_string();
        }
        "device" => {
            if tail.is_empty() {
                return Err(ParseError::DeviceMissingTail);
            }
            if tail.contains('.') || tail.contains('/') {
                return Err(ParseError::DeviceBadShape);
            }
            out.kind = URAKind::Device;
            out.device_id = tail.to_string();
        }
        "agent" => {
            if tail.is_empty() {
                return Err(ParseError::AgentMissingTail);
            }
            // Exactly one dot, splitting user-id from agent-id.
            // agent-id MUST NOT contain dots — that namespace
            // belongs to ability URIs.
            let (uid, aid) = tail.split_once('.').ok_or(ParseError::AgentBadShape)?;
            if uid.is_empty() || aid.is_empty() {
                return Err(ParseError::AgentBadShape);
            }
            if aid.contains('.') || aid.contains('/') || uid.contains('/') {
                return Err(ParseError::AgentBadShape);
            }
            out.kind = URAKind::Agent;
            out.user_id = uid.to_string();
            out.agent_id = aid.to_string();
        }
        "ability" => {
            if tail.is_empty() {
                return Err(ParseError::AbilityMissingTail);
            }
            // splitn(3, '.') — first two dots are fixed boundaries
            // (user-id, agent-id are single tokens with no internal
            // dots). Everything after the second dot is ability-id
            // verbatim and may itself contain dots ("fs.read").
            let mut it = tail.splitn(3, '.');
            let uid = it.next().unwrap_or("");
            let aid = it.next().unwrap_or("");
            let abid = it.next().unwrap_or("");
            if uid.is_empty() || aid.is_empty() || abid.is_empty() {
                return Err(ParseError::AbilityBadShape);
            }
            if uid.contains('/') || aid.contains('/') || aid.contains('.') {
                return Err(ParseError::AbilityBadShape);
            }
            out.kind = URAKind::Ability;
            out.user_id = uid.to_string();
            out.agent_id = aid.to_string();
            out.ability_id = abid.to_string();
        }
        "hub" => {
            // Realm-singleton: strict reject if a v4.1.3 caller still
            // emits `/hub/01HUB`.
            if !tail.is_empty() {
                return Err(ParseError::HubUnexpectedTail(tail.to_string()));
            }
            out.kind = URAKind::Hub;
        }
        "resource" => {
            if tail.is_empty() {
                return Err(ParseError::ResourceMissingTail);
            }
            let (uid, after_user) =
                tail.split_once('/').ok_or(ParseError::ResourceMissingNs)?;
            if uid.is_empty() {
                return Err(ParseError::ResourceEmptyUser);
            }
            let (ns_str, path) = match after_user.split_once('/') {
                Some((n, p)) => (n, p),
                None => (after_user, ""),
            };
            if ns_str.is_empty() {
                return Err(ParseError::ResourceEmptyNs);
            }
            let ns = ResourceNamespace::from_str(ns_str)
                .ok_or_else(|| ParseError::ResourceUnknownNs(ns_str.to_string()))?;
            out.kind = URAKind::Resource;
            out.user_id = uid.to_string();
            out.namespace = Some(ns);
            out.path = path.to_string();
        }
        other => return Err(ParseError::UnknownRole(other.to_string())),
    }

    Ok(out)
}

/// Fast classifier — `URAKind::Unknown` on parse failure.
pub fn kind_from_ura(uri: &str) -> URAKind {
    parse_ura(uri).map(|c| c.kind).unwrap_or(URAKind::Unknown)
}

/// Realm extractor — empty string on parse failure.
pub fn realm_from_ura(uri: &str) -> String {
    parse_ura(uri).map(|c| c.realm).unwrap_or_default()
}

/// Render the natural display id for a URA. device/user → bare id;
/// agent → "<user>.<agent>"; ability → "<user>.<agent>.<ab>";
/// hub → "hub"; resource → "<user>/<ns>/<path>". Returns the URI
/// itself on parse failure so callers can chain a v1 fallback.
pub fn display_id(uri: &str) -> String {
    match parse_ura(uri) {
        Err(_) => uri.to_string(),
        Ok(c) => match c.kind {
            URAKind::Device => c.device_id,
            URAKind::User => c.user_id,
            URAKind::Agent => format!("{}.{}", c.user_id, c.agent_id),
            URAKind::Ability => format!("{}.{}.{}", c.user_id, c.agent_id, c.ability_id),
            URAKind::Hub => "hub".to_string(),
            URAKind::Resource => format!(
                "{}/{}/{}",
                c.user_id,
                c.namespace.map(|n| n.as_str()).unwrap_or(""),
                c.path
            ),
            URAKind::Unknown => uri.to_string(),
        },
    }
}

/// Normalise a URI used as a `PresenceRegistry` lookup key.
///
/// PresenceRegistry keys on the caller-claimed URI exact-match
/// (see `presence_registry.rs` §1 invariants). After URA v4.1.4
/// the canonical device shape is `easynet:///r/<realm>/device/<uuid>`,
/// but a peer hub running an older build (or a CLI bridge that
/// hand-built a URI) may still emit the v1/v2 `agent/<bare-uuid>`
/// shape — same node, different segment label. Without
/// normalisation that lookup misses and the hub returns
/// `target_offline` even though the device is registered live.
///
/// Rule: when [`parse_ura`] succeeds, the URI is already canonical
/// — return it verbatim. When it fails AND the shape is exactly
/// `easynet:///r/<realm>/agent/<token>` where `<token>` is a single
/// segment with no `.` (i.e. NOT a real `<user>.<agent>` agent
/// URA), rewrite the role segment to `device`. Otherwise return
/// the input unchanged so a malformed URI surfaces a normal lookup
/// miss rather than a silent rewrite.
///
/// This is the v4.1.5 Postel boundary: parsers are strict, but
/// presence/forward-invoke lookup paths accept the one well-known
/// pre-v4.1.4 shape so peer compatibility holds during the rolling
/// upgrade window. New clients should always emit the canonical
/// `device/<uuid>` form.
pub fn canonicalize_presence_key(uri: &str) -> String {
    if parse_ura(uri).is_ok() {
        return uri.to_string();
    }
    let head = "easynet:///r/";
    let Some(rest) = uri.strip_prefix(head) else {
        return uri.to_string();
    };
    let Some((realm, after_realm)) = rest.split_once('/') else {
        return uri.to_string();
    };
    let Some(token) = after_realm.strip_prefix("agent/") else {
        return uri.to_string();
    };
    if token.is_empty() || token.contains('/') || token.contains('.') {
        return uri.to_string();
    }
    format!("{head}{realm}/device/{token}")
}

/// Realm-agnostic v1-shape strip for `easynet:///r/<realm>/agent/<id>`
/// shapes that pre-Phase-2A daemons still emit. Used as a fallback
/// when [`parse_ura`] rejects a URI; returns the input unchanged on
/// any structural mismatch so callers can chain with [`display_id`].
pub fn strip_v1_agent_prefix(uri: &str) -> String {
    let head = "easynet:///r/";
    let Some(rest) = uri.strip_prefix(head) else {
        return uri.to_string();
    };
    let Some((_realm, after_realm)) = rest.split_once('/') else {
        return uri.to_string();
    };
    match after_realm.strip_prefix("agent/") {
        Some(id) => id.to_string(),
        None => uri.to_string(),
    }
}

// Legacy aliases retained briefly for the in-tree sweep window. They
// route through the new helpers so the wire shape stays correct.

/// Deprecated: use [`device_uri`] (alias for v1 `node_id_from_device_ura`).
#[deprecated(note = "use parse_ura(uri).device_id or strip_v1_agent_prefix")]
pub fn node_id_from_device_ura(uri: &str) -> String {
    if let Ok(c) = parse_ura(uri) {
        if c.kind == URAKind::Device {
            return c.device_id;
        }
    }
    strip_v1_agent_prefix(uri)
}

/// Deprecated: use [`parse_ura(uri).user_id`].
#[deprecated(note = "use parse_ura(uri).user_id")]
pub fn username_from_user_ura(uri: &str) -> String {
    parse_ura(uri).map(|c| c.user_id).unwrap_or_default()
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_easynet_constant() {
        assert_eq!(REALM_EASYNET, "easynet.run");
    }

    #[test]
    fn hub_uri_is_realm_singleton() {
        // v4.1.4: no /01HUB tail.
        assert_eq!(hub_uri("easynet.run"), "easynet:///r/easynet.run/hub");
    }

    #[test]
    fn device_uri_shape() {
        let uuid = "4065c47a-ec6f-4330-87a5-0d69787709b8";
        assert_eq!(
            device_uri("localhost", uuid),
            format!("easynet:///r/localhost/device/{uuid}")
        );
    }

    #[test]
    fn user_uri_shape() {
        let uuid = "5ff5ac67-ac43-400a-9f36-4899eddf68ff";
        assert_eq!(
            user_uri("localhost", uuid),
            format!("easynet:///r/localhost/user/{uuid}")
        );
    }

    #[test]
    fn agent_uri_user_anchored() {
        let uuid = "5ff5ac67-ac43-400a-9f36-4899eddf68ff";
        assert_eq!(
            agent_uri("localhost", uuid, "claude"),
            format!("easynet:///r/localhost/agent/{uuid}.claude")
        );
    }

    #[test]
    fn ability_uri_three_dot_tail() {
        let uuid = "5ff5ac67-ac43-400a-9f36-4899eddf68ff";
        assert_eq!(
            ability_uri("localhost", uuid, "claude", "fs.read"),
            format!("easynet:///r/localhost/ability/{uuid}.claude.fs.read")
        );
    }

    #[test]
    fn resource_uri_user_anchored_with_namespace() {
        let uuid = "5ff5ac67-ac43-400a-9f36-4899eddf68ff";
        assert_eq!(
            resource_uri("localhost", uuid, ResourceNamespace::Fs, "tmp/foo.txt"),
            format!("easynet:///r/localhost/resource/{uuid}/fs/tmp/foo.txt")
        );
        // Leading-slash path is normalised.
        assert_eq!(
            resource_uri("localhost", uuid, ResourceNamespace::Fs, "/tmp/foo.txt"),
            format!("easynet:///r/localhost/resource/{uuid}/fs/tmp/foo.txt")
        );
    }

    #[test]
    fn prefix_helpers_have_trailing_slash() {
        assert!(realm_user_prefix("acme").ends_with('/'));
        assert!(realm_device_prefix("acme").ends_with('/'));
        assert!(realm_agent_prefix("acme").ends_with('/'));
        assert!(realm_ability_prefix("acme").ends_with('/'));
        assert!(realm_resource_prefix("acme").ends_with('/'));
    }

    #[test]
    fn parse_user() {
        let p = parse_ura("easynet:///r/localhost/user/alice-uuid").unwrap();
        assert_eq!(p.kind, URAKind::User);
        assert_eq!(p.user_id, "alice-uuid");
    }

    #[test]
    fn parse_device() {
        let p = parse_ura("easynet:///r/localhost/device/dev-uuid").unwrap();
        assert_eq!(p.kind, URAKind::Device);
        assert_eq!(p.device_id, "dev-uuid");
    }

    #[test]
    fn parse_agent() {
        let p = parse_ura("easynet:///r/localhost/agent/u1.claude").unwrap();
        assert_eq!(p.kind, URAKind::Agent);
        assert_eq!(p.user_id, "u1");
        assert_eq!(p.agent_id, "claude");
    }

    #[test]
    fn parse_ability_with_dotted_ability_id() {
        // ability-id may contain dots; user/agent are single tokens.
        let p = parse_ura("easynet:///r/localhost/ability/u1.claude.fs.read").unwrap();
        assert_eq!(p.kind, URAKind::Ability);
        assert_eq!(p.user_id, "u1");
        assert_eq!(p.agent_id, "claude");
        assert_eq!(p.ability_id, "fs.read");
    }

    #[test]
    fn parse_hub_realm_singleton() {
        let p = parse_ura("easynet:///r/localhost/hub").unwrap();
        assert_eq!(p.kind, URAKind::Hub);
    }

    #[test]
    fn parse_hub_rejects_v1_tail() {
        // v4.1.3's /hub/01HUB shape is a strict reject in v4.1.4.
        let err = parse_ura("easynet:///r/localhost/hub/01HUB").unwrap_err();
        matches!(err, ParseError::HubUnexpectedTail(_));
    }

    #[test]
    fn parse_resource() {
        let p = parse_ura("easynet:///r/localhost/resource/u1/fs/tmp/foo.txt").unwrap();
        assert_eq!(p.kind, URAKind::Resource);
        assert_eq!(p.user_id, "u1");
        assert_eq!(p.namespace, Some(ResourceNamespace::Fs));
        assert_eq!(p.path, "tmp/foo.txt");
    }

    #[test]
    fn parse_resource_rejects_unknown_namespace() {
        let err =
            parse_ura("easynet:///r/localhost/resource/u1/notarealns/x").unwrap_err();
        match err {
            ParseError::ResourceUnknownNs(ref ns) => assert_eq!(ns, "notarealns"),
            other => panic!("wanted ResourceUnknownNs, got {other:?}"),
        }
    }

    #[test]
    fn parse_failures() {
        assert!(parse_ura("").is_err());
        assert!(parse_ura("http://example.com").is_err());
        // Old single-tail agent shape now rejected.
        assert!(parse_ura("easynet:///r/easynet.run/agent/dev-A").is_err());
        // Empty agent-id after dot.
        assert!(parse_ura("easynet:///r/easynet.run/agent/u1.").is_err());
        // No realm.
        assert!(parse_ura("easynet:///r//user/u1").is_err());
    }

    #[test]
    fn display_id_per_kind() {
        let uuid = "u1";
        assert_eq!(
            display_id(&user_uri("localhost", uuid)),
            "u1"
        );
        assert_eq!(
            display_id(&device_uri("localhost", "dev-1")),
            "dev-1"
        );
        assert_eq!(
            display_id(&agent_uri("localhost", uuid, "claude")),
            "u1.claude"
        );
        assert_eq!(
            display_id(&ability_uri("localhost", uuid, "claude", "fs.read")),
            "u1.claude.fs.read"
        );
        assert_eq!(display_id(&hub_uri("localhost")), "hub");
        assert_eq!(
            display_id(&resource_uri(
                "localhost",
                uuid,
                ResourceNamespace::Fs,
                "etc/hosts"
            )),
            "u1/fs/etc/hosts"
        );
    }

    #[test]
    fn display_id_v1_fallback_returns_input() {
        let v1 = "easynet:///r/easynet.run/agent/dev-A";
        assert_eq!(display_id(v1), v1);
        // strip_v1_agent_prefix recovers the id.
        assert_eq!(strip_v1_agent_prefix(v1), "dev-A");
    }

    #[test]
    fn canonicalize_presence_key_passes_canonical_device_through() {
        let uuid = "4065c47a-ec6f-4330-87a5-0d69787709b8";
        let canonical = format!("easynet:///r/easynet.run/device/{uuid}");
        assert_eq!(canonicalize_presence_key(&canonical), canonical);
    }

    #[test]
    fn canonicalize_presence_key_rewrites_legacy_agent_bare_id_to_device() {
        // v1/v2 shape: device-as-agent, bare token, no dot.
        // Peer hubs from older builds still emit this; lookup
        // against the v4.1.4 device-shape PresenceRegistry would
        // otherwise miss and return target_offline.
        let uuid = "4065c47a-ec6f-4330-87a5-0d69787709b8";
        let legacy = format!("easynet:///r/easynet.run/agent/{uuid}");
        let canonical = format!("easynet:///r/easynet.run/device/{uuid}");
        assert_eq!(canonicalize_presence_key(&legacy), canonical);
    }

    #[test]
    fn canonicalize_presence_key_passes_canonical_agent_through() {
        // Real agent URA — `<user>.<agent>` shape, two segments.
        // Already canonical; do NOT rewrite to device.
        let canonical = "easynet:///r/easynet.run/agent/alice.claude";
        assert_eq!(canonicalize_presence_key(canonical), canonical);
    }

    #[test]
    fn canonicalize_presence_key_passes_hub_through() {
        let canonical = "easynet:///r/easynet.run/hub";
        assert_eq!(canonicalize_presence_key(canonical), canonical);
    }

    #[test]
    fn canonicalize_presence_key_returns_malformed_input_unchanged() {
        // Bad scheme — leave alone so the lookup miss is the
        // operator-visible error, not a silent rewrite.
        let bad = "https://hub.example/r/foo";
        assert_eq!(canonicalize_presence_key(bad), bad);

        let empty_realm = "easynet:///r//device/x";
        assert_eq!(canonicalize_presence_key(empty_realm), empty_realm);

        let multi_seg = "easynet:///r/realm/agent/a/b/c";
        assert_eq!(canonicalize_presence_key(multi_seg), multi_seg);
    }
}
