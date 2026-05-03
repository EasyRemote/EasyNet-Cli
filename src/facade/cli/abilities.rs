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
// regardless of `--node`. The replacement: invoke `easynet.discover`
// — itself an ability registered on the local daemon's
// LocalAbilityRegistry — through the same IPC path
// `easynet ability invoke` uses. One dispatcher; one source of
// truth for the catalogue. The `--node` flag is reserved for a
// future federation-Invoke entry; passing a remote node id today
// returns a precise error rather than silently auto-routing local.
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

use anyhow::{bail, Context};
use clap::Args;
use serde_json::Value;

use crate::support::local_invoke::invoke_local_ability;
use crate::support::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Filter to a single agent's owned abilities. Equivalent to
    /// `easynet agent abilities <name>`. The filter matches
    /// abilities whose qualified name starts with `<agent>.`.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
    /// ⚠ Reserved for federation routing. `--node` may only be used
    /// today to pin to the local node; passing any other value
    /// surfaces a precise error pointing at the missing federation
    /// Invoke entry. Listing across nodes ships in a follow-up to
    /// AXON-RFC-001 P1.5.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Glob pattern to filter by ability name (e.g. `fs.*`,
    /// `claude.*`, `*.health`). The bare `--pattern ""` is equivalent
    /// to omitting the flag.
    #[arg(long, default_value = "")]
    pub pattern: String,
    /// Output format. `table` is the default human view; `json`
    /// emits the raw catalogue, suitable for piping into `jq`.
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

    // Table view: name, owner, kind, description (truncated). Owner
    // is the bare agent name when the qualified form is
    // `<agent>.<verb>`, "system" otherwise. Kind is `shell` /
    // `agent_chat` / `system` so an operator can see at a glance
    // whether an ability runs deterministically or via the LLM.
    let mut table = output::table(&["Ability", "Owner", "Kind", "Description"]);
    for entry in &filtered {
        let name = entry.get("name").and_then(Value::as_str).unwrap_or("-");
        let (owner, _verb) = split_qualified(name);
        let kind = entry
            .get("fulfilled_by")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if owner == "system" {
                    "system"
                } else {
                    "agent_chat"
                }
            });
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(|s| {
                let one_line = s.lines().next().unwrap_or(s);
                if one_line.len() > 78 {
                    format!("{}…", &one_line[..77])
                } else {
                    one_line.to_string()
                }
            })
            .unwrap_or_default();
        table.add_row(vec![name, owner, kind, &description]);
    }

    println!("{table}");
    Ok(())
}

/// Invoke `easynet.discover` on the local daemon and return the
/// raw `abilities` array. Goes through the shared local-invoke
/// helper so daemon-down / IPC-failure / daemon-error rendering
/// stay byte-identical to every other CLI surface.
fn fetch_local_catalogue() -> anyhow::Result<Vec<Value>> {
    let value = invoke_local_ability("easynet.discover", serde_json::json!({}))?;
    extract_abilities(&value)
}

/// Joint-plan unified path: `easynet ability list --node <URA>`
/// forwards `easynet.discover` to the target device through
/// `federation.forward_invoke`. The peer daemon's local
/// `easynet.discover` handler runs and returns its own catalogue;
/// the forward bridge unwraps the response and we extract the
/// abilities array the same way `fetch_local_catalogue` does.
fn fetch_remote_catalogue(node: &str) -> anyhow::Result<Vec<Value>> {
    let value = invoke_remote_easynet_discover(node)?;
    extract_abilities(&value)
}

#[cfg(feature = "axon-pb")]
fn invoke_remote_easynet_discover(node: &str) -> anyhow::Result<Value> {
    let target_uri = if node.starts_with("easynet:///r/") {
        crate::support::federation_invoke::parse_node_uri(node)?
    } else {
        let creds = crate::persistence::config::load_credentials().map_err(|_| {
            anyhow::anyhow!(
                "cannot resolve node {node:?}: pass a canonical \
                 `easynet:///r/<realm>/device/<id>` URI or pair this device first"
            )
        })?;
        crate::uri::device_uri(&creds.tenant_id, node)
    };
    let caller_uri = crate::persistence::config::load_credentials()
        .ok()
        .filter(|c| !c.tenant_id.trim().is_empty() && !c.node_id.trim().is_empty())
        .map(|c| crate::uri::device_uri(c.tenant_id.trim(), c.node_id.trim()));
    crate::support::federation_invoke::invoke_via_federation_forward(
        "easynet.discover",
        serde_json::json!({}),
        &target_uri,
        caller_uri.as_deref(),
    )
    .with_context(|| format!("forward easynet.discover to target={target_uri}"))
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

    #[test]
    fn run_with_remote_node_returns_a_precise_actionable_error() {
        // `--node <other>` MUST refuse rather than silently
        // dispatching locally — same contract as
        // `easynet ability invoke`. A script that previously
        // relied on federation-wide listing should fail loud, not
        // get a partial local-only result and call it the
        // federation view.
        let err = run(AbilitiesArgs {
            agent: None,
            node: Some("some-remote-node".into()),
            pattern: String::new(),
            format: OutputFormat::Table,
        })
        .expect_err("remote --node must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("federation") || msg.contains("--node"),
            "must mention --node / federation status; got: {msg}"
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
}
