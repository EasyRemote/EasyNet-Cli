// EasyNet CLI — `easynet discover` / `easynet ability search`
// ============================================================
//
// File: src/facade/cli/discover.rs
// Description: Intent-first ability discovery over the daemon's
//              four-tier `<agent>.discover` ladder (seven-axes spec
//              W1, tasks T1.1–T1.3). One resolver path, two calls:
//
//                tier 1 self    ┐  one call, scope="device"
//                tier 2 device  ┘  (local registry walk)
//                tier 3 user    —  one call, scope="user"
//                                  (federation.resolve via the hub)
//
//              Tier 4 (scope="public", cross-tenant) is deliberately
//              not searched by default: realm-wide browsing answers
//              the daily question; tenant-crossing is an explicit
//              product opt-in, not a default.
//
// Ranking contract (frozen — spec W1-E2E-1 asserts it recomputable):
// the intent is lowercased and tokenized; each candidate scores token
// hits against its display name (weight 3, +2 for a name-segment
// prefix hit), description (1), and owner segment (1), with +2 when
// every token hit somewhere. Zero-score candidates drop. No LLM, no
// network-side ranking — a user can always predict why a row ranked
// where it did. The runtime ladder is therefore queried WITHOUT a
// `query` argument: it is the candidate *source* (uniform score),
// ranking stays here, so source and ranking never entangle.
//
// The owner segment fed to the scorer is the candidate's canonical
// Ability URA for every tier — the earlier dual-path implementation
// scored local rows ownerless, which made identical abilities rank
// differently by origin. One source, one scoring input shape.
//
// Federation degradation is typed, never fatal: tier 3 returns
// `{error: {code: federation_not_joined | federation_unavailable}}`,
// surfaced as a `federation` status object while the local tiers
// still print, exit code 0 (spec D9).
//
// The seven-tuple is auditable: the tier-1/2 call goes through
// `invoke_local_ability_with_invocation_meta`, and the envelope echo
// (caller / callee / ability / subject / …) is included verbatim in
// `--format json` output as `invocation` (spec 0.1-7, W1-E2E-1 ⑤).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::support::local_invoke::{
    invoke_local_ability, invoke_local_ability_with_invocation_meta,
};
use crate::ura::AbilitySelector;

/// Narrow re-export so integration tests (and other `pub` consumers
/// of this module) can name the flag type without opening the whole
/// `support::output` leaf layer.
pub use crate::support::output::OutputFormat;

/// Candidates requested from each ladder scope. Deliberately far
/// above the runtime's `DEFAULT_TOP_K = 20`: the ladder is our
/// candidate *source*, and ranking happens here — a source that
/// pre-truncates would silently hide rows the scorer wants.
const SOURCE_TOP_K: usize = 200;

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// What you want done, in your own words — e.g. "read a file on
    /// my laptop" or "chat with codex".
    pub intent: String,
    /// Maximum candidates to print.
    #[arg(long, default_value_t = 15)]
    pub limit: usize,
    /// Search only this device's tiers (self + device); skip the
    /// realm directory.
    #[arg(long)]
    pub local_only: bool,
    /// Group the human-readable listing as an owner tree (display
    /// projection only — never an address; spec §0.1-5). Ignored
    /// with `--format json`, which already carries owner fields.
    #[arg(long)]
    pub tree: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// One ranked candidate, projected from a runtime ladder row.
///
/// `owner_ura` stays unserialized (tree grouping key); the serialized
/// shape is the frozen W1-E2E-1 contract: score / name / ura /
/// owner_kind / scope / trust_level / description.
#[derive(Debug, serde::Serialize)]
pub struct Candidate {
    pub score: u32,
    pub name: String,
    pub ura: String,
    pub owner_kind: &'static str,
    pub scope: String,
    /// Always `null` until W2 wires `identity.get_trust` — the
    /// column renders `–` so the surface says what it doesn't know.
    pub trust_level: Option<String>,
    pub description: String,
    #[serde(skip)]
    pub owner_ura: String,
}

impl Candidate {
    /// Project one runtime ladder row, scoring it against the intent
    /// tokens. Returns `None` for zero-score rows and for rows whose
    /// `qualified_name` does not round-trip the Axon URA parser —
    /// the caller counts those separately so dropped rows are never
    /// silent (spec §0.1-8).
    fn from_ladder_row(row: &Value, tokens: &[String]) -> Result<Option<Self>, ()> {
        let ura = row
            .get("qualified_name")
            .and_then(Value::as_str)
            .ok_or(())?;
        let selector = AbilitySelector::parse(ura).map_err(|_| ())?;
        let owner = row.get("owner").and_then(Value::as_str).unwrap_or_default();
        let ability = row
            .get("ability")
            .and_then(Value::as_str)
            .unwrap_or_else(|| selector.public_name());
        let description = row
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let scope = row
            .get("scope_matched")
            .and_then(Value::as_str)
            .unwrap_or("device");

        let name = if owner.is_empty() {
            ability.to_string()
        } else {
            format!("{owner}.{ability}")
        };
        let score = score_candidate(tokens, &name, description, Some(ura));
        if score == 0 {
            return Ok(None);
        }
        Ok(Some(Candidate {
            score,
            name,
            ura: ura.to_string(),
            owner_kind: selector.owner_kind(),
            scope: scope.to_string(),
            trust_level: None,
            description: description.to_string(),
            owner_ura: selector.owner_ura().to_string(),
        }))
    }
}

/// Typed projection of the ladder's federation error envelope.
#[derive(Debug, serde::Serialize)]
pub struct FederationStatus {
    pub status: String,
    pub message: String,
}

impl FederationStatus {
    /// `Some` when the tier-3 response is the typed degradation
    /// envelope (`{error: {code, message}}`), `None` for a normal
    /// candidate payload.
    fn from_envelope(value: &Value) -> Option<Self> {
        let err = value.get("error")?;
        Some(FederationStatus {
            status: err
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("federation_unavailable")
                .to_string(),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// The full result of one discover run — owns its renderings.
///
/// `execute` returns this typed report so integration tests (and any
/// future surface: TUI, MCP) assert on data, not on captured stdout;
/// `run` is the thin render shell.
#[derive(Debug, serde::Serialize)]
pub struct DiscoverReport {
    pub query: String,
    pub tiers_searched: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation: Option<FederationStatus>,
    /// Envelope echo of the tier-1/2 invocation (seven-tuple audit
    /// surface; spec 0.1-7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation: Option<Value>,
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped_unparseable: usize,
    pub candidates: Vec<Candidate>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl DiscoverReport {
    fn render_json(&self) -> anyhow::Result<()> {
        println!("{}", serde_json::to_string_pretty(self)?);
        Ok(())
    }

    fn render_human(&self, tree: bool) {
        if let Some(fed) = &self.federation {
            eprintln!(
                "{}",
                style(format!(
                    "note: realm tier skipped ({}): {}",
                    fed.status, fed.message
                ))
                .dim()
            );
        }
        if self.skipped_unparseable > 0 {
            eprintln!(
                "{}",
                style(format!(
                    "note: {} candidate(s) dropped — non-canonical URA from a peer",
                    self.skipped_unparseable
                ))
                .dim()
            );
        }
        if self.candidates.is_empty() {
            println!(
                "{}",
                style(format!(
                    "no abilities matched \"{}\" — try broader words, or `easynet ability list` \
                     to browse the catalogue",
                    self.query
                ))
                .dim()
            );
            return;
        }
        if tree {
            self.render_tree();
        } else {
            self.render_table();
        }
        println!(
            "\n{}",
            style("invoke one with: easynet ability invoke <URA> [args…]").dim()
        );
    }

    fn render_table(&self) {
        println!(
            "{:>5}  {:<34} {:<7} {:<6} {}",
            style("SCORE").bold(),
            style("ABILITY").bold(),
            style("TIER").bold(),
            style("TRUST").bold(),
            style("DESCRIPTION").bold()
        );
        for c in &self.candidates {
            println!(
                "{:>5}  {:<34} {:<7} {:<6} {}",
                c.score,
                truncate(&c.name, 34),
                c.scope,
                c.trust_level.as_deref().unwrap_or("–"),
                truncate(&c.description, 44),
            );
        }
    }

    /// Owner tree — a display projection over `owner_ura`, never an
    /// address (spec §3.6: the tree belongs to discovery, not to
    /// calling or addressing).
    fn render_tree(&self) {
        for (owner, rows) in self.group_by_owner() {
            let kind = rows.first().map(|c| c.owner_kind).unwrap_or("agent");
            println!(
                "{} {}",
                style(owner).bold(),
                style(format!("[{kind}]")).dim()
            );
            let last = rows.len().saturating_sub(1);
            for (i, c) in rows.iter().enumerate() {
                let branch = if i == last { "└─" } else { "├─" };
                println!(
                    "  {branch} {:<30} {:<7} {:>4}  {}",
                    truncate(&c.name, 30),
                    c.scope,
                    c.score,
                    truncate(&c.description, 40),
                );
            }
        }
    }

    /// Stable owner grouping (BTreeMap: owners sort lexically);
    /// within a group the overall score ordering is preserved.
    fn group_by_owner(&self) -> BTreeMap<&str, Vec<&Candidate>> {
        let mut groups: BTreeMap<&str, Vec<&Candidate>> = BTreeMap::new();
        for c in &self.candidates {
            groups.entry(c.owner_ura.as_str()).or_default().push(c);
        }
        groups
    }
}

pub fn run(args: DiscoverArgs) -> anyhow::Result<()> {
    let report = execute(&args)?;
    match args.format {
        OutputFormat::Json => report.render_json(),
        OutputFormat::Table => {
            report.render_human(args.tree);
            Ok(())
        }
    }
}

/// Compute the discover report without rendering it — the typed
/// surface e2e tests and future renderers consume.
pub fn execute(args: &DiscoverArgs) -> anyhow::Result<DiscoverReport> {
    let tokens = tokenize(&args.intent);
    if tokens.is_empty() {
        anyhow::bail!("intent is empty after tokenization; describe what you want done");
    }

    let ladder = resolve_ladder_entry()?;
    let mut skipped = 0_usize;

    // Tiers 1+2 — local registry walk, with the envelope echo kept
    // for the audit surface.
    let (local_value, invocation_meta) = invoke_local_ability_with_invocation_meta(
        &ladder,
        json!({ "scope": "device", "top_k": SOURCE_TOP_K }),
        None,
        &[],
        None,
        None,
        None,
    )
    .context("walk local discover tiers (self + device)")?;
    let mut candidates = project_rows(&local_value, &tokens, &mut skipped);
    let mut tiers_searched = vec!["self", "device"];

    // Tier 3 — realm directory, typed degradation on failure.
    let mut federation = None;
    if !args.local_only {
        let realm_value =
            invoke_local_ability(&ladder, json!({ "scope": "user", "top_k": SOURCE_TOP_K }))
                .context("walk realm discover tier (user)")?;
        match FederationStatus::from_envelope(&realm_value) {
            Some(status) => federation = Some(status),
            None => {
                candidates.extend(project_rows(&realm_value, &tokens, &mut skipped));
                tiers_searched.push("user");
            }
        }
    }

    // Highest score first; ties resolve by name, then URA, so output
    // is stable. One ability appears once: the URA is the identity.
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.ura.cmp(&b.ura))
    });
    candidates.dedup_by(|a, b| a.ura == b.ura);
    candidates.truncate(args.limit);

    Ok(DiscoverReport {
        query: args.intent.clone(),
        tiers_searched,
        federation,
        invocation: Some(invocation_meta),
        skipped_unparseable: skipped,
        candidates,
    })
}

/// Resolve the daemon's discover entry point from the agent registry.
///
/// The ladder is registered per agent (`<agent>.discover`,
/// `discover_ability::register_for_agent`), so "whose ladder" is a
/// question for the agent registry — `agent.list` — not for the
/// ability catalogue. The CLI hardcodes no agent name: it takes the
/// lexically first registered agent so the choice is deterministic.
/// Any agent's ladder walks the same device registry; only the
/// self-tier attribution differs, and the tiers merge in our output
/// anyway. The dispatch key mirrors the registration site's
/// `format!("{agent}.{ABILITY_VERB}")` — one shared verb constant,
/// no second spelling.
fn resolve_ladder_entry() -> anyhow::Result<String> {
    let value = invoke_local_ability("agent.list", json!({}))
        .context("resolve discover entry from the agent registry")?;
    let mut names: Vec<String> = value
        .get("agents")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
        .into_iter()
        .next()
        .map(|agent| {
            format!(
                "{agent}.{}",
                crate::runtime::agents::discover_ability::ABILITY_VERB
            )
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no agents are registered on this daemon, so no \
                 `<agent>.discover` ladder exists; add one with \
                 `easynet agent add`, then retry"
            )
        })
}

/// Project every ladder row into a scored candidate; count rows whose
/// URA does not parse instead of dropping them silently.
fn project_rows(value: &Value, tokens: &[String], skipped: &mut usize) -> Vec<Candidate> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| match Candidate::from_ladder_row(row, tokens) {
                    Ok(candidate) => candidate,
                    Err(()) => {
                        *skipped += 1;
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
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

    // ── Ranking contract (frozen; moved verbatim from ability_search) ──

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

    // ── Ladder-row projection ──────────────────────────────────────────

    /// Fixture URAs come from the Axon builders re-exported through
    /// `crate::ura` — never hand-written literals (spec §0.1-8).
    fn agent_row(realm: &str) -> Value {
        json!({
            "qualified_name": crate::ura::ability_ura(realm, "user-1", "codex", "chat"),
            "owner": "codex",
            "ability": "chat",
            "description": "chat with the codex agent",
            "scope_matched": "self",
        })
    }

    fn device_row(realm: &str) -> Value {
        json!({
            "qualified_name": crate::ura::device_ability_ura(realm, "device-9", "fs.read"),
            "owner": "device-9",
            "ability": "fs.read",
            "description": "read a file from disk",
            "scope_matched": "device",
        })
    }

    #[test]
    fn ladder_row_projects_owner_kind_from_the_parsed_ura() {
        let toks = vec!["chat".to_string()];
        let c = Candidate::from_ladder_row(&agent_row("acme"), &toks)
            .unwrap()
            .expect("scores > 0");
        assert_eq!(c.owner_kind, "agent");
        assert_eq!(c.scope, "self");
        assert!(c.trust_level.is_none(), "trust is W2; must be null today");

        let toks = vec!["read".to_string(), "file".to_string()];
        let c = Candidate::from_ladder_row(&device_row("acme"), &toks)
            .unwrap()
            .expect("scores > 0");
        assert_eq!(c.owner_kind, "device");
    }

    #[test]
    fn ladder_row_with_non_canonical_ura_is_counted_not_silent() {
        let toks = vec!["read".to_string()];
        let row = json!({
            "qualified_name": "not-a-ura",
            "owner": "x", "ability": "read", "description": "read things",
        });
        assert!(Candidate::from_ladder_row(&row, &toks).is_err());

        let mut skipped = 0;
        let out = project_rows(&json!({ "candidates": [row] }), &toks, &mut skipped);
        assert!(out.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn zero_score_ladder_rows_drop_without_counting_as_unparseable() {
        let toks = vec!["weather".to_string()];
        let mut skipped = 0;
        let out = project_rows(
            &json!({ "candidates": [device_row("acme")] }),
            &toks,
            &mut skipped,
        );
        assert!(out.is_empty());
        assert_eq!(skipped, 0);
    }

    // ── Federation degradation envelope ────────────────────────────────

    #[test]
    fn federation_error_envelope_projects_to_typed_status() {
        let envelope = json!({
            "candidates": [], "scope": "user", "query": null,
            "error": { "code": "federation_not_joined", "message": "no credentials", "retriable": false }
        });
        let status = FederationStatus::from_envelope(&envelope).expect("typed envelope");
        assert_eq!(status.status, "federation_not_joined");

        let ok = json!({ "candidates": [], "scope": "user", "query": null });
        assert!(FederationStatus::from_envelope(&ok).is_none());
    }

    // ── JSON contract freeze (spec §0.2-9: asserted names never change) ──

    #[test]
    fn json_contract_field_names_are_frozen() {
        use std::collections::BTreeSet;

        let toks = vec!["chat".to_string()];
        let mut skipped = 0;
        let candidates = project_rows(
            &json!({ "candidates": [agent_row("acme")] }),
            &toks,
            &mut skipped,
        );
        let report = DiscoverReport {
            query: "chat".into(),
            tiers_searched: vec!["self", "device"],
            federation: Some(FederationStatus {
                status: "federation_not_joined".into(),
                message: "no credentials".into(),
            }),
            invocation: None,
            skipped_unparseable: 0,
            candidates,
        };
        let v = serde_json::to_value(&report).expect("serializes");

        let top: BTreeSet<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            top,
            BTreeSet::from(["query", "tiers_searched", "federation", "candidates"]),
            "skip-if-absent fields must vanish when empty; asserted names are frozen"
        );

        let row: BTreeSet<&str> = v["candidates"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            row,
            BTreeSet::from([
                "score",
                "name",
                "ura",
                "owner_kind",
                "scope",
                "trust_level",
                "description"
            ])
        );
        assert!(
            v["candidates"][0]["trust_level"].is_null(),
            "trust column is present-and-null until W2 wires identity.get_trust"
        );
        assert_eq!(v["federation"]["status"], "federation_not_joined");
    }

    // ── Tree projection (display only — spec §3.6) ─────────────────────

    #[test]
    fn tree_groups_by_owner_and_preserves_score_order_within_groups() {
        let toks = vec!["read".to_string(), "chat".to_string()];
        let mut skipped = 0;
        let candidates = project_rows(
            &json!({ "candidates": [agent_row("acme"), device_row("acme")] }),
            &toks,
            &mut skipped,
        );
        let report = DiscoverReport {
            query: "read chat".into(),
            tiers_searched: vec!["self", "device"],
            federation: None,
            invocation: None,
            skipped_unparseable: 0,
            candidates,
        };
        let groups = report.group_by_owner();
        assert_eq!(groups.len(), 2, "agent owner and device owner group apart");
        for rows in groups.values() {
            assert!(rows.windows(2).all(|w| w[0].score >= w[1].score));
        }
    }
}
