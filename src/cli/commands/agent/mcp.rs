// EasyNet CLI — `easynet agent` MCP binding surface: plan/write tool projections + manifests
// Split from cli/agent.rs (F-033 / T4.6); bodies are move-only.

use serde_json::Value;
use std::{path::Path, time::Duration};

use crate::daemon::execution::mission::directory::AgentDirectory;
use crate::support::platform::output;

use super::*;

pub(super) fn run_mcp(args: McpArgs) -> anyhow::Result<()> {
    match args.action {
        McpAction::Add(a) => run_mcp_add(a),
    }
}

/// Top-level CLI entry. Each phase is delegated to a small,
/// independently testable helper so this function reads as the
/// product flow:
///
///   1. resolve the target agent + MCP config
///   2. plan the manifests that should exist (no filesystem writes)
///   3. validate the user's `--tool` selection against the plan
///   4. materialise / dry-run the plans + render the operator summary
pub(crate) fn run_mcp_add(args: McpAddArgs) -> anyhow::Result<()> {
    let config_path = args
        .config
        .clone()
        .map(Ok)
        .unwrap_or_else(crate::daemon::execution::mcp::McpClientService::default_config_path)?;
    let svc = crate::daemon::execution::mcp::McpClientService::from_path(&config_path)?;

    let declared_cost = build_cost_meta(args.cost_kind, args.cost_label.as_deref())?;
    let plan = plan_mcp_additions(
        &svc,
        &config_path,
        args.server.as_deref(),
        &args.tools,
        &args.prefix,
        args.skip_unreachable,
        declared_cost.as_ref(),
    )?;
    assert_tools_filter_satisfied(&args.tools, &plan.planned)?;

    if plan.planned.is_empty() {
        report_empty_plan(&plan.list_failures);
        return Ok(());
    }

    let manifests = plan
        .planned
        .iter()
        .map(mcp_manifest_for)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let outcome = if args.dry_run {
        let directory =
            crate::cli::commands::agent_new_ability::preview_agent_directory(&args.name)?;
        preview_mcp_additions(&directory, &manifests, args.overwrite)?
    } else {
        let committed = crate::cli::commands::agent_new_ability::commit_ability_manifests(
            &args.name,
            manifests,
            args.overwrite,
            crate::cli::commands::agent_new_ability::AbilityConflictPolicy::RetainSameBinding,
        )?;
        McpAdditionOutcome {
            written: committed.written.len(),
            skipped: committed.skipped.len(),
            abilities_dir: committed.root_path.join("abilities"),
            live_publication_count: committed.publication.len(),
        }
    };
    report_write_outcome(&args.name, &plan, &outcome, args.dry_run);
    Ok(())
}

/// Result of phase (2): the manifests we'd write and the per-upstream
/// `tools/list` failures we tolerated via `--skip-unreachable`.
#[derive(Debug, Default)]
struct McpAdditionPlan {
    planned: Vec<McpAbilityPlan>,
    list_failures: Vec<String>,
}

/// Result of phase (4): per-plan disposition. `written` counts new
/// files created; `skipped` counts plans whose target already held
/// the same `(server, tool)` binding (idempotent re-runs).
#[derive(Debug, Default)]
struct McpAdditionOutcome {
    written: usize,
    skipped: usize,
    abilities_dir: std::path::PathBuf,
    live_publication_count: usize,
}

/// Build the manifest plan for one `easynet agent mcp add` invocation.
///
/// Pure-ish: the only side effect is talking to `svc` (which itself
/// reads the operator's `mcp_clients.json` config). No filesystem
/// writes happen here — that's phase (4).
/// Build the `CostMeta` value the manifest writer will stamp on every
/// generated ability, or `None` when the operator did not pass
/// `--cost-kind`. Folds the two flags into one structure here so the
/// downstream pipeline only has to consider "declared / not declared",
/// not the cartesian product.
pub(super) fn build_cost_meta(
    cost_kind: Option<CostKindArg>,
    cost_label: Option<&str>,
) -> anyhow::Result<Option<crate::daemon::ability::manifest::CostMeta>> {
    use crate::daemon::ability::manifest::CostMeta;
    let Some(kind) = cost_kind else {
        return Ok(None);
    };
    // Trimmed-empty labels are forbidden by `CostMeta::validate`, but
    // the CLI surface lets a user pass `--cost-label ""` (or just
    // whitespace) — translate that into "omitted" so it round-trips
    // through validation rather than failing at write time.
    let label = cost_label
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string);
    let meta = CostMeta {
        kind: kind.into_core(),
        label,
    };
    Ok(Some(meta))
}

fn plan_mcp_additions(
    svc: &crate::daemon::execution::mcp::McpClientService,
    config_path: &std::path::Path,
    server_filter: Option<&str>,
    tool_filter: &[String],
    prefix: &str,
    skip_unreachable: bool,
    declared_cost: Option<&crate::daemon::ability::manifest::CostMeta>,
) -> anyhow::Result<McpAdditionPlan> {
    let selected_servers = select_mcp_servers(svc, server_filter)?;
    if selected_servers.is_empty() {
        anyhow::bail!(
            "no MCP servers configured in {}; populate the file with at least one server entry first",
            config_path.display()
        );
    }

    let mut plan = McpAdditionPlan::default();
    for server in selected_servers {
        let listing = match mcp_rpc_blocking_timeout(
            svc,
            &server,
            "tools/list",
            serde_json::json!({}),
            mcp_tools_list_timeout(),
        ) {
            Ok(v) => v,
            Err(e) if skip_unreachable => {
                plan.list_failures.push(format!("{server}: {e}"));
                continue;
            }
            Err(e) => anyhow::bail!("{server}: tools/list failed: {e}"),
        };
        let tools = listing
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("{server}: tools/list response missing `tools` array")
            })?;
        for tool in tools {
            let Some(upstream_tool) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !tool_filter.is_empty() && !tool_filter.iter().any(|t| t == upstream_tool) {
                continue;
            }
            let input_schema = normalize_mcp_input_schema(tool.get("inputSchema").cloned());
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Call MCP tool `{upstream_tool}` on `{server}`."));
            let verb = generated_mcp_ability_name(prefix, &server, upstream_tool);
            plan.planned.push(McpAbilityPlan {
                server: server.clone(),
                tool: upstream_tool.to_string(),
                verb,
                description,
                input_schema,
                cost: declared_cost.cloned(),
            });
        }
    }
    Ok(plan)
}

/// Phase (3): every `--tool` the operator named must resolve into
/// the plan. Missing tools indicate a typo or a misaligned upstream
/// catalogue; failing loud here beats silently materialising fewer
/// abilities than the operator asked for.
pub(super) fn assert_tools_filter_satisfied(
    tool_filter: &[String],
    planned: &[McpAbilityPlan],
) -> anyhow::Result<()> {
    if tool_filter.is_empty() {
        return Ok(());
    }
    let found: std::collections::BTreeSet<&str> = planned.iter().map(|p| p.tool.as_str()).collect();
    let missing: Vec<&str> = tool_filter
        .iter()
        .map(String::as_str)
        .filter(|tool| !found.contains(tool))
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "requested MCP tool(s) not found in selected server set: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

/// Read-only dry-run projection. The daemon remains the sole writer; this
/// helper only preserves the existing preview/conflict UX.
fn preview_mcp_additions(
    dir: &AgentDirectory,
    manifests: &[crate::daemon::ability::manifest::AbilityManifest],
    overwrite: bool,
) -> anyhow::Result<McpAdditionOutcome> {
    let mut outcome = McpAdditionOutcome {
        abilities_dir: dir.abilities_dir(),
        ..Default::default()
    };
    for manifest in manifests {
        let body = manifest.to_toml_string()?;
        let path = dir
            .abilities_dir()
            .join(format!("{}.ability.toml", manifest.name()));

        if path.exists() && !overwrite {
            if existing_mcp_binding_from_path(&path)?.as_ref()
                == proposed_mcp_binding(manifest).as_ref()
            {
                outcome.skipped += 1;
                continue;
            }
            anyhow::bail!(
                "refusing to overwrite existing ability manifest {}; pass --overwrite to replace it",
                path.display()
            );
        }

        println!("--- {}", path.display());
        print!("{body}");
        outcome.written += 1;
    }
    Ok(outcome)
}

/// Operator summary when no plans were produced (either the filter
/// matched nothing or every upstream was unreachable under
/// `--skip-unreachable`).
fn report_empty_plan(list_failures: &[String]) {
    if list_failures.is_empty() {
        output::info("No MCP tools matched the requested selection.");
    } else {
        output::warn("No MCP tools were bound; every selected upstream failed tools/list.");
        for failure in list_failures {
            output::warn(failure);
        }
    }
}

/// Operator summary for a non-empty plan; mirrors the shape of the
/// other `easynet agent …` subcommands (success line + key/value
/// detail lines + trailing warnings for partial failures).
fn report_write_outcome(
    agent_name: &str,
    plan: &McpAdditionPlan,
    outcome: &McpAdditionOutcome,
    dry_run: bool,
) {
    if dry_run {
        output::success(&format!(
            "dry-run: {} MCP ability manifest(s) would be written for agent '{}'",
            plan.planned.len(),
            agent_name
        ));
    } else {
        output::success(&format!(
            "added {} MCP ability manifest(s) to agent '{}'",
            outcome.written, agent_name
        ));
        if outcome.skipped > 0 {
            output::detail(
                "skipped",
                &format!("{} existing identical binding(s)", outcome.skipped),
            );
        }
        output::detail("root", &outcome.abilities_dir.display().to_string());
        output::detail(
            "live",
            &format!(
                "{} committed Agent ability descriptor(s)",
                outcome.live_publication_count
            ),
        );
        output::info("The daemon committed the MCP bindings to the live capability catalog.");
    }
    for failure in &plan.list_failures {
        output::warn(failure);
    }
}

#[derive(Debug, Clone)]
pub(super) struct McpAbilityPlan {
    pub(super) server: String,
    pub(super) tool: String,
    pub(super) verb: String,
    pub(super) description: String,
    pub(super) input_schema: Value,
    /// Operator-declared cost meta, forwarded verbatim from
    /// `--cost-kind`/`--cost-label`. `None` writes a manifest with no
    /// `[cost]` table; the runtime falls back to the per-exec
    /// inference at metadata-emit time.
    pub(super) cost: Option<crate::daemon::ability::manifest::CostMeta>,
}

fn select_mcp_servers(
    svc: &crate::daemon::execution::mcp::McpClientService,
    server: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    mcp_block_on(async {
        let names = svc.server_names().await;
        match server {
            Some(wanted) => {
                if names.iter().any(|n| n == wanted) {
                    Ok(vec![wanted.to_string()])
                } else {
                    anyhow::bail!(
                        "MCP server {wanted:?} not found in configured servers: {}",
                        names.join(", ")
                    )
                }
            }
            None => Ok(names),
        }
    })
}

fn mcp_rpc_blocking_timeout(
    svc: &crate::daemon::execution::mcp::McpClientService,
    server: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> anyhow::Result<Value> {
    mcp_block_on(async move {
        match tokio::time::timeout(timeout, svc.rpc(server, method, params)).await {
            Ok(result) => result,
            Err(_) => anyhow::bail!("{method} timed out after {}s", timeout.as_secs()),
        }
    })
}

fn mcp_tools_list_timeout() -> Duration {
    let secs = std::env::var("EASYNET_MCP_TOOLS_LIST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(20);
    Duration::from_secs(secs)
}

fn mcp_block_on<F, T>(fut: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => Ok(tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fut)
        })?),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("build mcp cli runtime: {e}"))?;
            rt.block_on(fut)
        }
    }
}

pub(super) fn normalize_mcp_input_schema(schema: Option<Value>) -> Value {
    match schema {
        Some(v @ Value::Object(_)) => toml_safe_json_value(v),
        Some(v) => serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "x-easynet-originalInputSchema": toml_safe_json_value(v),
        }),
        None => serde_json::json!({
            "type": "object",
            "additionalProperties": true,
        }),
    }
}

fn toml_safe_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let safe = map
                .into_iter()
                .filter_map(|(k, v)| {
                    if v.is_null() {
                        None
                    } else {
                        Some((k, toml_safe_json_value(v)))
                    }
                })
                .collect();
            Value::Object(safe)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| {
                    if v.is_null() {
                        Value::String("null".into())
                    } else {
                        toml_safe_json_value(v)
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

pub(super) fn mcp_manifest_for(
    plan: &McpAbilityPlan,
) -> anyhow::Result<crate::daemon::ability::manifest::AbilityManifest> {
    use crate::daemon::ability::manifest::{AbilityExec, AbilityManifest, McpExec};
    let mut manifest = AbilityManifest::new(
        plan.verb.clone(),
        plan.description.clone(),
        plan.input_schema.clone(),
    )?
    .with_exec(AbilityExec::Mcp(McpExec {
        server: plan.server.clone(),
        tool: plan.tool.clone(),
    }))?
    .with_output_schema(serde_json::json!({
        "type": "object",
        "properties": {
            "content": {"type": "array"},
            "isError": {"type": "boolean"}
        },
        "required": ["content"],
        "additionalProperties": true
    }))?;
    if let Some(cost) = &plan.cost {
        manifest = manifest.with_cost(cost.clone())?;
    }
    Ok(manifest)
}

fn existing_mcp_binding_from_path(path: &Path) -> anyhow::Result<Option<(String, String)>> {
    let body = std::fs::read_to_string(path).map_err(|error| {
        anyhow::anyhow!("read existing ability manifest {}: {error}", path.display())
    })?;
    existing_mcp_binding(&body).map_err(|error| {
        anyhow::anyhow!(
            "parse existing ability manifest {} for MCP binding: {error:#}",
            path.display()
        )
    })
}

pub(super) fn existing_mcp_binding(body: &str) -> anyhow::Result<Option<(String, String)>> {
    use crate::daemon::ability::manifest::AbilityExec;
    let manifest = crate::daemon::ability::manifest::AbilityManifest::from_toml_str(body)?;
    Ok(match manifest.exec() {
        Some(AbilityExec::Mcp(exec)) => Some((exec.server.clone(), exec.tool.clone())),
        _ => None,
    })
}

fn proposed_mcp_binding(
    manifest: &crate::daemon::ability::manifest::AbilityManifest,
) -> Option<(String, String)> {
    use crate::daemon::ability::manifest::AbilityExec;
    match manifest.exec()? {
        AbilityExec::Mcp(exec) => Some((exec.server.clone(), exec.tool.clone())),
        _ => None,
    }
}

pub(super) fn generated_mcp_ability_name(prefix: &str, server: &str, tool: &str) -> String {
    let prefix_slug = slug_segment(prefix);
    let server_slug = slug_segment(server);
    let tool_slug = slug_segment(tool);
    let base = if prefix_slug.is_empty() {
        format!("{server_slug}_{tool_slug}")
    } else {
        format!("{prefix_slug}_{server_slug}_{tool_slug}")
    };
    McpAbilityNameProjection::from_parts(base, server, tool).into_name()
}

/// Canonical projection from upstream MCP server/tool identifiers to an
/// EasyNet ability name.
///
/// Flat names are the normal `{prefix}_{server}_{tool}` projection. Some
/// upstream identifiers slugify to an empty or separator-only shape; those are
/// not compatibility cases. They are a distinct, canonical projection state
/// that binds the raw upstream identifiers into a deterministic digest so two
/// punctuation-only tool pairs remain different.
#[derive(Debug, Clone, PartialEq, Eq)]
enum McpAbilityNameProjection {
    Flat(String),
    DigestDisambiguated { digest_input: String },
}

impl McpAbilityNameProjection {
    fn from_parts(base: String, server: &str, tool: &str) -> Self {
        if Self::requires_digest(&base) {
            Self::DigestDisambiguated {
                digest_input: format!("{server}:{tool}"),
            }
        } else {
            Self::Flat(base)
        }
    }

    fn requires_digest(base: &str) -> bool {
        base.is_empty() || base.chars().all(|c| c == '_' || c == '-')
    }

    fn into_name(self) -> String {
        match self {
            Self::Flat(name) => name,
            Self::DigestDisambiguated { digest_input } => {
                format!("mcp_{}", short_hex(digest_input.as_bytes()))
            }
        }
    }
}

fn slug_segment(raw: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' || ch == '-' {
            Some(ch)
        } else {
            Some('_')
        };
        if let Some(c) = mapped {
            if c == '_' || c == '-' {
                if !last_was_sep && !out.is_empty() {
                    out.push('_');
                    last_was_sep = true;
                }
            } else {
                out.push(c);
                last_was_sep = false;
            }
        }
    }
    while out.ends_with('_') || out.ends_with('-') {
        out.pop();
    }
    out
}

fn short_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}
