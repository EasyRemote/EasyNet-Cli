// EasyNet CLI — Local Agents Registry (RFC-001 §1.4 [P2])
// =========================================================
//
// File: src/daemon/persistence/local_agents.rs
//
// Owns ~/.easynet/local-agents.json — the persistent map from
// (profile, name) → canonical Agent URA for every Agent this
// daemon hosts. Mode 0600 because URAs are bound to keys whose
// rotation requires hub coordination.
//
// Why this exists
// ---------------
// Per RFC §1.4 the device-profile MUST mint canonical URAs for each
// hosted Agent (MCP adapter and LLM-profile-per-sub-agent) on
// first boot, then reuse them across daemon restarts. Without
// persistence the device would re-mint URAs on every restart and
// the hub would accumulate dead-but-present entries — eventually
// the realm directory would fill with stale Agents whose hosting
// daemon disappeared three reboots ago.
//
// Schema
// ------
// {
//   "host_device_ura": "easynet:///r/<realm>/device/<id>",
//   "hosted_agents": [
//     {
//       "profile":            "mcp" | "llm",
//       "name":               "default" | "claude" | "codex" | …,
//       "agent_ura":          "easynet:///r/<realm>/agent/<id>",
//       "signing_authority":  "hosted_by:<host_device_ura>",
//       "first_seen_at":      "<rfc3339>"
//     },
//     …
//   ]
// }
//
// What this file is NOT
// ---------------------
// - Not a credentials store. The host device's keypair lives in
//   `private.pem` (per §3 Step A). This file records URAs only.
// - Not a hub directory mirror. The cached RealmDirectory replicas
//   live in axon-runtime memory, populated from
//   `federation.heartbeat` deltas.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;

use serde::{de, Deserialize, Deserializer, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

pub(crate) const FILE_NAME: &str = "local-agents.json";

/// On-disk shape of `~/.easynet/local-agents.json`. Field names
/// must remain stable because this file is hosted-agent identity
/// authority. Missing files represent first boot; existing files
/// must carry the complete canonical schema.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct LocalAgentsFile {
    /// The host Device's canonical URA. Empty until the first successful
    /// `federation.join`. Populated from the join receipt body.
    pub host_device_ura: String,
    /// Hosted Agents (MCP adapter / LLM-per-sub-agent).
    /// Order is insertion order; readers MUST NOT rely on order.
    pub hosted_agents: Vec<HostedAgentEntry>,
}

impl<'de> Deserialize<'de> for LocalAgentsFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            host_device_ura: Option<String>,
            #[serde(default, rename = "host_device_agent_ura")]
            retired_host_device_ura: Option<String>,
            hosted_agents: Vec<HostedAgentEntry>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let host_device_ura = match (wire.host_device_ura, wire.retired_host_device_ura) {
            (Some(current), None) => current,
            (None, Some(retired)) => retired,
            (Some(current), Some(retired)) if current == retired => current,
            (Some(current), Some(retired)) => {
                return Err(de::Error::custom(format!(
                    "local-agents host_device_ura {current:?} conflicts with retired host_device_agent_ura {retired:?}"
                )));
            }
            (None, None) => return Err(de::Error::missing_field("host_device_ura")),
        };
        validate_host_device_ura_field(&host_device_ura).map_err(de::Error::custom)?;
        Ok(Self {
            host_device_ura,
            hosted_agents: wire.hosted_agents,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HostedAgentEntry {
    /// One of `mcp`, `llm`. Free-form to allow future hosted-Agent profiles
    /// without a schema migration.
    pub profile: String,
    /// Per-profile name. `"default"` for singleton profiles
    /// (`mcp`); the sub-agent name for `llm`
    /// (e.g. `"claude"`, `"codex"`).
    pub name: String,
    /// Canonical URA assigned by `federation.advertise_agent` (or
    /// minted locally for hosted Agents in §1.3 Model B).
    pub agent_ura: String,
    /// `"hosted_by:<host_device_ura>"` per §1.3.
    pub signing_authority: String,
    /// RFC 3339 timestamp; useful for operator triage when a stale
    /// entry's profile no longer exists in config.
    pub first_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalHostedAgentIdentityAggregate {
    host_device_ura: String,
    host_realm: String,
    host_device_id: String,
    hosted_agents: Vec<ValidatedHostedAgentIdentity>,
}

impl LocalHostedAgentIdentityAggregate {
    pub(crate) fn validate(file: &LocalAgentsFile) -> anyhow::Result<Self> {
        let host_device_ura = file.host_device_ura.trim();
        if file.host_device_ura != host_device_ura {
            anyhow::bail!("local hosted-agent host_device_ura must be trimmed");
        }
        if host_device_ura.is_empty() {
            if file.hosted_agents.is_empty() {
                return Ok(Self {
                    host_device_ura: String::new(),
                    host_realm: String::new(),
                    host_device_id: String::new(),
                    hosted_agents: Vec::new(),
                });
            }
            anyhow::bail!("local hosted-agent identities require host_device_ura");
        }
        let host = crate::core::ura::parse_ura(host_device_ura)
            .map_err(|error| anyhow::anyhow!("host_device_ura must be canonical: {error}"))?;
        if host.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!("host_device_ura must be a Device URA");
        }
        let host_device_id = host
            .device_id()
            .filter(|device_id| !device_id.is_empty())
            .ok_or_else(|| anyhow::anyhow!("host_device_ura must include a device id"))?
            .to_string();
        let mut seen_profile_names = std::collections::BTreeSet::new();
        let mut seen_agent_uras = std::collections::BTreeSet::new();
        let mut seen_agent_ids = std::collections::BTreeSet::new();
        let mut hosted_agents = Vec::with_capacity(file.hosted_agents.len());
        for entry in &file.hosted_agents {
            let validated =
                ValidatedHostedAgentIdentity::validate(entry, &host.realm, host_device_ura)?;
            if !seen_profile_names.insert((validated.profile.clone(), validated.name.clone())) {
                anyhow::bail!(
                    "duplicate hosted-agent profile/name {}/{}",
                    validated.profile,
                    validated.name
                );
            }
            if !seen_agent_uras.insert(validated.agent_ura.clone()) {
                anyhow::bail!("duplicate hosted-agent URA {}", validated.agent_ura);
            }
            if !seen_agent_ids.insert((validated.user_id.clone(), validated.agent_id.clone())) {
                anyhow::bail!(
                    "duplicate hosted-agent identity {}.{}",
                    validated.user_id,
                    validated.agent_id
                );
            }
            hosted_agents.push(validated);
        }
        Ok(Self {
            host_device_ura: host_device_ura.to_string(),
            host_realm: host.realm,
            host_device_id,
            hosted_agents,
        })
    }

    pub(crate) fn hosted_agents(&self) -> &[ValidatedHostedAgentIdentity] {
        &self.hosted_agents
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedHostedAgentIdentity {
    pub(crate) profile: String,
    pub(crate) name: String,
    pub(crate) agent_ura: String,
    pub(crate) signing_authority: String,
    pub(crate) first_seen_at: String,
    pub(crate) realm: String,
    pub(crate) user_id: String,
    pub(crate) agent_id: String,
}

impl ValidatedHostedAgentIdentity {
    fn validate(
        entry: &HostedAgentEntry,
        host_realm: &str,
        host_device_ura: &str,
    ) -> anyhow::Result<Self> {
        let profile = validated_trimmed_nonempty(&entry.profile, "hosted-agent profile")?;
        let name = validated_trimmed_nonempty(&entry.name, "hosted-agent name")?;
        let agent_ura = validated_trimmed_nonempty(&entry.agent_ura, "hosted-agent agent_ura")?;
        let signing_authority =
            validated_trimmed_nonempty(&entry.signing_authority, "hosted-agent signing_authority")?;
        let first_seen_at =
            validated_trimmed_nonempty(&entry.first_seen_at, "hosted-agent first_seen_at")?;
        let expected_signing_authority = format!("hosted_by:{host_device_ura}");
        if signing_authority != expected_signing_authority {
            anyhow::bail!(
                "hosted-agent signing_authority must be {expected_signing_authority:?}, got {signing_authority:?}"
            );
        }
        chrono::DateTime::parse_from_rfc3339(first_seen_at)
            .map_err(|_| anyhow::anyhow!("hosted-agent first_seen_at must be RFC3339"))?;
        let parsed = crate::core::ura::parse_ura(agent_ura).map_err(|error| {
            anyhow::anyhow!("hosted-agent agent_ura must be canonical: {error}")
        })?;
        if parsed.kind != crate::core::ura::URAKind::Agent {
            anyhow::bail!("hosted-agent agent_ura must be an Agent URA");
        }
        if parsed.realm != host_realm {
            anyhow::bail!(
                "hosted-agent agent_ura realm {} does not match host realm {}",
                parsed.realm,
                host_realm
            );
        }
        let (user_id, agent_id) = parsed.agent_ids().ok_or_else(|| {
            anyhow::anyhow!("hosted-agent agent_ura must include user and agent ids")
        })?;
        if user_id.is_empty() || agent_id.is_empty() {
            anyhow::bail!("hosted-agent agent_ura must include non-empty user and agent ids");
        }
        let realm = parsed.realm.clone();
        let user_id = user_id.to_string();
        let agent_id = agent_id.to_string();
        Ok(Self {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_ura: agent_ura.to_string(),
            signing_authority: signing_authority.to_string(),
            first_seen_at: first_seen_at.to_string(),
            realm,
            user_id,
            agent_id,
        })
    }
}

fn validated_trimmed_nonempty<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    if value != trimmed {
        anyhow::bail!("{field} must be trimmed");
    }
    Ok(trimmed)
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalAgentsLoadState {
    Loaded(LocalAgentsFile),
    Missing { path: PathBuf },
}

/// Resolve the on-disk path. Public so an integration test can
/// override `state_dir` via `XDG_CONFIG_HOME` without re-deriving
/// the layout.
pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

/// Read the hosted-agent identity projection while preserving storage state.
///
/// Missing storage is not an error at this layer and is not projected into an
/// empty registry here. Callers decide whether missing state is a first-boot
/// identity projection.
pub fn load_with_state() -> anyhow::Result<LocalAgentsLoadState> {
    let p = path();
    match fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(LocalAgentsLoadState::Loaded)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(LocalAgentsLoadState::Missing { path: p })
        }
        Err(error) => Err(anyhow::anyhow!("read {}: {error}", p.display())),
    }
}

/// Stable read projection for callers that need first-boot empty identity
/// state. Production lifecycle paths use this named helper instead of hiding
/// the policy inside the storage reader.
pub fn load_for_fresh_host_projection() -> anyhow::Result<LocalAgentsFile> {
    match load_with_state()? {
        LocalAgentsLoadState::Loaded(file) => Ok(file),
        LocalAgentsLoadState::Missing { .. } => Ok(LocalAgentsFile::default()),
    }
}

/// Public read projection preserving the existing API shape.
#[allow(dead_code)]
pub fn load() -> anyhow::Result<LocalAgentsFile> {
    load_for_fresh_host_projection()
}

/// Atomically write the file with mode 0600.
pub fn save(file: &LocalAgentsFile) -> anyhow::Result<()> {
    validate_host_device_ura_field(&file.host_device_ura)
        .map_err(|error| anyhow::anyhow!("invalid local-agents host_device_ura: {error}"))?;
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
        .map_err(Into::into)
}

/// Look up a hosted Agent's URA by `(profile, name)`. Returns the
/// URA if present, `None` otherwise. Used by the publish layer to
/// decide whether to mint a fresh URA or reuse an existing one.
pub fn lookup_hosted_ura(file: &LocalAgentsFile, profile: &str, name: &str) -> Option<String> {
    file.hosted_agents
        .iter()
        .find(|e| e.profile == profile && e.name == name)
        .map(|e| e.agent_ura.clone())
}

/// Insert or update a hosted Agent entry. Returns `true` when an
/// existing entry was replaced (signals the caller to log it for
/// operator visibility); `false` when the entry is new.
pub fn upsert_hosted_agent(
    file: &mut LocalAgentsFile,
    profile: &str,
    name: &str,
    agent_ura: &str,
) -> bool {
    let signing_authority = if file.host_device_ura.is_empty() {
        // Pre-join state: keep the field shape but flag the host as
        // unknown. The first persisted post-join save replaces this.
        "hosted_by:<unset>".to_string()
    } else {
        format!("hosted_by:{}", file.host_device_ura)
    };
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(entry) = file
        .hosted_agents
        .iter_mut()
        .find(|e| e.profile == profile && e.name == name)
    {
        entry.agent_ura = agent_ura.to_string();
        entry.signing_authority = signing_authority;
        return true;
    }
    file.hosted_agents.push(HostedAgentEntry {
        profile: profile.to_string(),
        name: name.to_string(),
        agent_ura: agent_ura.to_string(),
        signing_authority,
        first_seen_at: now,
    });
    false
}

fn validate_host_device_ura_field(host_device_ura: &str) -> Result<(), String> {
    let trimmed = host_device_ura.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if host_device_ura != trimmed {
        return Err("host_device_ura must be trimmed".to_string());
    }
    let parsed = crate::core::ura::parse_ura(trimmed)
        .map_err(|error| format!("host_device_ura must be a canonical Device URA: {error}"))?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        return Err(format!(
            "host_device_ura must be a Device URA, got {:?}",
            parsed.kind
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_projects_explicit_load_state() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        match load_with_state().expect("load state") {
            LocalAgentsLoadState::Missing { path: missing } => assert_eq!(missing, path()),
            LocalAgentsLoadState::Loaded(file) => {
                panic!("missing local-agents storage must not become loaded default: {file:?}")
            }
        }
    }

    #[test]
    fn first_boot_projection_returns_empty_identity_registry() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let f = load_for_fresh_host_projection().expect("first boot projection");
        assert!(f.host_device_ura.is_empty());
        assert!(f.hosted_agents.is_empty());
    }

    #[test]
    fn existing_file_projects_loaded_state() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut f = LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "mcp", "default", "easynet:///r/acme/agent/mcp");
        save(&f).expect("save local agents");

        match load_with_state().expect("load state") {
            LocalAgentsLoadState::Loaded(loaded) => assert_eq!(loaded, f),
            LocalAgentsLoadState::Missing { path: missing } => {
                panic!(
                    "saved local-agents file must load from {}",
                    missing.display()
                )
            }
        }
    }

    #[test]
    fn existing_malformed_file_fails_closed() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let p = path();
        std::fs::create_dir_all(p.parent().expect("state dir")).expect("create state dir");
        std::fs::write(&p, b"{not-json").expect("write malformed local-agents");

        let err = load_with_state().expect_err("malformed existing file must fail");

        assert!(err.to_string().contains("parse"), "unexpected error: {err}");
    }

    #[test]
    fn upsert_inserts_when_absent_and_returns_false() {
        let mut f = LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        let replaced =
            upsert_hosted_agent(&mut f, "llm", "claude", "easynet:///r/acme/agent/u1.01LLM");
        assert!(!replaced);
        assert_eq!(f.hosted_agents.len(), 1);
        assert_eq!(f.hosted_agents[0].profile, "llm");
        assert_eq!(f.hosted_agents[0].name, "claude");
        assert_eq!(
            f.hosted_agents[0].signing_authority,
            "hosted_by:easynet:///r/acme/device/01DEV"
        );
    }

    #[test]
    fn upsert_replaces_when_present_and_returns_true() {
        let mut f = LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "llm", "claude", "ura-v1");
        let replaced = upsert_hosted_agent(&mut f, "llm", "claude", "ura-v2");
        assert!(replaced);
        assert_eq!(f.hosted_agents.len(), 1);
        assert_eq!(f.hosted_agents[0].agent_ura, "ura-v2");
    }

    #[test]
    fn upsert_pre_join_records_unset_signing_authority() {
        let mut f = LocalAgentsFile::default();
        upsert_hosted_agent(&mut f, "mcp", "default", "ura-mcp");
        assert_eq!(
            f.hosted_agents[0].signing_authority, "hosted_by:<unset>",
            "pre-join entries must be flagged so operators know to re-save after join"
        );
    }

    #[test]
    fn lookup_returns_ura_when_present() {
        let mut f = LocalAgentsFile::default();
        upsert_hosted_agent(&mut f, "mcp", "default", "ura-mcp");
        assert_eq!(
            lookup_hosted_ura(&f, "mcp", "default"),
            Some("ura-mcp".to_string())
        );
        assert_eq!(lookup_hosted_ura(&f, "mcp", "other"), None);
        assert_eq!(lookup_hosted_ura(&f, "llm", "default"), None);
    }

    #[test]
    fn round_trip_serde_preserves_all_fields() {
        let mut f = LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "llm", "claude", "ura-llm");
        upsert_hosted_agent(&mut f, "mcp", "default", "ura-mcp");

        let json = serde_json::to_string_pretty(&f).unwrap();
        let back: LocalAgentsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn serialize_emits_host_device_ura_not_retired_agent_field() {
        let f = LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };

        let json = serde_json::to_string_pretty(&f).unwrap();

        assert!(json.contains("\"host_device_ura\""), "{json}");
        assert!(!json.contains("host_device_agent_ura"), "{json}");
    }

    #[test]
    fn deserialize_migrates_retired_host_device_agent_ura() {
        let json = r#"{
            "host_device_agent_ura": "easynet:///r/acme/device/01DEV",
            "hosted_agents": []
        }"#;

        let file: LocalAgentsFile = serde_json::from_str(json).unwrap();

        assert_eq!(file.host_device_ura, "easynet:///r/acme/device/01DEV");
        assert!(file.hosted_agents.is_empty());
    }

    #[test]
    fn deserialize_rejects_conflicting_host_device_fields() {
        let json = r#"{
            "host_device_ura": "easynet:///r/acme/device/01DEV",
            "host_device_agent_ura": "easynet:///r/acme/device/02DEV",
            "hosted_agents": []
        }"#;

        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("conflicting current and retired host fields must fail closed");

        assert!(err.to_string().contains("conflicts"), "{err}");
    }

    #[test]
    fn deserialize_rejects_host_device_ura_that_is_not_device() {
        let json = r#"{
            "host_device_ura": "easynet:///r/acme/agent/u1.host",
            "hosted_agents": []
        }"#;

        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("host_device_ura must be a Device URA");

        assert!(err.to_string().contains("Device URA"), "{err}");
    }

    #[test]
    fn deserialize_rejects_unknown_fields() {
        let json = r#"{
            "host_device_ura": "easynet:///r/acme/device/01DEV",
            "hosted_agents": [
                {
                    "profile": "llm",
                    "name": "claude",
                    "agent_ura": "ura-1",
                    "signing_authority": "hosted_by:easynet:///r/acme/device/01DEV",
                    "first_seen_at": "2026-04-27T00:00:00Z",
                    "future_field": "ignored"
                }
            ],
            "future_top_level_field": 42
        }"#;
        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("unknown local-agents fields must fail closed");
        assert!(
            err.to_string().contains("unknown field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deserialize_rejects_missing_host_device_ura() {
        let json = r#"{"hosted_agents": []}"#;
        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("missing host_device_ura must fail closed");
        assert!(
            err.to_string().contains("missing field `host_device_ura`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deserialize_rejects_missing_hosted_agents() {
        let json = r#"{"host_device_ura": ""}"#;
        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("missing hosted_agents must fail closed");
        assert!(
            err.to_string().contains("missing field `hosted_agents`"),
            "unexpected error: {err}"
        );
    }

    fn valid_hosted_agent_file() -> LocalAgentsFile {
        LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".to_string(),
                name: "claude".to_string(),
                agent_ura: "easynet:///r/acme/agent/u1.claude".to_string(),
                signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
                first_seen_at: "2026-07-16T00:00:00Z".to_string(),
            }],
        }
    }

    #[test]
    fn hosted_identity_aggregate_accepts_canonical_rows() {
        let aggregate =
            LocalHostedAgentIdentityAggregate::validate(&valid_hosted_agent_file()).unwrap();
        assert_eq!(aggregate.host_device_ura, "easynet:///r/acme/device/dev-1");
        assert_eq!(aggregate.host_realm, "acme");
        assert_eq!(aggregate.host_device_id, "dev-1");
        assert_eq!(aggregate.hosted_agents().len(), 1);
        let hosted = &aggregate.hosted_agents()[0];
        assert_eq!(hosted.profile, "llm");
        assert_eq!(hosted.name, "claude");
        assert_eq!(hosted.realm, "acme");
        assert_eq!(hosted.user_id, "u1");
        assert_eq!(hosted.agent_id, "claude");
    }

    #[test]
    fn hosted_identity_aggregate_rejects_entries_without_host_device() {
        let mut file = valid_hosted_agent_file();
        file.host_device_ura.clear();

        let error = LocalHostedAgentIdentityAggregate::validate(&file)
            .expect_err("hosted identities require a host Device");

        assert!(error.to_string().contains("host_device_ura"), "{error}");
    }

    #[test]
    fn hosted_identity_aggregate_rejects_cross_realm_agent() {
        let mut file = valid_hosted_agent_file();
        file.hosted_agents[0].agent_ura = "easynet:///r/other/agent/u1.claude".to_string();

        let error = LocalHostedAgentIdentityAggregate::validate(&file)
            .expect_err("hosted Agent realm must match host realm");

        assert!(
            error.to_string().contains("does not match host realm"),
            "{error}"
        );
    }

    #[test]
    fn hosted_identity_aggregate_rejects_bad_signing_authority() {
        let mut file = valid_hosted_agent_file();
        file.hosted_agents[0].signing_authority =
            "hosted_by:easynet:///r/acme/device/other".to_string();

        let error = LocalHostedAgentIdentityAggregate::validate(&file)
            .expect_err("hosted Agent signing authority must bind host Device");

        assert!(error.to_string().contains("signing_authority"), "{error}");
    }

    #[test]
    fn hosted_identity_aggregate_rejects_duplicate_agent_identity() {
        let mut file = valid_hosted_agent_file();
        let mut duplicate = file.hosted_agents[0].clone();
        duplicate.profile = "mcp".to_string();
        duplicate.name = "default".to_string();
        file.hosted_agents.push(duplicate);

        let error = LocalHostedAgentIdentityAggregate::validate(&file)
            .expect_err("duplicate hosted Agent URA must fail closed");

        assert!(
            error.to_string().contains("duplicate hosted-agent URA"),
            "{error}"
        );
    }

    #[test]
    fn hosted_identity_aggregate_rejects_invalid_timestamp() {
        let mut file = valid_hosted_agent_file();
        file.hosted_agents[0].first_seen_at = "not-rfc3339".to_string();

        let error = LocalHostedAgentIdentityAggregate::validate(&file)
            .expect_err("hosted Agent timestamps must be RFC3339");

        assert!(error.to_string().contains("RFC3339"), "{error}");
    }
}
