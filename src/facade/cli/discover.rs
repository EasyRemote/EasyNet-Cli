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
// The seven-tuple is auditable: each daemon discover call goes through
// `invoke_local_ability_with_invocation_meta`, and the envelope echoes
// (caller / callee / ability / subject / …) are included verbatim in
// `--format json` output as `invocations` (spec 0.1-7, W1-E2E-1 ⑤).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};

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
    /// Agent name whose own ladder should define the `self` tier.
    /// Omit for a device aggregate view with no implicit self agent.
    #[arg(long, value_name = "AGENT")]
    pub as_agent: Option<String>,
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
/// owner_kind / scope / description / callable / identity_state,
/// with diagnostic emitted only for non-callable rows.
#[derive(Debug, serde::Serialize)]
pub struct Candidate {
    pub score: u32,
    pub name: String,
    pub ura: String,
    pub owner_kind: &'static str,
    pub scope: String,
    pub description: String,
    pub callable: bool,
    pub identity_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
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
            .unwrap_or_default();
        let identity_state = row
            .get("identity_state")
            .and_then(Value::as_str)
            .unwrap_or("minted");
        let owner = row.get("owner").and_then(Value::as_str).unwrap_or_default();
        let selector = if ura.is_empty() {
            None
        } else {
            Some(AbilitySelector::parse(ura).map_err(|_| ())?)
        };
        let ability = row
            .get("ability")
            .and_then(Value::as_str)
            .or_else(|| selector.as_ref().map(AbilitySelector::public_name))
            .unwrap_or_default();
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
        let owner_signal = if ura.is_empty() { owner } else { ura };
        let score = score_candidate(tokens, &name, description, Some(owner_signal));
        if score == 0 {
            return Ok(None);
        }
        let mut diagnostic = row
            .get("diagnostic")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (owner_kind, owner_ura, callable) = match selector.as_ref() {
            Some(selector) => {
                let callable = row.get("callable").and_then(Value::as_bool);
                if callable.is_none() {
                    diagnostic.get_or_insert_with(|| {
                        "callable status missing from discovery row; treating as non-callable"
                            .to_string()
                    });
                }
                (
                    selector.owner_kind(),
                    selector.owner_ura().to_string(),
                    callable.unwrap_or(false),
                )
            }
            None if identity_state != "minted" => {
                ("agent", format!("unminted-agent:{owner}"), false)
            }
            None => return Err(()),
        };
        Ok(Some(Candidate {
            score,
            name,
            ura: ura.to_string(),
            owner_kind,
            scope: scope.to_string(),
            description: description.to_string(),
            callable,
            identity_state: identity_state.to_string(),
            diagnostic,
            owner_ura,
        }))
    }
}

/// Typed projection of the ladder's federation error envelope.
#[derive(Debug, serde::Serialize)]
pub struct FederationStatus {
    pub status: String,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DiscoverDiagnostic {
    pub scope: String,
    pub code: &'static str,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalSelfProjection {
    Preserve,
    DeviceAggregate,
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
    /// Envelope echoes of every daemon discover invocation in execution order
    /// (seven-tuple audit surface; spec 0.1-7).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invocations: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiscoverDiagnostic>,
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
        for diagnostic in &self.diagnostics {
            eprintln!(
                "{}",
                style(format!(
                    "note: discover {} ({}): {}",
                    diagnostic.scope, diagnostic.code, diagnostic.message
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
            style("invoke callable rows with: easynet ability invoke <URA> [args…]").dim()
        );
    }

    fn render_table(&self) {
        println!(
            "{:>5}  {:<34} {:<7} {:<38} {}",
            style("SCORE").bold(),
            style("ABILITY").bold(),
            style("TIER").bold(),
            style("URA").bold(),
            style("DESCRIPTION").bold()
        );
        for c in &self.candidates {
            let ura = if c.callable {
                truncate(&c.ura, 38)
            } else {
                format!("not callable: {}", c.identity_state)
            };
            println!(
                "{:>5}  {:<34} {:<7} {:<38} {}",
                c.score,
                truncate(&c.name, 34),
                c.scope,
                truncate(&ura, 38),
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
                    if c.callable {
                        c.scope.as_str()
                    } else {
                        "unminted"
                    },
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
    DiscoverRuntimeService::new(args).execute()
}

struct DiscoverRuntimeService<'a> {
    args: &'a DiscoverArgs,
    tokens: Vec<String>,
    self_projection: LocalSelfProjection,
    ladder: String,
    skipped: usize,
    invocations: Vec<Value>,
    diagnostics: Vec<DiscoverDiagnostic>,
}

impl<'a> DiscoverRuntimeService<'a> {
    fn new(args: &'a DiscoverArgs) -> Self {
        Self {
            args,
            tokens: Vec::new(),
            self_projection: LocalSelfProjection::DeviceAggregate,
            ladder: String::new(),
            skipped: 0,
            invocations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn execute(mut self) -> anyhow::Result<DiscoverReport> {
        if self.args.limit == 0 {
            anyhow::bail!("limit must be positive; omit it or pass a value greater than zero");
        }
        self.tokens = tokenize(&self.args.intent);
        if self.tokens.is_empty() {
            anyhow::bail!("intent is empty after tokenization; describe what you want done");
        }

        self.self_projection = if self.args.as_agent.as_deref().is_some() {
            LocalSelfProjection::Preserve
        } else {
            LocalSelfProjection::DeviceAggregate
        };
        self.ladder = resolve_ladder_entry(self.args.as_agent.as_deref())?;

        let (local_value, invocation_meta) = self.walk_tier("device", &[])?;
        self.invocations.push(invocation_meta);
        self.record_source_diagnostic(&local_value);
        let mut candidates = project_rows(
            &local_value,
            &self.tokens,
            &mut self.skipped,
            self.self_projection,
        );
        let mut tiers_searched = if self.self_projection == LocalSelfProjection::Preserve {
            vec!["self", "device"]
        } else {
            vec!["device"]
        };

        let mut federation = None;
        if !self.args.local_only {
            let local_invocation_meta = self
                .invocations
                .last()
                .cloned()
                .context("local discover tier did not produce invocation metadata")?;
            let causal_parents =
                self.realm_causal_parents_from_local_invocation(&local_invocation_meta)?;
            let trace_id = self
                .invocations
                .first()
                .and_then(|meta| meta.get("trace_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let (realm_value, realm_invocation_meta) =
                self.walk_tier_with_trace("user", causal_parents.as_slice(), trace_id)?;
            self.invocations.push(realm_invocation_meta);
            self.record_source_diagnostic(&realm_value);
            match FederationStatus::from_envelope(&realm_value) {
                Some(status) => federation = Some(status),
                None => {
                    candidates.extend(project_rows(
                        &realm_value,
                        &self.tokens,
                        &mut self.skipped,
                        LocalSelfProjection::Preserve,
                    ));
                    tiers_searched.push("user");
                }
            }
        }

        candidates = rank_and_deduplicate_candidates(candidates, self.args.limit);

        Ok(DiscoverReport {
            query: self.args.intent.clone(),
            tiers_searched,
            federation,
            invocations: self.invocations,
            diagnostics: self.diagnostics,
            skipped_unparseable: self.skipped,
            candidates,
        })
    }

    fn walk_tier(
        &self,
        scope: &'static str,
        causal_parents: &[Value],
    ) -> anyhow::Result<(Value, Value)> {
        self.walk_tier_with_trace(scope, causal_parents, None)
    }

    fn walk_tier_with_trace(
        &self,
        scope: &'static str,
        causal_parents: &[Value],
        trace_id: Option<&str>,
    ) -> anyhow::Result<(Value, Value)> {
        invoke_local_ability_with_invocation_meta(
            &self.ladder,
            json!({ "scope": scope, "source_window": "all" }),
            None,
            causal_parents,
            None,
            trace_id,
            self.args.as_agent.as_deref(),
        )
        .with_context(|| format!("walk discover tier {scope:?}"))
    }

    fn record_source_diagnostic(&mut self, value: &Value) {
        let Some(source) = value.get("source") else {
            return;
        };
        let truncated = source
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !truncated {
            return;
        }
        let scope = value
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let available = source
            .get("available")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let limit = source
            .get("limit")
            .map(|value| match value {
                Value::Null => "all".to_string(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "unknown".to_string());
        self.diagnostics.push(DiscoverDiagnostic {
            scope,
            code: "source_truncated",
            message: format!(
                "runtime returned {limit} of {available} source candidates before CLI ranking"
            ),
        });
    }

    fn realm_causal_parents_from_local_invocation(
        &self,
        meta: &Value,
    ) -> anyhow::Result<Vec<Value>> {
        let parent = receipt_parent_from_invocation_meta(meta).map_err(|err| {
            anyhow::anyhow!(
                "{err}; refusing realm discovery without the local receipt anchor causal parent"
            )
        })?;
        Ok(vec![parent])
    }
}

fn receipt_parent_from_invocation_meta(meta: &Value) -> anyhow::Result<Value> {
    let anchor = meta
        .get("receipt")
        .and_then(|receipt| receipt.get("anchor"))
        .ok_or_else(|| anyhow::anyhow!("invocation metadata is missing receipt.anchor"))?;
    let receipt_ura = anchor
        .get("receipt_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("invocation metadata receipt.anchor is missing receipt_ura")
        })?;
    let receipt_hash = anchor
        .get("receipt_hash")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("invocation metadata receipt.anchor is missing receipt_hash")
        })?;
    Ok(json!({
        "receipt_ura": receipt_ura,
        "receipt_hash": receipt_hash,
    }))
}

fn rank_and_deduplicate_candidates(mut candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
    // Highest score first; ties resolve by name, then URA, so output
    // is stable. One ability appears once: the URA is the identity.
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.ura.cmp(&b.ura))
    });
    let mut seen_uras = BTreeSet::new();
    candidates.retain(|candidate| {
        let identity = if candidate.ura.is_empty() {
            format!("{}:{}", candidate.owner_ura, candidate.name)
        } else {
            candidate.ura.clone()
        };
        seen_uras.insert(identity)
    });
    candidates.truncate(limit);
    candidates
}

/// Resolve the daemon's discover entry point.
///
/// Default top-level discovery uses the daemon-owned aggregate discover
/// entry. `--as-agent` intentionally opts into one concrete
/// `<agent>.discover` self tier and is validated against `agent.list`.
fn resolve_ladder_entry(as_agent: Option<&str>) -> anyhow::Result<String> {
    let Some(requested_agent) = as_agent.map(str::trim).filter(|agent| !agent.is_empty()) else {
        return Ok(crate::runtime::agents::discover_ability::DEVICE_DISCOVER_ABILITY.to_string());
    };
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
    if !names.iter().any(|name| name == requested_agent) {
        anyhow::bail!(
            "agent {requested_agent:?} is not registered on this daemon; choose one from \
             `easynet agent list`"
        );
    }
    Ok(format!(
        "{requested_agent}.{}",
        crate::runtime::agents::discover_ability::ABILITY_VERB
    ))
}

/// Project every ladder row into a scored candidate; count rows whose
/// URA does not parse instead of dropping them silently.
fn project_rows(
    value: &Value,
    tokens: &[String],
    skipped: &mut usize,
    self_projection: LocalSelfProjection,
) -> Vec<Candidate> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let row = normalize_self_projection(row, self_projection)?;
                    match Candidate::from_ladder_row(&row, tokens) {
                        Ok(candidate) => candidate,
                        Err(()) => {
                            *skipped += 1;
                            None
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_self_projection(row: &Value, projection: LocalSelfProjection) -> Option<Value> {
    if projection == LocalSelfProjection::Preserve {
        return Some(row.clone());
    }
    if row.get("scope_matched").and_then(Value::as_str) != Some("self") {
        return Some(row.clone());
    }
    if row.get("visibility").and_then(Value::as_str) == Some("self") {
        return None;
    }
    let mut normalized = row.clone();
    if let Some(object) = normalized.as_object_mut() {
        object.insert("scope_matched".to_string(), json!("device"));
    }
    Some(normalized)
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

    #[test]
    fn execute_rejects_zero_limit_before_daemon_invocation() {
        let err = execute(&DiscoverArgs {
            intent: "read file".to_string(),
            limit: 0,
            local_only: true,
            as_agent: None,
            tree: false,
            format: OutputFormat::Table,
        })
        .unwrap_err();

        assert!(err.to_string().contains("limit must be positive"), "{err}");
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
            "callable": true,
        })
    }

    fn device_row(realm: &str) -> Value {
        json!({
            "qualified_name": crate::ura::device_ability_ura(realm, "device-9", "fs.read"),
            "owner": "device-9",
            "ability": "fs.read",
            "description": "read a file from disk",
            "scope_matched": "device",
            "callable": true,
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
        let out = project_rows(
            &json!({ "candidates": [row] }),
            &toks,
            &mut skipped,
            LocalSelfProjection::Preserve,
        );
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
            LocalSelfProjection::Preserve,
        );
        assert!(out.is_empty());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn unminted_identity_rows_are_not_counted_as_unparseable() {
        let toks = vec!["weather".to_string()];
        let mut skipped = 0;
        let out = project_rows(
            &json!({
                "candidates": [{
                    "qualified_name": "",
                    "owner": "apprentice",
                    "ability": "weather",
                    "description": "weather lookup",
                    "scope_matched": "device",
                    "identity_state": "identity_not_minted",
                    "callable": false,
                    "diagnostic": "agent has no hosted URA"
                }]
            }),
            &toks,
            &mut skipped,
            LocalSelfProjection::Preserve,
        );
        assert_eq!(skipped, 0);
        assert_eq!(out.len(), 1);
        assert!(!out[0].callable);
        assert_eq!(out[0].identity_state, "identity_not_minted");
    }

    #[test]
    fn minted_rows_without_callable_are_fail_closed() {
        let toks = vec!["chat".to_string()];
        let mut row = agent_row("acme");
        row.as_object_mut().expect("row object").remove("callable");

        let c = Candidate::from_ladder_row(&row, &toks)
            .unwrap()
            .expect("scores > 0");

        assert!(!c.callable, "missing callability must not fail open");
        assert_eq!(
            c.diagnostic.as_deref(),
            Some("callable status missing from discovery row; treating as non-callable")
        );
    }

    #[test]
    fn source_truncation_is_reported_before_cli_ranking() {
        let args = DiscoverArgs {
            intent: "chat".to_string(),
            limit: 5,
            local_only: true,
            as_agent: None,
            tree: false,
            format: OutputFormat::Json,
        };
        let mut service = DiscoverRuntimeService::new(&args);
        service.record_source_diagnostic(&json!({
            "scope": "device",
            "source": {
                "available": 12000,
                "returned": 20,
                "limit": 20,
                "truncated": true,
            }
        }));

        assert_eq!(service.diagnostics.len(), 1);
        assert_eq!(service.diagnostics[0].code, "source_truncated");
        assert!(service.diagnostics[0].message.contains("12000"));
    }

    #[test]
    fn missing_local_receipt_anchor_refuses_realm_walk() {
        let args = DiscoverArgs {
            intent: "chat".to_string(),
            limit: 5,
            local_only: false,
            as_agent: None,
            tree: false,
            format: OutputFormat::Json,
        };
        let service = DiscoverRuntimeService::new(&args);
        let err = service
            .realm_causal_parents_from_local_invocation(&json!({
                "metadata_state": "missing_receipt_anchor",
                "trace_id": "trace-1",
            }))
            .expect_err("realm discovery must not run without local receipt anchor");

        assert!(
            format!("{err}").contains("refusing realm discovery without the local receipt anchor"),
            "{err}"
        );
        assert!(service.diagnostics.is_empty());
    }

    #[test]
    fn rank_and_deduplicate_removes_non_adjacent_duplicate_uras() {
        fn candidate(score: u32, name: &str, ura: &str) -> Candidate {
            Candidate {
                score,
                name: name.to_string(),
                ura: ura.to_string(),
                owner_kind: "agent",
                scope: "device".to_string(),
                description: String::new(),
                callable: true,
                identity_state: "minted".to_string(),
                diagnostic: None,
                owner_ura: "owner".to_string(),
            }
        }

        let out = rank_and_deduplicate_candidates(
            vec![
                candidate(
                    10,
                    "alpha.chat",
                    "easynet:///r/acme/user/u/agent/a/ability/chat",
                ),
                candidate(9, "beta.read", "easynet:///r/acme/device/d/ability/read"),
                candidate(
                    8,
                    "gamma.chat",
                    "easynet:///r/acme/user/u/agent/a/ability/chat",
                ),
            ],
            10,
        );

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].score, 10);
        assert_eq!(out[0].name, "alpha.chat");
        assert_eq!(out[1].name, "beta.read");
    }

    #[test]
    fn device_aggregate_projection_does_not_invent_self_identity() {
        let toks = vec!["chat".to_string()];
        let mut public_self = agent_row("acme");
        public_self["visibility"] = json!("public");
        let private_self = json!({
            "qualified_name": crate::ura::ability_ura("acme", "user-1", "codex", "private_chat"),
            "owner": "codex",
            "ability": "private_chat",
            "description": "chat privately",
            "scope_matched": "self",
            "visibility": "self",
        });

        let mut skipped = 0;
        let out = project_rows(
            &json!({ "candidates": [public_self, private_self] }),
            &toks,
            &mut skipped,
            LocalSelfProjection::DeviceAggregate,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scope, "device");
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

    #[test]
    fn receipt_parent_projection_requires_receipt_anchor() {
        let backed = json!({
            "receipt": {
                "anchor": {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/req-1",
                    "receipt_hash": "abc123",
                }
            },
            "metadata_state": "receipt_backed",
        });
        assert_eq!(
            receipt_parent_from_invocation_meta(&backed).expect("receipt parent"),
            json!({
                "receipt_ura": "easynet:///r/acme/resource/invocations/req-1",
                "receipt_hash": "abc123",
            })
        );

        let missing_anchor = json!({ "metadata_state": "missing_receipt_anchor" });
        let err = receipt_parent_from_invocation_meta(&missing_anchor).unwrap_err();
        assert!(
            err.to_string().contains("receipt.anchor"),
            "missing receipt metadata must fail before a child invocation is built: {err}"
        );
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
            LocalSelfProjection::Preserve,
        );
        let report = DiscoverReport {
            query: "chat".into(),
            tiers_searched: vec!["self", "device"],
            federation: Some(FederationStatus {
                status: "federation_not_joined".into(),
                message: "no credentials".into(),
            }),
            invocations: Vec::new(),
            diagnostics: Vec::new(),
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
                "description",
                "callable",
                "identity_state",
            ])
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
            LocalSelfProjection::Preserve,
        );
        let report = DiscoverReport {
            query: "read chat".into(),
            tiers_searched: vec!["self", "device"],
            federation: None,
            invocations: Vec::new(),
            diagnostics: Vec::new(),
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
