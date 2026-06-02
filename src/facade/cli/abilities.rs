// EasyNet CLI — `easynet ability list`
// ======================================
//
// File: src/facade/cli/abilities.rs
// Description: List the abilities reachable on this node — the
//              union of system / device-level abilities and every
//              registered agent's owned ability set.
//
// Why this command exists alongside `easynet agent abilities <name>`
// ---------------------------------------------------------------
// Two scopes, two commands. `agent abilities <name>` is the
// owner-side view: "what does CLAUDE own?". This command is the
// caller-side view: "what can I invoke on this device?". The two
// surface different mental models of the same registry — one
// centered on the agent, one centered on the node — and a CLI that
// only ships the first leaves a caller without a one-shot
// "everything I can call" listing. `--agent <name>` collapses this
// command into the owner-side view, so the two commands stay
// equivalent under that filter.
//
// Routing post AXON-RFC-001 P1.5
// -------------------------------
// Pre-rewrite this file called
// `bridge.list_mcp_tools(tenant, pattern, scope)`, the federation
// RPC removed by AXON-RFC-001 P1.5. Every `easynet ability list`
// invocation therefore hit
//
//     bridge: list_mcp_tools removed by AXON-RFC-001 P1.5; use
//     Invoke against the appropriate Agent ability
//
// regardless of `--node`. The replacement: invoke the daemon-hosted
// metadata ability through Axon's local Invocation gRPC path, the
// same route `easynet ability invoke` uses for local calls. One
// Axon runtime; one source of truth for the catalogue. The `--node`
// flag is reserved for a future federation-Invoke entry; passing a
// remote node id today returns a precise error rather than silently
// auto-routing local.
//
// Filtering model
// ---------------
//   --agent <name>     Only abilities owned by `<name>` (i.e. names
//                      with the prefix `<name>.`). Equivalent to
//                      `easynet agent abilities <name>`.
//   --pattern <glob>   Glob filter on the fully qualified name. `*`
//                      matches anything but `.`; `**` matches across
//                      `.` boundaries.
//   --format <f>       `table` (human) | `json` (raw catalogue).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::bail;
#[cfg(feature = "axon-pb")]
use anyhow::Context;
use clap::Args;
use console::{measure_text_width, style};
use serde_json::Value;

use crate::support::local_invoke::invoke_local_ability;
use crate::support::output::{self, OutputFormat};
use crate::ura::{parse_ura, URAKind};

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Filter to a single agent's owned abilities. Equivalent to 'easynet agent abilities <name>'.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Reserved for federation routing — only the local node is accepted today; remote listing ships post-AXON-RFC-001 P1.5.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Glob pattern to filter by ability name (e.g. fs.*, claude.*, *.health). Empty pattern is equivalent to omitting the flag.
    #[arg(long, default_value = "")]
    pub pattern: String,
    /// Output format — table (human, default) or json (raw catalogue, jq-friendly).
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: AbilitiesArgs) -> anyhow::Result<()> {
    // Joint-plan unified path: `--node` is now wired through
    // `federation.forward_invoke` against the target device URA.
    // Each daemon's `easynet.discover` ability returns its OWN
    // catalogue; cross-device discovery is the caller's job
    // (forward_invoke routes through the target's daemon).
    let abilities = match args.node.as_deref().map(str::trim) {
        None | Some("local") => fetch_local_catalogue()?,
        Some("") => bail!(
            "--node was given but empty; omit the flag to list local abilities, \
             or pass `easynet:///r/<realm>/device/<id>` to list a peer device's \
             catalogue."
        ),
        Some(node) => fetch_remote_catalogue(node)?,
    };
    let filtered = filter_abilities(abilities, args.agent.as_deref(), &args.pattern)?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        let scope = scope_label(args.agent.as_deref(), &args.pattern);
        output::warn(&format!(
            "no abilities matched {scope} on the local node. Use \
             `easynet ability list --format json` to see the full catalogue, \
             or `easynet agent list` to see which agents are registered."
        ));
        return Ok(());
    }

    // Grouped table: one section per owner kind (Hub / Agent /
    // Device / User), each headed by the canonical owner URA.
    // The triple (DEVICE, AGENT, USER) is encoded by the section,
    // so per-row we only need ABILITY + KIND + DESCRIPTION. This
    // matches the URA ontology — abilities partition cleanly by
    // owner kind, never mix — and reads as a tree: the operator
    // sees the realm hub's published surface, then per-agent
    // surfaces, then the device-local registry.
    render_grouped(&filtered);
    Ok(())
}

/// Build owned (Device, Agent, User, Kind) cells for one ability
/// entry. The three identity columns are projections of the
/// owner URA — only the slots that the owner kind actually
/// names get populated; the rest stay `-`.
///
/// URA kind → which columns are meaningful (per AXON-RFC-001
/// v4.1.5 §A.URA):
///
///   `device/<id>`      → DEVICE only.
///   `agent/<u>.<a>`    → AGENT + USER. The agent's host device
///                        is a *separate* edge (`local-agents.json`
///                        records it), not part of the agent URA,
///                        so DEVICE stays `-`.
///   `hub`              → AGENT only ("hub", realm-singleton).
///                        Hub is not on any device.
///   any other / parse-fail → all dashes.
///
/// Past iterations of this function filled DEVICE with the
/// scope-device id (the daemon being queried) for hub / agent
/// rows. That mixed two different facts — "who owns this verb"
/// vs "which daemon's catalogue am I reading" — and produced
/// self-contradictory rows like
/// `01HUB.openai.chat_completions  device=99e59cc…  agent=hub`
/// (says it's both on the device and on the hub). The owner URA
/// is the only authority for the identity columns; the calling
/// daemon's id, when relevant, belongs in a separate context
/// line, not in the per-row identity columns.
fn extract_columns(entry: &Value) -> (String, String, String, String) {
    let owner_ura = entry
        .get("owner_agent_ura")
        .and_then(Value::as_str)
        .unwrap_or("");
    let parsed = parse_ura(owner_ura).ok();

    // KIND is read straight from the owner URA kind — that's the
    // authoritative classifier. The legacy `fulfilled_by`
    // descriptor field still wins when present (handlers that
    // explicitly tag themselves, e.g. `mcp_proxy`); when absent
    // we fall back to the owner-kind label rather than guessing
    // from the ability name. The pre-migration default was
    // `agent_chat`, which mis-labelled every device-owned and
    // user-owned verb.
    let kind = entry
        .get("fulfilled_by")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| match parsed.as_ref().map(|p| p.kind) {
            Some(URAKind::Device) => "system".to_string(),
            Some(URAKind::Hub) => "hub".to_string(),
            Some(URAKind::Agent) => "agent".to_string(),
            Some(URAKind::User) => "user".to_string(),
            _ => "-".to_string(),
        });

    let dash = || "-".to_string();
    let (device, agent, user) = match parsed {
        Some(p) => match p.kind {
            URAKind::Device => (p.device_id, dash(), dash()),
            URAKind::Agent => (dash(), p.agent_id, p.user_id),
            URAKind::User => (dash(), dash(), p.user_id),
            URAKind::Hub => (dash(), "hub".to_string(), dash()),
            _ => (dash(), dash(), dash()),
        },
        // Unparseable owner URA. We do not invent owner kinds from
        // the ability-name namespace — the daemon's synth path
        // (`meta_ability::list_abilities_handler`) is responsible
        // for emitting a parseable URA, and a row that surfaces
        // here represents real catalogue corruption (e.g. a hosted
        // agent persisted with a non-canonical URA shape). Render
        // dashes so the row is visible, but do not paper over the
        // underlying defect with namespace-derived stand-ins.
        None => (dash(), dash(), dash()),
    };

    (device, agent, user, kind)
}

/// Owner-kind classification used to group abilities under a
/// labelled section. Order matches render order — Hub first
/// (realm-published surface), then per-agent + per-user
/// surfaces, then the device-local registry, with `Other`
/// reserved for parse failures so a corrupt URA still surfaces
/// rather than vanishing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum GroupKey {
    Hub(String),
    Agent {
        user: String,
        agent: String,
        ura: String,
    },
    User {
        user: String,
        ura: String,
    },
    Device(String),
    Other,
}

impl GroupKey {
    fn header(&self) -> String {
        match self {
            GroupKey::Hub(ura) => format!("HUB ({ura})"),
            GroupKey::Agent { user, agent, ura } => {
                format!("AGENT {user}.{agent} ({ura})")
            }
            GroupKey::User { user, ura } => format!("USER {user} ({ura})"),
            GroupKey::Device(ura) => format!("DEVICE / SYSTEM ({ura})"),
            GroupKey::Other => "OTHER".to_string(),
        }
    }

    /// Section ordering. Lower = printed first.
    fn section_order(&self) -> u8 {
        match self {
            GroupKey::Hub(_) => 0,
            GroupKey::Agent { .. } => 1,
            GroupKey::User { .. } => 2,
            GroupKey::Device(_) => 3,
            GroupKey::Other => 4,
        }
    }
}

fn group_for(entry: &Value) -> GroupKey {
    let owner_ura = entry
        .get("owner_agent_ura")
        .and_then(Value::as_str)
        .unwrap_or("");
    match parse_ura(owner_ura) {
        Ok(p) => match p.kind {
            URAKind::Hub => GroupKey::Hub(owner_ura.to_string()),
            URAKind::Agent => GroupKey::Agent {
                user: p.user_id,
                agent: p.agent_id,
                ura: owner_ura.to_string(),
            },
            URAKind::User => GroupKey::User {
                user: p.user_id,
                ura: owner_ura.to_string(),
            },
            URAKind::Device => GroupKey::Device(owner_ura.to_string()),
            _ => GroupKey::Other,
        },
        Err(_) => GroupKey::Other,
    }
}

fn render_grouped(filtered: &[Value]) {
    use std::collections::BTreeMap;

    // Bucket entries by owner-kind group. BTreeMap keeps the
    // sections in a stable order (by section_order then by URA),
    // so two runs against the same daemon emit byte-identical
    // output — handy for diff-based catalogue review.
    let mut groups: BTreeMap<(u8, GroupKey), Vec<&Value>> = BTreeMap::new();
    for entry in filtered {
        let g = group_for(entry);
        groups
            .entry((g.section_order(), g))
            .or_default()
            .push(entry);
    }

    let term_width = console::Term::stderr().size().1 as usize;
    let headers = ["ABILITY", "KIND", "DESCRIPTION"];

    eprintln!();
    for ((_, key), entries) in &groups {
        // Section header: bold colored title with the canonical
        // owner URA so the operator can copy/paste it into a
        // cross-device invoke or share it with a peer.
        let title = key.header();
        let header_style = match key {
            GroupKey::Hub(_) => style(&title).magenta().bold(),
            GroupKey::Agent { .. } => style(&title).green().bold(),
            GroupKey::User { .. } => style(&title).yellow().bold(),
            GroupKey::Device(_) => style(&title).blue().bold(),
            GroupKey::Other => style(&title).red().bold(),
        };
        eprintln!("  {header_style}");

        // Per-section column widths: ability + kind only;
        // description reflows against the terminal so long
        // single-line descriptions don't wrap mid-row.
        let mut rows: Vec<[String; 3]> = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string();
            let (_d, _a, _u, kind) = extract_columns(entry);
            let description = entry
                .get("description")
                .and_then(Value::as_str)
                .map(|s| s.lines().next().unwrap_or(s).to_string())
                .unwrap_or_default();
            rows.push([name, kind, description]);
        }
        let widths = column_widths(&headers, &rows);
        let total_fixed: usize = widths[..2].iter().sum::<usize>() + 2 * 2; // 2-space gutters
        let desc_budget = term_width
            .saturating_sub(4 + total_fixed) // leading 4-space indent for grouped rows
            .max(20);

        // Group-local header row + rule. Indented one extra step
        // beyond the section title so the visual hierarchy reads
        // clearly even on a narrow terminal.
        eprintln!(
            "    {}  {}  {}",
            style(pad(headers[0], widths[0])).dim(),
            style(pad(headers[1], widths[1])).dim(),
            style(headers[2]).dim(),
        );
        let rule_width: usize = (widths[..2].iter().sum::<usize>() + 2 * 2 + desc_budget)
            .min(term_width.saturating_sub(4).max(40));
        eprintln!("    {}", style("─".repeat(rule_width)).dim());

        for row in &rows {
            let desc = truncate_display(&row[2], desc_budget);
            eprintln!(
                "    {}  {}  {}",
                style(pad(&row[0], widths[0])).cyan(),
                style(pad(&row[1], widths[1])).dim(),
                desc,
            );
        }
        eprintln!();
    }
}

fn column_widths(headers: &[&str; 3], rows: &[[String; 3]]) -> [usize; 3] {
    let mut w = [0usize; 3];
    for (i, h) in headers.iter().enumerate() {
        w[i] = measure_text_width(h);
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            w[i] = w[i].max(measure_text_width(cell));
        }
    }
    w
}

fn pad(text: &str, width: usize) -> String {
    let w = measure_text_width(text);
    if w >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - w))
    }
}

fn truncate_display(text: &str, max: usize) -> String {
    if measure_text_width(text) <= max {
        return text.to_string();
    }
    // Truncate by char count, leaving room for the ellipsis. Falls
    // back to byte-safe slicing via `chars().take`.
    let limit = max.saturating_sub(1).max(1);
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// Invoke `easynet.discover` on the local daemon and return the
/// raw `abilities` array. Goes through the shared local-invoke
/// helper so daemon-down / IPC-failure / daemon-error rendering
/// stay byte-identical to every other CLI surface.
fn fetch_local_catalogue() -> anyhow::Result<Vec<Value>> {
    let value = invoke_local_ability("device.meta.list_abilities", serde_json::json!({}))?;
    extract_abilities(&value)
}

/// Joint-plan unified path: `easynet ability list --node <URA>`
/// forwards `easynet.discover` to the target device through
/// `federation.forward_invoke`. The peer daemon's local
/// `easynet.discover` handler runs and returns its own catalogue;
/// the forward bridge unwraps the response and we extract the
/// abilities array the same way `fetch_local_catalogue` does.
/// Bare UUID targets go through the shared cross-hub directory
/// lookup helper before falling back to the caller's local realm.
fn fetch_remote_catalogue(node: &str) -> anyhow::Result<Vec<Value>> {
    let value = invoke_remote_easynet_discover(node)?;
    extract_abilities(&value)
}

#[cfg(feature = "axon-pb")]
fn invoke_remote_easynet_discover(node: &str) -> anyhow::Result<Value> {
    let target_ura = crate::support::remote_device::resolve_target_device_ura(node)?;
    let caller_ura = crate::support::remote_device::caller_device_ura_from_credentials();
    crate::support::federation_invoke::invoke_via_federation_forward(
        "device.meta.list_abilities",
        serde_json::json!({}),
        &target_ura,
        caller_ura.as_deref(),
    )
    .with_context(|| format!("forward easynet.discover to target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_easynet_discover(node: &str) -> anyhow::Result<Value> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        &format!("listing abilities on remote node {node:?}"),
    ))
}

fn extract_abilities(value: &Value) -> anyhow::Result<Vec<Value>> {
    // The handler returns `{"abilities": [...]}`. Tolerate the
    // older spelling `tools` for forward-compat in case a follow-up
    // renames it; either way we extract a Vec<Value>.
    Ok(value
        .get("abilities")
        .or_else(|| value.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Apply `--agent` + `--pattern` filtering. Both are AND-composed:
/// `--agent claude --pattern '*.health'` keeps abilities owned by
/// `claude` AND whose name matches `*.health`. An empty pattern
/// matches everything (the default).
fn filter_abilities(
    abilities: Vec<Value>,
    agent: Option<&str>,
    pattern: &str,
) -> anyhow::Result<Vec<Value>> {
    let agent_prefix = agent.map(|name| format!("{name}."));
    let pattern_owned = if pattern.is_empty() {
        None
    } else {
        Some(pattern.to_string())
    };
    let out: Vec<Value> = abilities
        .into_iter()
        .filter(|entry| {
            let name = match entry.get("name").and_then(Value::as_str) {
                Some(n) => n,
                None => return false,
            };
            if let Some(prefix) = agent_prefix.as_deref() {
                if !name.starts_with(prefix) {
                    return false;
                }
            }
            if let Some(p) = pattern_owned.as_deref() {
                if !glob_match(p, name) {
                    return false;
                }
            }
            true
        })
        .collect();
    Ok(out)
}

/// Minimal glob matcher tailored to ability names. Supported:
/// * `*`  — matches any character except `.` (one verb segment).
/// * `**` — matches any character including `.` (multi-segment).
/// * `?`  — matches a single non-`.` character.
/// Everything else is a literal. We deliberately avoid pulling in
/// `regex` or `globset` for one CLI flag; the surrounding tests
/// pin the semantics so a future rewrite that swaps in a real
/// glob crate has to argue with the existing behaviour rather
/// than silently shift the matched set.
///
/// The implementation is the textbook recursive matcher: at each
/// star, we recurse on every legal partition of the remaining
/// text. Abilities catalogues are O(dozens) of entries with
/// patterns of a handful of characters, so the recursion depth
/// and big-O cost are both inconsequential. Recursion (vs the
/// backtracking pointer-shuffle commonly used for `fnmatch`-style
/// matchers) keeps the "single-star cannot cross `.`" rule
/// trivially correct: the recursion bails the moment the star's
/// consumed window steps over a `.`.
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pat: &[u8], text: &[u8]) -> bool {
    if pat.is_empty() {
        return text.is_empty();
    }
    if pat[0] == b'*' {
        // Distinguish `**` (crosses `.`) from `*` (does not).
        let (rest, double) = if pat.len() >= 2 && pat[1] == b'*' {
            (&pat[2..], true)
        } else {
            (&pat[1..], false)
        };
        // The star may consume zero characters → does the rest of
        // the pattern match the entire remaining text?
        if glob_match_inner(rest, text) {
            return true;
        }
        // …or one more character, recursively. Single-star is
        // forbidden from eating `.`; once we've consumed a `.`,
        // single-star recursion ends.
        let mut consumed = 0usize;
        while consumed < text.len() {
            let c = text[consumed];
            if !double && c == b'.' {
                // Single `*` cannot cross a `.`; later iterations
                // would only consume more dots.
                return false;
            }
            consumed += 1;
            if glob_match_inner(rest, &text[consumed..]) {
                return true;
            }
        }
        return false;
    }
    if text.is_empty() {
        return false;
    }
    let pc = pat[0];
    let tc = text[0];
    let lit_match = match pc {
        b'?' => tc != b'.',
        _ => pc == tc,
    };
    if !lit_match {
        return false;
    }
    glob_match_inner(&pat[1..], &text[1..])
}

#[cfg(test)]
fn split_qualified(name: &str) -> (&str, &str) {
    match name.split_once('.') {
        Some((head, rest)) => (head, rest),
        None => ("system", name),
    }
}

fn scope_label(agent: Option<&str>, pattern: &str) -> String {
    match (agent, pattern.is_empty()) {
        (None, true) => "anything".to_string(),
        (None, false) => format!("pattern {pattern:?}"),
        (Some(a), true) => format!("agent {a:?}"),
        (Some(a), false) => format!("agent {a:?} + pattern {pattern:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str) -> Value {
        json!({"name": name, "description": "stub", "fulfilled_by": "shell"})
    }

    #[test]
    fn filter_with_no_agent_no_pattern_passes_everything_through() {
        let xs = vec![entry("a.b"), entry("c.d"), entry("system.x")];
        let out = filter_abilities(xs.clone(), None, "").unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn filter_by_agent_keeps_only_owned_abilities() {
        let xs = vec![
            entry("claude.weather"),
            entry("claude.chat"),
            entry("codex.summarise"),
            entry("system.fs.read"),
        ];
        let out = filter_abilities(xs, Some("claude"), "").unwrap();
        let names: Vec<_> = out
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["claude.weather", "claude.chat"]);
    }

    #[test]
    fn filter_by_glob_pattern_with_single_star_does_not_cross_dot() {
        // `claude.*` matches one verb segment under claude only.
        // A regression that compiled `*` to `.*` would let
        // `claude.weather.x` slip through.
        let xs = vec![
            entry("claude.weather"),
            entry("claude.weather.fancy"),
            entry("codex.weather"),
        ];
        let out = filter_abilities(xs, None, "claude.*").unwrap();
        let names: Vec<_> = out
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["claude.weather"]);
    }

    #[test]
    fn filter_by_double_star_glob_crosses_dot_boundaries() {
        let xs = vec![entry("claude.a.b.c"), entry("claude.x"), entry("codex.x")];
        let out = filter_abilities(xs, None, "claude.**").unwrap();
        let names: Vec<_> = out
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["claude.a.b.c", "claude.x"]);
    }

    #[test]
    fn filter_combines_agent_and_pattern_with_and_semantics() {
        let xs = vec![
            entry("claude.weather"),
            entry("claude.chat"),
            entry("codex.weather"),
        ];
        let out = filter_abilities(xs, Some("claude"), "*.weather").unwrap();
        let names: Vec<_> = out
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["claude.weather"]);
    }

    #[test]
    fn split_qualified_separates_owner_and_verb_for_dotted_names() {
        assert_eq!(split_qualified("claude.weather"), ("claude", "weather"));
        assert_eq!(split_qualified("system.fs.read"), ("system", "fs.read"));
    }

    #[test]
    fn split_qualified_treats_a_bare_verb_as_a_system_ability() {
        // A name without a `.` cannot be agent-owned (the verb
        // separator is mandatory by the registry's invariants), so
        // we surface "system" as the owner column rather than a
        // misleading bare verb.
        assert_eq!(split_qualified("orphan-verb"), ("system", "orphan-verb"));
    }

    /// Joint-plan phase 1.5: `--node <other>` no longer hard-bails;
    /// it forwards `easynet.discover` to the target device URA
    /// through `federation.forward_invoke`. The forward path only
    /// exists in builds with `--features axon-pb`; without the
    /// feature `invoke_remote_easynet_discover` short-circuits to
    /// `federation_not_wired_error`, which is asserted by
    /// `run_with_remote_node_short_circuits_when_axon_pb_off` below.
    /// Splitting the two cases lets `cargo test --lib` (default
    /// features) AND `cargo test --features axon-pb` both pass.
    #[cfg(feature = "axon-pb")]
    #[test]
    fn run_with_remote_node_routes_through_forward_invoke() {
        // In a unit-test environment the local daemon UDS is absent,
        // so the call surfaces as either "daemon not running" /
        // "cannot resolve node ... without local credentials" /
        // "forward easynet.discover to target=...". The contract
        // this test pins is "remote node attempts the forward path"
        // — error message MUST mention either the forward target or
        // one of the resolution-stage errors so a script can grep
        // for it.
        let err = run(AbilitiesArgs {
            agent: None,
            node: Some("some-remote-node".into()),
            pattern: String::new(),
            format: OutputFormat::Table,
        })
        .expect_err("remote --node without daemon must surface a typed error");
        let msg = format!("{err}");
        assert!(
            msg.contains("forward")
                || msg.contains("daemon")
                || msg.contains("credentials")
                || msg.contains("federation"),
            "must mention forward / daemon / credentials / federation; got: {msg}"
        );
    }

    /// Counterpart to the test above — pins the no-feature build's
    /// behaviour: the `--node` path returns the "axon-pb required"
    /// error verbatim, with the offending action ("listing
    /// abilities on remote node ...") in front.
    #[cfg(not(feature = "axon-pb"))]
    #[test]
    fn run_with_remote_node_short_circuits_when_axon_pb_off() {
        let err = run(AbilitiesArgs {
            agent: None,
            node: Some("some-remote-node".into()),
            pattern: String::new(),
            format: OutputFormat::Table,
        })
        .expect_err("remote --node without axon-pb must surface a typed error");
        let msg = format!("{err}");
        assert!(
            msg.contains("axon-pb") && msg.contains("listing abilities on remote node"),
            "must point at the missing feature gate; got: {msg}"
        );
    }

    #[test]
    fn run_with_empty_node_string_is_caught_as_shell_expansion_accident() {
        let err = run(AbilitiesArgs {
            agent: None,
            node: Some("   ".into()),
            pattern: String::new(),
            format: OutputFormat::Table,
        })
        .expect_err("empty --node must be rejected");
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn group_for_buckets_each_owner_kind_into_its_section() {
        // Pin the partition: hub URA → Hub, agent URA → Agent,
        // user URA → User, device URA → Device. A future render
        // change that loses or merges a bucket trips this test.
        let hub = json!({
            "name": "hub.openai.chat_completions",
            "owner_agent_ura": "easynet:///r/easynet.run/hub",
        });
        let agent = json!({
            "name": "alice.codex.chat",
            "owner_agent_ura": "easynet:///r/easynet.run/agent/alice.codex",
        });
        let user = json!({
            "name": "alice.api_key.create",
            "owner_agent_ura": "easynet:///r/easynet.run/user/alice",
        });
        let device = json!({
            "name": "device.fs.read",
            "owner_agent_ura":
                "easynet:///r/easynet.run/device/00000000-0000-0000-0000-000000000001",
        });
        assert!(matches!(group_for(&hub), GroupKey::Hub(_)));
        assert!(matches!(group_for(&agent), GroupKey::Agent { .. }));
        assert!(matches!(group_for(&user), GroupKey::User { .. }));
        assert!(matches!(group_for(&device), GroupKey::Device(_)));
    }

    #[test]
    fn group_for_emits_other_for_unparseable_owner_ura() {
        let bad = json!({
            "name": "stray.thing",
            "owner_agent_ura": "not-a-ura",
        });
        assert!(matches!(group_for(&bad), GroupKey::Other));
    }

    #[test]
    fn group_section_order_matches_render_priority() {
        // Hub → Agent → User → Device → Other. Lower number prints
        // first.
        assert!(
            GroupKey::Hub("x".into()).section_order()
                < GroupKey::Agent {
                    user: "u".into(),
                    agent: "a".into(),
                    ura: "x".into()
                }
                .section_order()
        );
        assert!(
            GroupKey::Agent {
                user: "u".into(),
                agent: "a".into(),
                ura: "x".into()
            }
            .section_order()
                < GroupKey::User {
                    user: "u".into(),
                    ura: "x".into()
                }
                .section_order()
        );
        assert!(
            GroupKey::User {
                user: "u".into(),
                ura: "x".into()
            }
            .section_order()
                < GroupKey::Device("x".into()).section_order()
        );
        assert!(GroupKey::Device("x".into()).section_order() < GroupKey::Other.section_order());
    }
}
