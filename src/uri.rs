// EasyNet-Cli — canonical URA builders + parser (URI v2)
// ============================================================
//
// File: src/uri.rs
//
// Single source of truth for v2 URI strings on the Rust side. URI
// v2 contract: always 3 segments after `r/`:
//
//   r/<realm>/<kind>/<tail>
//
// Subject kinds: hub, device, user, agent.
// Ability kind:  ability (tail = <agent_id>.<ability_name> dot-flattened).
// Resource kind: resource (tail = <kind>.<id>).
//
// realm = federation namespace, DNS-aligned. Production = "easynet.run".
//
// Backward-compatibility:
//   Pre-URA-rewrite daemons emit `r/<realm>/agent/<node_id>` for
//   device URIs. Parsers in this module accept the v1 shape under
//   the `Agent` kind so a v2 receiver can still admit a v1 caller
//   during the migration window.

#![allow(dead_code)]

use std::fmt;

/// Production realm — DNS-aligned federation namespace.
pub const REALM_EASYNET: &str = "easynet.run";

pub const HUB_AGENT_ID: &str = "01HUB";

const URI_SCHEME: &str = "easynet:///r/";

/// Subject kind discriminator. Maps the `<kind>` segment of a v2 URI.
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

/// Parsed URI components. `tail` is the raw `<tail>` string;
/// `agent_id` and `name` are populated only for Agent / Ability
/// kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct URAComponents {
    pub realm: String,
    pub kind: URAKind,
    pub tail: String,
    pub agent_id: Option<String>,
    pub name: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    BadScheme,
    MissingRealm,
    MissingKind,
    MissingTail,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::BadScheme => f.write_str("URI must start with easynet:///r/"),
            ParseError::MissingRealm => f.write_str("URI missing <realm> segment"),
            ParseError::MissingKind => f.write_str("URI missing <kind> segment"),
            ParseError::MissingTail => f.write_str("URI missing <tail> segment"),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Builders ────────────────────────────────────────────────────

pub fn hub_uri(realm: &str) -> String {
    format!("{URI_SCHEME}{realm}/hub/{HUB_AGENT_ID}")
}

pub fn device_uri(realm: &str, node_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/device/{node_id}")
}

pub fn user_uri(realm: &str, username: &str) -> String {
    format!("{URI_SCHEME}{realm}/user/{username}")
}

pub fn agent_uri(realm: &str, agent_id: &str) -> String {
    format!("{URI_SCHEME}{realm}/agent/{agent_id}")
}

pub fn ability_uri(realm: &str, agent_id: &str, ability_name: &str) -> String {
    format!("{URI_SCHEME}{realm}/ability/{agent_id}.{ability_name}")
}

pub fn resource_uri(realm: &str, kind: &str, id: &str) -> String {
    format!("{URI_SCHEME}{realm}/resource/{kind}.{id}")
}

// ── Prefix builders (federation.resolve filters) ────────────────

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

// ── Parser ──────────────────────────────────────────────────────

/// Parse a v2 URI into its components. Lenient on unknown kinds
/// (returns `URAKind::Unknown`); strict on segment count.
pub fn parse_ura(uri: &str) -> Result<URAComponents, ParseError> {
    let rest = uri.strip_prefix(URI_SCHEME).ok_or(ParseError::BadScheme)?;
    let (realm, after_realm) = rest.split_once('/').ok_or(ParseError::MissingRealm)?;
    if realm.is_empty() {
        return Err(ParseError::MissingRealm);
    }
    let (kind_str, tail) = after_realm.split_once('/').ok_or(ParseError::MissingKind)?;
    if kind_str.is_empty() {
        return Err(ParseError::MissingKind);
    }
    if tail.is_empty() {
        return Err(ParseError::MissingTail);
    }

    let mut c = URAComponents {
        realm: realm.to_string(),
        kind: URAKind::Unknown,
        tail: tail.to_string(),
        agent_id: None,
        name: None,
        raw: uri.to_string(),
    };
    c.kind = match kind_str {
        "hub" => URAKind::Hub,
        "device" => URAKind::Device,
        "user" => URAKind::User,
        "agent" => {
            c.agent_id = Some(tail.to_string());
            URAKind::Agent
        }
        "ability" => URAKind::Ability,
        "resource" => URAKind::Resource,
        _ => URAKind::Unknown,
    };
    Ok(c)
}

/// Fast classifier — `URAKind::Unknown` on parse failure.
pub fn kind_from_ura(uri: &str) -> URAKind {
    parse_ura(uri).map(|c| c.kind).unwrap_or(URAKind::Unknown)
}

/// Realm extractor — empty string on parse failure.
pub fn realm_from_ura(uri: &str) -> String {
    parse_ura(uri).map(|c| c.realm).unwrap_or_default()
}

/// Extract `<node_id>` from a device URA. Empty on non-device or
/// parse failure. Accepts the v1 `agent/<node>` shape during the
/// migration window — receivers built against v1 daemons need to
/// recognise the device kind even when the on-the-wire URI uses
/// the agent segment.
pub fn node_id_from_device_ura(uri: &str) -> String {
    match parse_ura(uri) {
        Ok(c) if c.kind == URAKind::Device => c.tail,
        // v1 fallback: agent-prefixed device URIs from old daemons.
        Ok(c) if c.kind == URAKind::Agent => c.tail,
        _ => String::new(),
    }
}

/// Extract `<username>` from a user URA. Empty on non-user.
pub fn username_from_user_ura(uri: &str) -> String {
    match parse_ura(uri) {
        Ok(c) if c.kind == URAKind::User => c.tail,
        _ => String::new(),
    }
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
    fn hub_uri_shape() {
        assert_eq!(hub_uri("easynet.run"), "easynet:///r/easynet.run/hub/01HUB");
    }

    #[test]
    fn device_uri_shape() {
        assert_eq!(
            device_uri("easynet.run", "en-30675fdd"),
            "easynet:///r/easynet.run/device/en-30675fdd"
        );
    }

    #[test]
    fn user_uri_shape() {
        assert_eq!(
            user_uri("easynet.run", "alice"),
            "easynet:///r/easynet.run/user/alice"
        );
    }

    #[test]
    fn agent_uri_shape() {
        assert_eq!(
            agent_uri("easynet.run", "alice.claude"),
            "easynet:///r/easynet.run/agent/alice.claude"
        );
    }

    #[test]
    fn ability_uri_shape() {
        assert_eq!(
            ability_uri("easynet.run", "alice.claude", "skill.alive-video"),
            "easynet:///r/easynet.run/ability/alice.claude.skill.alive-video"
        );
    }

    #[test]
    fn resource_uri_shape() {
        assert_eq!(
            resource_uri("easynet.run", "paper", "01HZX"),
            "easynet:///r/easynet.run/resource/paper.01HZX"
        );
    }

    #[test]
    fn prefix_helpers_have_trailing_slash() {
        assert!(realm_device_prefix("acme").ends_with('/'));
        assert!(realm_agent_prefix("acme").ends_with('/'));
        assert!(realm_ability_prefix("acme").ends_with('/'));
        assert!(realm_resource_prefix("acme").ends_with('/'));
    }

    #[test]
    fn parse_all_kinds() {
        let cases = vec![
            ("easynet:///r/easynet.run/hub/01HUB", URAKind::Hub),
            ("easynet:///r/easynet.run/device/en-30675fdd", URAKind::Device),
            ("easynet:///r/easynet.run/user/alice", URAKind::User),
            ("easynet:///r/easynet.run/agent/alice.claude", URAKind::Agent),
            (
                "easynet:///r/easynet.run/ability/alice.claude.skill.alive-video",
                URAKind::Ability,
            ),
            ("easynet:///r/easynet.run/resource/paper.01HZX", URAKind::Resource),
            ("easynet:///r/easynet.run/unknown/x", URAKind::Unknown),
        ];
        for (uri, want_kind) in cases {
            let got = parse_ura(uri).unwrap_or_else(|e| panic!("parse {uri:?}: {e}"));
            assert_eq!(got.kind, want_kind, "kind for {uri:?}");
            assert_eq!(got.raw, uri);
        }
    }

    #[test]
    fn parse_failures() {
        assert!(parse_ura("").is_err());
        assert!(parse_ura("http://example.com").is_err());
        assert!(parse_ura("easynet:///r/easynet.run/agent/").is_err());
        assert!(parse_ura("easynet:///r/easynet.run").is_err());
    }

    #[test]
    fn node_id_from_device_v2() {
        assert_eq!(
            node_id_from_device_ura("easynet:///r/easynet.run/device/en-A"),
            "en-A"
        );
    }

    #[test]
    fn node_id_from_device_v1_fallback() {
        // v1 daemon-emitted shape still extractable.
        assert_eq!(
            node_id_from_device_ura("easynet:///r/easynet.run/agent/en-A"),
            "en-A"
        );
    }

    #[test]
    fn node_id_from_non_device_empty() {
        assert_eq!(
            node_id_from_device_ura("easynet:///r/easynet.run/user/alice"),
            ""
        );
    }
}
