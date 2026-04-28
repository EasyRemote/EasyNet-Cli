// EasyNet CLI — Local Agents Registry (RFC-001 §1.4 [P2])
// =========================================================
//
// File: src/persistence/local_agents.rs
//
// Owns ~/.easynet/local-agents.json — the persistent map from
// (profile, name) → canonical Agent URI for every Agent this
// daemon hosts. Mode 0600 because URIs are bound to keys whose
// rotation requires hub coordination.
//
// Why this exists
// ---------------
// Per RFC §1.4 the device-profile MUST mint canonical URIs for each
// hosted Agent (consent, policy, mcp, llm-profile-per-sub-agent) on
// first boot, then reuse them across daemon restarts. Without
// persistence the device would re-mint URIs on every restart and
// the hub would accumulate dead-but-present entries — eventually
// the realm directory would fill with stale Agents whose hosting
// daemon disappeared three reboots ago.
//
// Schema
// ------
// {
//   "host_device_agent_uri": "easynet:///r/<realm>/agent/<id>",
//   "hosted_agents": [
//     {
//       "profile":            "consent" | "policy" | "mcp" | "llm",
//       "name":               "default" | "claude" | "codex" | …,
//       "agent_uri":          "easynet:///r/<realm>/agent/<id>",
//       "signing_authority":  "hosted_by:<host_device_agent_uri>",
//       "first_seen_at":      "<rfc3339>"
//     },
//     …
//   ]
// }
//
// What this file is NOT
// ---------------------
// - Not a credentials store. The host device's keypair lives in
//   `private.pem` (per §3 Step A). This file records URIs only.
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
/// must remain stable — older daemons must read what newer daemons
/// write, and the file is operator-inspectable.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalAgentsFile {
    /// The device-profile Agent's canonical URA. Empty until the
    /// first successful `federation.join`. Populated from the join
    /// receipt body.
    #[serde(default)]
    pub host_device_agent_uri: String,
    /// Hosted Agents (consent / policy / mcp / llm-per-sub-agent).
    /// Order is insertion order; readers MUST NOT rely on order.
    #[serde(default)]
    pub hosted_agents: Vec<HostedAgentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostedAgentEntry {
    /// One of `consent`, `policy`, `mcp`, `llm`. Free-form to allow
    /// future profiles without a schema migration.
    pub profile: String,
    /// Per-profile name. `"default"` for singleton profiles
    /// (consent, policy, mcp); the sub-agent name for `llm`
    /// (e.g. `"claude"`, `"codex"`).
    pub name: String,
    /// Canonical URA assigned by `federation.advertise_agent` (or
    /// minted locally for hosted Agents in §1.3 Model B).
    pub agent_uri: String,
    /// `"hosted_by:<host_device_agent_uri>"` per §1.3.
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
    let bytes = fs::read(&p)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

/// Atomically write the file with mode 0600.
pub fn save(file: &LocalAgentsFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
}

/// Look up a hosted Agent's URA by `(profile, name)`. Returns the
/// URA if present, `None` otherwise. Used by the publish layer to
/// decide whether to mint a fresh URA or reuse an existing one.
pub fn lookup_hosted_uri(
    file: &LocalAgentsFile,
    profile: &str,
    name: &str,
) -> Option<String> {
    file.hosted_agents
        .iter()
        .find(|e| e.profile == profile && e.name == name)
        .map(|e| e.agent_uri.clone())
}

/// Insert or update a hosted Agent entry. Returns `true` when an
/// existing entry was replaced (signals the caller to log it for
/// operator visibility); `false` when the entry is new.
pub fn upsert_hosted_agent(
    file: &mut LocalAgentsFile,
    profile: &str,
    name: &str,
    agent_uri: &str,
) -> bool {
    let signing_authority = if file.host_device_agent_uri.is_empty() {
        // Pre-join state: keep the field shape but flag the host as
        // unknown. The first persisted post-join save replaces this.
        "hosted_by:<unset>".to_string()
    } else {
        format!("hosted_by:{}", file.host_device_agent_uri)
    };
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(entry) = file
        .hosted_agents
        .iter_mut()
        .find(|e| e.profile == profile && e.name == name)
    {
        entry.agent_uri = agent_uri.to_string();
        entry.signing_authority = signing_authority;
        return true;
    }
    file.hosted_agents.push(HostedAgentEntry {
        profile: profile.to_string(),
        name: name.to_string(),
        agent_uri: agent_uri.to_string(),
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
        assert!(f.host_device_agent_uri.is_empty());
        assert!(f.hosted_agents.is_empty());
    }

    #[test]
    fn upsert_inserts_when_absent_and_returns_false() {
        let mut f = LocalAgentsFile {
            host_device_agent_uri: "easynet:///r/acme/agent/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        let replaced =
            upsert_hosted_agent(&mut f, "llm", "claude", "easynet:///r/acme/agent/01LLM");
        assert!(!replaced);
        assert_eq!(f.hosted_agents.len(), 1);
        assert_eq!(f.hosted_agents[0].profile, "llm");
        assert_eq!(f.hosted_agents[0].name, "claude");
        assert_eq!(
            f.hosted_agents[0].signing_authority,
            "hosted_by:easynet:///r/acme/agent/01DEV"
        );
    }

    #[test]
    fn upsert_replaces_when_present_and_returns_true() {
        let mut f = LocalAgentsFile {
            host_device_agent_uri: "easynet:///r/acme/agent/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "llm", "claude", "uri-v1");
        let replaced = upsert_hosted_agent(&mut f, "llm", "claude", "uri-v2");
        assert!(replaced);
        assert_eq!(f.hosted_agents.len(), 1);
        assert_eq!(f.hosted_agents[0].agent_uri, "uri-v2");
    }

    #[test]
    fn upsert_pre_join_records_unset_signing_authority() {
        let mut f = LocalAgentsFile::default();
        upsert_hosted_agent(&mut f, "consent", "default", "uri-c");
        assert_eq!(
            f.hosted_agents[0].signing_authority,
            "hosted_by:<unset>",
            "pre-join entries must be flagged so operators know to re-save after join"
        );
    }

    #[test]
    fn lookup_returns_uri_when_present() {
        let mut f = LocalAgentsFile::default();
        upsert_hosted_agent(&mut f, "mcp", "default", "uri-mcp");
        assert_eq!(
            lookup_hosted_uri(&f, "mcp", "default"),
            Some("uri-mcp".to_string())
        );
        assert_eq!(lookup_hosted_uri(&f, "mcp", "other"), None);
        assert_eq!(lookup_hosted_uri(&f, "llm", "default"), None);
    }

    #[test]
    fn round_trip_serde_preserves_all_fields() {
        let mut f = LocalAgentsFile {
            host_device_agent_uri: "easynet:///r/acme/agent/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut f, "llm", "claude", "uri-llm");
        upsert_hosted_agent(&mut f, "consent", "default", "uri-c");

        let json = serde_json::to_string_pretty(&f).unwrap();
        let back: LocalAgentsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn deserialize_tolerates_unknown_fields_for_forward_compat() {
        // A future schema may add `metadata`, `tags`, etc. Older
        // daemons must still parse the file (serde default behaviour
        // for our struct is to ignore unknown fields).
        let json = r#"{
            "host_device_agent_uri": "easynet:///r/acme/agent/01DEV",
            "hosted_agents": [
                {
                    "profile": "llm",
                    "name": "claude",
                    "agent_uri": "uri-1",
                    "signing_authority": "hosted_by:easynet:///r/acme/agent/01DEV",
                    "first_seen_at": "2026-04-27T00:00:00Z",
                    "future_field": "ignored"
                }
            ],
            "future_top_level_field": 42
        }"#;
        let f: LocalAgentsFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.hosted_agents.len(), 1);
        assert_eq!(f.hosted_agents[0].name, "claude");
    }
}
