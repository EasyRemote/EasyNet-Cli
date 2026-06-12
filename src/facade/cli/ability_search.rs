// EasyNet CLI — `easynet ability search` subcommand
// ===================================================
//
// File: src/facade/cli/ability_search.rs
// Description: Intent-first ability discovery (commit-plan-2 D2 /
//              Gate D). The user types what they want done; we rank
//              candidates from the daemon's own catalogue
//              (`meta.list_abilities`) and, when the daemon is
//              federated, from the realm directory
//              (`federation.discover`) — one resolver-backed path, no
//              agent IDs or internal namespaces required.
//
// Ranking contract (deliberately simple and explainable, extensible
// later per commit-plan-2 D2): the query is lowercased and tokenized;
// each candidate scores token hits against its name (weight 3, +2
// extra for a name-segment prefix hit), description (1), and owner
// segment (1), with a +2 bonus when every token hit somewhere.
// Zero-score candidates are dropped. No LLM, no network ranking — a
// user can always predict why a row ranked where it did.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::support::local_invoke::invoke_local_ability;
use crate::support::output::OutputFormat;

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// What you want done, in your own words — e.g. "read a file on
    /// my laptop" or "chat with codex".
    pub intent: String,
    /// Maximum candidates to print.
    #[arg(long, default_value_t = 15)]
    pub limit: usize,
    /// Search only this daemon's own catalogue; skip the federated
    /// realm directory.
    #[arg(long)]
    pub local_only: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// One ranked candidate, normalised from either source.
#[derive(Debug, serde::Serialize)]
struct Candidate {
    score: u32,
    name: String,
    description: String,
    /// `local` (this daemon's catalogue) or `federated` (realm
    /// directory projection).
    origin: &'static str,
    /// Owner URA when the federated directory supplied one.
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_ura: Option<String>,
}

pub fn run(args: SearchArgs) -> anyhow::Result<()> {
    let tokens = tokenize(&args.intent);
    if tokens.is_empty() {
        anyhow::bail!("intent is empty after tokenization; describe what you want done");
    }

    let mut candidates = local_candidates(&tokens)?;
    if !args.local_only {
        match federated_candidates(&tokens) {
            Ok(mut federated) => candidates.append(&mut federated),
            Err(err) => {
                // Federation being unreachable must not hide local
                // results: degrade softly and say so.
                eprintln!(
                    "{}",
                    style(format!(
                        "note: federated directory unavailable ({err:#}); showing local \
                         catalogue only"
                    ))
                    .dim()
                );
            }
        }
    }

    // Highest score first; ties resolve by name so output is stable.
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
    candidates.dedup_by(|a, b| a.name == b.name && a.origin == b.origin);
    candidates.truncate(args.limit);

    if matches!(args.format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "intent": args.intent, "candidates": candidates })
            )?
        );
        return Ok(());
    }

    if candidates.is_empty() {
        println!(
            "{}",
            style(format!(
                "no abilities matched \"{}\" — try broader words, or `easynet ability list` \
                 to browse the catalogue",
                args.intent
            ))
            .dim()
        );
        return Ok(());
    }

    println!(
        "{:>5}  {:<34} {:<9} {}",
        style("SCORE").bold(),
        style("ABILITY").bold(),
        style("ORIGIN").bold(),
        style("DESCRIPTION").bold()
    );
    for c in &candidates {
        println!(
            "{:>5}  {:<34} {:<9} {}",
            c.score,
            truncate(&c.name, 34),
            c.origin,
            truncate(&c.description, 50),
        );
    }
    println!(
        "\n{}",
        style("invoke one with: easynet ability invoke <ABILITY> [args…]").dim()
    );
    Ok(())
}

/// This daemon's own catalogue via `meta.list_abilities` — the same
/// surface `ability list` reads, so search and list can never drift.
fn local_candidates(tokens: &[String]) -> anyhow::Result<Vec<Candidate>> {
    let value = invoke_local_ability("meta.list_abilities", json!({}))
        .context("query local ability catalogue")?;
    let rows = value
        .get("abilities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(rows
        .iter()
        .filter_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            let description = row
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let score = score_candidate(tokens, name, description, None);
            (score > 0).then(|| Candidate {
                score,
                name: name.to_string(),
                description: description.to_string(),
                origin: "local",
                owner_ura: None,
            })
        })
        .collect())
}

/// Federated realm directory via `federation.discover`. Directory
/// entries project per-agent ability catalogues when peers advertised
/// them (`include_abilities`); entries without ability rows still
/// surface as owner-level candidates so a matching agent is
/// discoverable even before its abilities are advertised.
fn federated_candidates(tokens: &[String]) -> anyhow::Result<Vec<Candidate>> {
    let value = invoke_local_ability("federation.discover", json!({}))
        .context("query federated directory")?;
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for entry in &entries {
        let owner_ura = entry
            .get("agent_ura")
            .or_else(|| entry.get("device_ura"))
            .or_else(|| entry.get("ura"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let abilities = entry.get("abilities").and_then(Value::as_array);
        match abilities {
            Some(rows) if !rows.is_empty() => {
                for row in rows {
                    let Some(name) = row.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let description = row
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let score = score_candidate(tokens, name, description, Some(&owner_ura));
                    if score > 0 {
                        out.push(Candidate {
                            score,
                            name: name.to_string(),
                            description: description.to_string(),
                            origin: "federated",
                            owner_ura: Some(owner_ura.clone()),
                        });
                    }
                }
            }
            _ => {
                if owner_ura.is_empty() {
                    continue;
                }
                let score = score_candidate(tokens, &owner_ura, "", Some(&owner_ura));
                if score > 0 {
                    out.push(Candidate {
                        score,
                        name: owner_ura.clone(),
                        description: "(directory entry — abilities not yet advertised)".to_string(),
                        origin: "federated",
                        owner_ura: Some(owner_ura.clone()),
                    });
                }
            }
        }
    }
    Ok(out)
}

fn tokenize(intent: &str) -> Vec<String> {
    intent
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(ToOwned::to_owned)
        .collect()
}

/// Explainable scoring: name hit 3 (+2 for a segment prefix),
/// description hit 1, owner hit 1, +2 when every token hit somewhere.
fn score_candidate(tokens: &[String], name: &str, description: &str, owner: Option<&str>) -> u32 {
    let name_lc = name.to_lowercase();
    let desc_lc = description.to_lowercase();
    let owner_lc = owner.map(str::to_lowercase).unwrap_or_default();
    let segments: Vec<&str> = name_lc
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    let mut score = 0_u32;
    let mut all_hit = true;
    for token in tokens {
        let mut hit = false;
        if name_lc.contains(token.as_str()) {
            score += 3;
            hit = true;
            if segments.iter().any(|s| s.starts_with(token.as_str())) {
                score += 2;
            }
        }
        if desc_lc.contains(token.as_str()) {
            score += 1;
            hit = true;
        }
        if !owner_lc.is_empty() && owner_lc.contains(token.as_str()) {
            score += 1;
            hit = true;
        }
        all_hit &= hit;
    }
    if score > 0 && all_hit && tokens.len() > 1 {
        score += 2;
    }
    score
}

fn truncate(text: &str, max: usize) -> String {
    crate::facade::cli::abilities::truncate_display(text, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tokens: &[&str], name: &str, desc: &str) -> u32 {
        let toks: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        score_candidate(&toks, name, desc, None)
    }

    #[test]
    fn name_segment_prefix_outranks_description_hit() {
        assert!(s(&["chat"], "alice.codex.chat", "") > s(&["chat"], "fs.read", "chat helper"));
    }

    #[test]
    fn all_token_bonus_rewards_full_intent_coverage() {
        let full = s(&["read", "file"], "device.fs.read", "read a file from disk");
        let partial = s(&["read", "file"], "device.fs.read", "");
        assert!(full > partial);
    }

    #[test]
    fn zero_score_for_unrelated_candidates() {
        assert_eq!(s(&["weather"], "device.fs.read", "read a file"), 0);
    }

    #[test]
    fn tokenize_drops_punctuation_and_short_words() {
        assert_eq!(tokenize("read a file, now!"), vec!["read", "file", "now"]);
    }
}
