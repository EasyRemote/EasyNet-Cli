// EasyNet CLI — trust-level directory (`trust-levels.json`)
// ==========================================================
//
// File: src/persistence/trust_levels.rs
// Description: On-disk directory of explicit trust-level rulings,
//              keyed by canonical Agent URA (seven-axes W2 T2.1,
//              D8 default: the trust subject is the *agent*; any
//              node-level projection for the Axon enforcement gate
//              is derived, never stored here).
//
// This is the storage behind the `identity.get_trust` /
// `identity.set_trust` abilities (RFC-001 restatement of the former
// GetNodeTrust/SetNodeTrust RPCs). It records *rulings* — an entry
// exists only when an operator explicitly set a level. Absence of an
// entry is not a level; the read surface reports the baseline
// (`STANDARD`) with `source = "default"` so consumers can tell a
// ruling from a default.
//
// Schema (operator-inspectable, like `local-agents.json`)
// -------------------------------------------------------
// {
//   "levels": {
//     "easynet:///r/<realm>/agent/<id>": {
//       "trust_level":            "UNTRUSTED" | "PROBATION" | "STANDARD"
//                                 | "ELEVATED" | "PRIVILEGED",
//       "updated_at":             "<rfc3339>",
//       "updated_by_invocation":  "<invocation-id>" | absent
//     },
//     …
//   }
// }
//
// What this file is NOT
// ---------------------
// - Not the realm trust anchor (`realm-trust.toml`): the anchor
//   answers "whose KEYS does admission accept" (commit-plan-2 D3);
//   this directory answers "once accepted, how far do we trust them"
//   (the TrustLevel attribute the Axon resilience gate consumes).
// - Not a protocol shape: level names mirror Axon's `TrustLevel`
//   enum (types.proto:729) and are validated against the pb enum at
//   the ability layer — this module stores strings it was handed.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

pub(crate) const FILE_NAME: &str = "trust-levels.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustLevelsFile {
    /// Explicit rulings, keyed by canonical Agent URA.
    #[serde(default)]
    pub levels: BTreeMap<String, TrustLevelRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustLevelRecord {
    /// Canonical short level name (`STANDARD`, `ELEVATED`, …) —
    /// validated against the pb `TrustLevel` enum by the writer.
    pub trust_level: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_invocation: Option<String>,
}

pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub fn load() -> anyhow::Result<TrustLevelsFile> {
    let p = path();
    if !p.exists() {
        return Ok(TrustLevelsFile::default());
    }
    let bytes = fs::read(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

/// Atomically write the file with mode 0600 (same discipline as
/// `local-agents.json`).
pub fn save(file: &TrustLevelsFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
}

impl TrustLevelsFile {
    pub fn get(&self, agent_ura: &str) -> Option<&TrustLevelRecord> {
        self.levels.get(agent_ura)
    }

    /// Record a ruling, returning the previous level name when one
    /// existed (the ability surfaces it so a `set` receipt tells the
    /// whole story).
    pub fn upsert(
        &mut self,
        agent_ura: &str,
        trust_level: &str,
        updated_at: &str,
        updated_by_invocation: Option<String>,
    ) -> Option<String> {
        self.levels
            .insert(
                agent_ura.to_string(),
                TrustLevelRecord {
                    trust_level: trust_level.to_string(),
                    updated_at: updated_at.to_string(),
                    updated_by_invocation,
                },
            )
            .map(|prev| prev.trust_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_round_trips_and_absent_entry_is_none() {
        let file = TrustLevelsFile::default();
        let json = serde_json::to_string(&file).expect("serialize");
        let back: TrustLevelsFile = serde_json::from_str(&json).expect("parse");
        assert!(back.get("easynet:///r/x/agent/u.a").is_none());
    }

    #[test]
    fn upsert_returns_previous_ruling() {
        let mut file = TrustLevelsFile::default();
        assert_eq!(file.upsert("ura", "STANDARD", "t0", None), None);
        assert_eq!(
            file.upsert("ura", "ELEVATED", "t1", Some("inv-1".into())),
            Some("STANDARD".to_string())
        );
        let rec = file.get("ura").expect("ruling exists");
        assert_eq!(rec.trust_level, "ELEVATED");
        assert_eq!(rec.updated_by_invocation.as_deref(), Some("inv-1"));
    }
}
