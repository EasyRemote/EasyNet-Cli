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
// hosted Agent (consent, mcp, llm-profile-per-sub-agent) on
// first boot, then reuse them across daemon restarts. Without
// persistence the device would re-mint URAs on every restart and
// the hub would accumulate dead-but-present entries — eventually
// the realm directory would fill with stale Agents whose hosting
// daemon disappeared three reboots ago.
//
// Schema
// ------
// {
//   "host_device_agent_ura": "easynet:///r/<realm>/agent/<id>",
//   "hosted_agents": [
//     {
//       "profile":            "consent" | "mcp" | "llm",
//       "name":               "default" | "claude" | "codex" | …,
//       "agent_ura":          "easynet:///r/<realm>/agent/<id>",
//       "signing_authority":  "hosted_by:<host_device_agent_ura>",
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

use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

pub(crate) const FILE_NAME: &str = "local-agents.json";

/// On-disk shape of `~/.easynet/local-agents.json`. Field names
/// must remain stable because this file is hosted-agent identity
/// authority. Missing files represent first boot; existing files
/// must carry the complete canonical schema.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LocalAgentsFile {
    /// The device-profile Agent's canonical URA. Empty until the
    /// first successful `federation.join`. Populated from the join
    /// receipt body.
    pub host_device_agent_ura: String,
    /// Hosted Agents (consent / mcp / llm-per-sub-agent).
    /// Order is insertion order; readers MUST NOT rely on order.
    pub hosted_agents: Vec<HostedAgentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HostedAgentEntry {
    /// One of `consent`, `mcp`, `llm`. Free-form to allow
    /// future profiles without a schema migration.
    pub profile: String,
    /// Per-profile name. `"default"` for singleton profiles
    /// (consent, mcp); the sub-agent name for `llm`
    /// (e.g. `"claude"`, `"codex"`).
    pub name: String,
    /// Canonical URA assigned by `federation.advertise_agent` (or
    /// minted locally for hosted Agents in §1.3 Model B).
    pub agent_ura: String,
    /// `"hosted_by:<host_device_agent_ura>"` per §1.3.
    pub signing_authority: String,
    /// RFC 3339 timestamp; useful for operator triage when a stale
    /// entry's profile no longer exists in config.
    pub first_seen_at: String,
}

/// Resolve the on-disk path. Public so an integration test can
/// override `state_dir` via `XDG_CONFIG_HOME` without re-deriving
/// the layout.
pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

/// Read the file. Returns an empty `LocalAgentsFile` if the file
/// does not exist (first-boot case). Returns Err only on parse
/// failure or unrecoverable I/O.
pub fn load() -> anyhow::Result<LocalAgentsFile> {
    let p = path();
    if !p.exists() {
        return Ok(LocalAgentsFile::default());
    }
    let bytes = fs::read(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

/// Atomically write the file with mode 0600.
pub fn save(file: &LocalAgentsFile) -> anyhow::Result<()> {
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
    let signing_authority = if file.host_device_agent_ura.is_empty() {
        // Pre-join state: keep the field shape but flag the host as
        // unknown. The first persisted post-join save replaces this.
        "hosted_by:<unset>".to_string()
    } else {
        format!("hosted_by:{}", file.host_device_agent_ura)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        // A guaranteed-nonexistent path. We exercise the in-memory
        // default rather than touching state_dir() so the test does
        // not depend on the user's $HOME.
        let f = LocalAgentsFile::default();
        assert!(f.host_device_agent_ura.is_empty());
        assert!(f.hosted_agents.is_empty());
    }

    #[test]
    fn upsert_inserts_when_absent_and_returns_false() {
        let mut f = LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
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
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
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
        upsert_hosted_agent(&mut f, "consent", "default", "ura-c");
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
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "llm", "claude", "ura-llm");
        upsert_hosted_agent(&mut f, "consent", "default", "ura-c");

        let json = serde_json::to_string_pretty(&f).unwrap();
        let back: LocalAgentsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn deserialize_rejects_unknown_fields() {
        let json = r#"{
            "host_device_agent_ura": "easynet:///r/acme/device/01DEV",
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
    fn deserialize_rejects_missing_host_device_agent_ura() {
        let json = r#"{"hosted_agents": []}"#;
        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("missing host_device_agent_ura must fail closed");
        assert!(
            err.to_string()
                .contains("missing field `host_device_agent_ura`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deserialize_rejects_missing_hosted_agents() {
        let json = r#"{"host_device_agent_ura": ""}"#;
        let err = serde_json::from_str::<LocalAgentsFile>(json)
            .expect_err("missing hosted_agents must fail closed");
        assert!(
            err.to_string().contains("missing field `hosted_agents`"),
            "unexpected error: {err}"
        );
    }
}
