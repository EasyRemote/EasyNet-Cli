// EasyNet CLI — teach-grant directory (`teach-grants.json`)
// ===========================================================
//
// File: src/persistence/teach_grants.rs
// Description: The owner-initiative store behind GET route B
//              (seven-axes T3.3, spec §2.5): which abilities their
//              owner has explicitly made learnable, and by whom.
//
// Ontology (spec §0.1-6, non-negotiable): a capability is CONFERRED
// by its owner, never pulled by a consumer. Absence of a grant IS
// the `allow_transferred_code = false` default (capability.proto
// InstallPolicy) — an ability with no entry here is not learnable,
// full stop. `meta.teach` writes a grant; `meta.acquire` consumes
// one; both are ordinary ledgered invocations, so the receipt chain
// records who conferred what to whom.
//
// The file also keeps the LEARNED ledger: which manifests landed in
// which learner's workspace through `meta.acquire`. `meta.forget`
// only removes what this ledger names — a learner can unlearn a
// taught copy, never silently delete a native ability.
//
// Schema (operator-inspectable)
// -----------------------------
// {
//   "grants": [
//     { "ability": "<owner-local registry name, e.g. testbot.weather-probe>",
//       "owner_agent": "testbot",
//       "learner_ura": "easynet:///r/<realm>/agent/<id>",
//       "execution_mode": "sandbox_first",   // capability.proto:238 default
//       "granted_at": "<rfc3339>" },
//     …
//   ],
//   "learned": [
//     { "ability_name": "weather-probe",
//       "learner_agent": "apprentice",
//       "learned_from": "<the taught ability's canonical URA>",
//       "learned_at": "<rfc3339>" },
//     …
//   ]
// }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

pub(crate) const FILE_NAME: &str = "teach-grants.json";

/// Default execution posture for transferred code
/// (capability.proto:238). A string by protocol design — the proto
/// field is a string with documented values, not an enum.
pub const EXECUTION_MODE_DEFAULT: &str = "sandbox_first";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeachGrantsFile {
    #[serde(default)]
    pub grants: Vec<TeachGrant>,
    #[serde(default)]
    pub learned: Vec<LearnedRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeachGrant {
    pub ability: String,
    pub owner_agent: String,
    pub learner_ura: String,
    pub execution_mode: String,
    pub granted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRecord {
    pub ability_name: String,
    pub learner_agent: String,
    pub learned_from: String,
    pub learned_at: String,
}

pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub fn load() -> anyhow::Result<TeachGrantsFile> {
    let p = path();
    if !p.exists() {
        return Ok(TeachGrantsFile::default());
    }
    let bytes = fs::read(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

pub fn save(file: &TeachGrantsFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
}

impl TeachGrantsFile {
    /// The grant covering `(ability, learner)`, if the owner conferred
    /// one. No entry = not learnable (the InstallPolicy default).
    pub fn grant_for(&self, ability: &str, learner_ura: &str) -> Option<&TeachGrant> {
        self.grants
            .iter()
            .find(|g| g.ability == ability && g.learner_ura == learner_ura)
    }

    /// The learned-ledger row for `(learner, ability)` — what
    /// `meta.forget` is allowed to remove.
    pub fn learned_by(&self, learner_agent: &str, ability_name: &str) -> Option<usize> {
        self.learned
            .iter()
            .position(|l| l.learner_agent == learner_agent && l.ability_name == ability_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_grant_is_the_default_refusal() {
        let file = TeachGrantsFile::default();
        assert!(
            file.grant_for("testbot.weather-probe", "ura").is_none(),
            "no entry means allow_transferred_code=false"
        );
    }

    #[test]
    fn grant_is_scoped_to_one_learner() {
        let mut file = TeachGrantsFile::default();
        file.grants.push(TeachGrant {
            ability: "testbot.weather-probe".into(),
            owner_agent: "testbot".into(),
            learner_ura: "ura-b".into(),
            execution_mode: EXECUTION_MODE_DEFAULT.into(),
            granted_at: "t0".into(),
        });
        assert!(file.grant_for("testbot.weather-probe", "ura-b").is_some());
        assert!(
            file.grant_for("testbot.weather-probe", "ura-c").is_none(),
            "a grant confers to ONE learner, not to the world"
        );
    }
}
