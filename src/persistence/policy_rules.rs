// EasyNet CLI — policy rule store (`policy-rules.json`)
// ======================================================
//
// File: src/persistence/policy_rules.rs
// Description: The persisted rule store behind `policy.evaluate` /
//              `policy.simulate` (seven-axes T3.2, CLI half). The
//              evaluator is a TINY MATCHER by ruling (spec D7): three
//              guided predicates, no expression language — review
//              §3.8 chose "tiny matcher" over re-implementing OPA.
//
// Rule semantics (first match wins, file order; no priority field
// until a real need shows up):
//   * `action`        — must equal the admitted action; today every
//                       admitted envelope is an "invoke".
//   * `family_prefix` — optional; matches when the envelope's ability
//                       name starts with it. Family is a POLICY SCOPE
//                       and display projection only — never an
//                       address (spec §0.1-5).
//   * `trust_below`   — optional; matches when the caller's recorded
//                       trust level ranks strictly below this level
//                       (`deny action=invoke unless trust>=STANDARD`
//                       ≡ `{effect: deny, trust_below: "STANDARD"}`).
//                       Levels rank by the Axon pb `TrustLevel` enum.
//   * no rule matches — the baseline allows: an empty store keeps
//                       the daemon's historical allow-all behaviour,
//                       said out loud in the decision reason.
//
// Schema (operator-inspectable)
// -----------------------------
// {
//   "rules": [
//     { "id": "pr-1", "effect": "deny" | "allow", "action": "invoke",
//       "family_prefix": "aris." | absent,
//       "trust_below": "STANDARD" | absent,
//       "created_at": "<rfc3339>" },
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

pub(crate) const FILE_NAME: &str = "policy-rules.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyRulesFile {
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleEffect {
    Allow,
    Deny,
}

impl RuleEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleEffect::Allow => "allow",
            RuleEffect::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub effect: RuleEffect,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_below: Option<String>,
    pub created_at: String,
}

pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

pub fn load() -> anyhow::Result<PolicyRulesFile> {
    let p = path();
    if !p.exists() {
        return Ok(PolicyRulesFile::default());
    }
    let bytes = fs::read(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

pub fn save(file: &PolicyRulesFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
}

impl PolicyRulesFile {
    /// Next free rule id (`pr-<n>`). Ids are never reused within one
    /// file lifetime: the counter follows the highest existing id, so
    /// `policy why`-style references stay unambiguous after removals.
    pub fn next_id(&self) -> String {
        let max = self
            .rules
            .iter()
            .filter_map(|r| r.id.strip_prefix("pr-")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("pr-{}", max + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_id_follows_the_highest_survivor() {
        let mut file = PolicyRulesFile::default();
        assert_eq!(file.next_id(), "pr-1");
        file.rules.push(PolicyRule {
            id: "pr-7".into(),
            effect: RuleEffect::Deny,
            action: "invoke".into(),
            family_prefix: None,
            trust_below: Some("STANDARD".into()),
            created_at: "t0".into(),
        });
        assert_eq!(file.next_id(), "pr-8", "ids never regress after removals");
    }

    #[test]
    fn rule_round_trips_with_optional_predicates_absent() {
        let rule = PolicyRule {
            id: "pr-1".into(),
            effect: RuleEffect::Allow,
            action: "invoke".into(),
            family_prefix: None,
            trust_below: None,
            created_at: "t0".into(),
        };
        let json = serde_json::to_string(&rule).expect("serialize");
        assert!(!json.contains("family_prefix"), "absent predicates vanish");
        let back: PolicyRule = serde_json::from_str(&json).expect("parse");
        assert_eq!(back.effect, RuleEffect::Allow);
    }
}
