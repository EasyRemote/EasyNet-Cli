// EasyNet CLI
// ===========
//
// File: src/cli/mcp_install.rs
// Description: `easynet mcp-install` — install/update MCP server entries for Claude Code / Codex.
//
// Goals:
// - One command to add an `mcpServers.<name>` entry pointing at `easynet mcp serve`.
// - Support multiple installs (one per agent/device) by changing `--name` and `--bound-node`.
// - Safe by default: refuses to overwrite existing entries unless `--force`.
//
// Notes:
// - Claude Code settings live at `~/.claude/settings.json` (see case12 in EasyNet-Axon).
// - Codex CLI reads MCP servers from `~/.codex/config.toml` under `[mcp_servers.<name>]`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, ValueEnum};
use serde_json::{json, Map, Value};
use toml_edit::{value as toml_value, DocumentMut, Item, Table};

use crate::persistence::config;
use crate::support::output;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum McpInstallClient {
    Claude,
    Codex,
}

#[derive(Debug, Args)]
pub struct McpInstallArgs {
    /// Target client (claude|codex)
    #[arg(value_enum)]
    pub client: McpInstallClient,

    /// MCP server key under 'mcpServers' (e.g. "easynet", "easynet-device-a")
    #[arg(long, default_value = "easynet")]
    pub name: String,

    /// Tenant ID passed to 'easynet mcp serve'
    ///
    /// If omitted, we try reading from '~/.easynet/runtime.json', else default to "default".
    #[arg(long)]
    pub tenant: Option<String>,

    /// Runtime endpoint passed to 'easynet mcp serve'
    ///
    /// If omitted, 'easynet mcp serve' auto-detects from '~/.easynet/runtime.json'.
    #[arg(long)]
    pub endpoint: Option<String>,

    /// Pin node-scoped tools to this node_id. The MCP server will
    /// substitute 'node_id' into every invocation that omits it, so the
    /// hosting agent (Claude Code / Codex) talks to exactly one device
    /// for the lifetime of the session.
    ///
    /// By default the binding is a *hard lock*: an explicit 'node_id'
    /// that disagrees with '--bound-node' is rejected. Pass
    /// '--allow-node-override' to demote the binding to a *default*
    /// that callers may override on a per-call basis.
    #[arg(long, value_name = "NODE_ID")]
    pub bound_node: Option<String>,

    /// Demote '--bound-node' from a hard lock to a per-call default:
    /// calls that carry an explicit 'node_id' are routed to that node
    /// instead of being rejected. Has no effect without '--bound-node'.
    #[arg(long)]
    pub allow_node_override: bool,

    /// Label the MCP server with an agent id (purely informational; passed to 'easynet mcp serve --agent').
    #[arg(long)]
    pub agent: Option<String>,

    /// Override config file path.
    ///
    /// - claude: defaults to '~/.claude/settings.json'
    /// - codex: defaults to '~/.codex/config.toml'
    #[arg(long)]
    pub config_path: Option<String>,

    /// Explicit path to 'libaxon_dendrite_bridge' to inject into MCP server env as
    /// 'EASYNET_DENDRITE_BRIDGE_LIB'.
    ///
    /// Recommended when running from GUI apps (Claude Code / Codex App) that do not inherit
    /// your shell environment.
    #[arg(long)]
    pub bridge_lib: Option<String>,

    /// Print the resulting JSON and do not write.
    #[arg(long)]
    pub dry_run: bool,

    /// Overwrite existing entry with the same name.
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: McpInstallArgs) -> anyhow::Result<()> {
    let config_path = resolve_config_path(args.client, args.config_path.as_deref())?;
    let (tenant, endpoint) =
        resolve_runtime_defaults(args.tenant.as_deref(), args.endpoint.as_deref());

    let spec = build_install_spec(&tenant, endpoint.as_deref(), &args)?;

    match args.client {
        McpInstallClient::Claude => install_for_claude(&config_path, &spec, &args)?,
        McpInstallClient::Codex => install_for_codex(&config_path, &spec, &args)?,
    }

    if args.dry_run {
        return Ok(());
    }

    output::success(&format!(
        "Installed MCP server '{}' into {}",
        args.name,
        config_path.display()
    ));
    output::detail(
        "client",
        match args.client {
            McpInstallClient::Claude => "claude",
            McpInstallClient::Codex => "codex",
        },
    );
    output::detail("tenant", &tenant);
    if let Some(ep) = endpoint.as_deref() {
        output::detail("endpoint", ep);
    } else {
        output::detail("endpoint", "(auto-detect via ~/.easynet/runtime.json)");
    }
    if spec.env.contains_key("EASYNET_DENDRITE_BRIDGE_LIB") {
        output::detail("bridge_lib", "configured");
    } else {
        output::warn("EASYNET_DENDRITE_BRIDGE_LIB not configured for this MCP server.");
        output::step("If MCP tools fail to connect, re-run with:");
        output::step("  easynet mcp-install ... --bridge-lib /abs/path/to/libaxon_dendrite_bridge.(dylib|so|dll)");
    }
    if let Some(node) = args.bound_node.as_deref() {
        output::detail("bound_node", node);
        if !args.allow_node_override {
            output::detail("node_override", "disabled");
        }
    }
    if let Some(agent) = args.agent.as_deref() {
        output::detail("agent", agent);
    }
    Ok(())
}

fn resolve_runtime_defaults(
    tenant: Option<&str>,
    endpoint: Option<&str>,
) -> (String, Option<String>) {
    let mut resolved_tenant = tenant.map(|s| s.to_string());
    let mut resolved_endpoint = endpoint.map(|s| s.to_string());

    if let (Some(t), Some(_)) = (&resolved_tenant, &resolved_endpoint) {
        return (t.clone(), resolved_endpoint);
    }

    if let Ok(state) = config::load() {
        if resolved_endpoint.is_none() && !state.endpoint.trim().is_empty() {
            resolved_endpoint = Some(state.endpoint);
        }
        if resolved_tenant.is_none() {
            resolved_tenant = state.tenant.clone().or_else(|| Some("default".to_string()));
        }
    }

    (
        resolved_tenant.unwrap_or_else(|| "default".to_string()),
        resolved_endpoint,
    )
}

fn resolve_config_path(
    client: McpInstallClient,
    override_path: Option<&str>,
) -> anyhow::Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    let home = config::home_dir();
    match client {
        McpInstallClient::Claude => Ok(home.join(".claude").join("settings.json")),
        McpInstallClient::Codex => Ok(home.join(".codex").join("config.toml")),
    }
}

#[derive(Debug, Clone)]
struct InstallSpec {
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

fn build_install_spec(
    tenant: &str,
    endpoint: Option<&str>,
    args: &McpInstallArgs,
) -> anyhow::Result<InstallSpec> {
    // `easynet mcp serve` is a two-token CLI path; pre-fix this
    // wrote `mcp-server` (hyphenated, single token) which the
    // CLI dispatcher doesn't recognise. See workspace.rs slice-27
    // commit for the corresponding fix on the spawned-from-CLI
    // path; this one is the operator-facing
    // `easynet mcp install` path that writes the same shape into
    // ~/.claude/settings.json or ~/.codex/config.toml.
    // `easynet mcp serve` accepts only --tenant and --agent
    // (see cli/mcp_server.rs::McpServerArgs). The flags
    // we used to write — --endpoint, --bound-node,
    // --allow-node-override — were dropped in the P4.9
    // quarantine. Keep accepting them as `easynet mcp install`
    // CLI inputs for backwards compatibility (the operator's
    // muscle memory still uses them) but DON'T write them into
    // the spawn args; doing so causes claude/codex to spawn the
    // MCP subprocess with "unexpected argument" failures.
    let mut cmd_args: Vec<String> = vec![
        "mcp".to_string(),
        "serve".to_string(),
        "--tenant".to_string(),
        tenant.to_string(),
    ];
    let _ = endpoint; // accepted for back-compat, not written
    let _ = &args.bound_node;
    let _ = args.allow_node_override;
    if let Some(agent) = args.agent.as_deref() {
        cmd_args.push("--agent".to_string());
        cmd_args.push(agent.to_string());
    }

    let command = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "easynet".to_string());

    let mut env = BTreeMap::<String, String>::new();
    if let Some(lib) = super::bridge_lib::resolve_bridge_lib(args.bridge_lib.as_deref())? {
        env.insert("EASYNET_DENDRITE_BRIDGE_LIB".to_string(), lib);
    }

    Ok(InstallSpec {
        command,
        args: cmd_args,
        env,
    })
}

fn install_for_claude(
    config_path: &Path,
    spec: &InstallSpec,
    args: &McpInstallArgs,
) -> anyhow::Result<()> {
    let server_entry = build_claude_server_entry(spec)?;

    let mut root = load_json_object_or_empty(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let servers = ensure_object_field(&mut root, "mcpServers")?;

    if servers.contains_key(&args.name) && !args.force {
        anyhow::bail!(
            "mcpServers.{} already exists in {} (use --force to overwrite)",
            args.name,
            config_path.display()
        );
    }
    servers.insert(args.name.clone(), server_entry);

    let out = serde_json::to_string_pretty(&Value::Object(root))? + "\n";
    if args.dry_run {
        print!("{out}");
        return Ok(());
    }
    write_atomic(config_path, out.as_bytes())?;
    Ok(())
}

fn build_claude_server_entry(spec: &InstallSpec) -> anyhow::Result<Value> {
    // Use a stable ordering for JSON output (helps diffs).
    let mut entry = BTreeMap::<String, Value>::new();
    entry.insert("command".to_string(), Value::String(spec.command.clone()));
    entry.insert(
        "args".to_string(),
        Value::Array(spec.args.iter().cloned().map(Value::String).collect()),
    );
    if !spec.env.is_empty() {
        let mut env_obj = Map::new();
        for (k, v) in &spec.env {
            env_obj.insert(k.clone(), Value::String(v.clone()));
        }
        entry.insert("env".to_string(), Value::Object(env_obj));
    }
    Ok(Value::Object(
        entry.into_iter().collect::<Map<String, Value>>(),
    ))
}

fn install_for_codex(
    config_path: &Path,
    spec: &InstallSpec,
    args: &McpInstallArgs,
) -> anyhow::Result<()> {
    let mut doc = load_toml_document_or_empty(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;

    ensure_table(&mut doc, "mcp_servers");
    {
        let servers = doc["mcp_servers"]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("mcp_servers must be a TOML table"))?;

        if servers.contains_key(&args.name) && !args.force {
            anyhow::bail!(
                "mcp_servers.{} already exists in {} (use --force to overwrite)",
                args.name,
                config_path.display()
            );
        }

        let server_item = servers
            .entry(&args.name)
            .or_insert_with(|| Item::Table(Table::new()));
        let server = server_item
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("mcp_servers.{} must be a TOML table", args.name))?;

        server.insert("command", toml_value(spec.command.clone()));
        let mut args_array = toml_edit::Array::new();
        for arg in &spec.args {
            args_array.push(arg.as_str());
        }
        server.insert("args", toml_value(args_array));

        if !spec.env.is_empty() {
            // Ensure env is a table (override inline table / non-table values if present).
            if server.get("env").and_then(Item::as_table).is_none() {
                server.insert("env", Item::Table(Table::new()));
            }
            let env_table = server
                .get_mut("env")
                .and_then(Item::as_table_mut)
                .ok_or_else(|| {
                    anyhow::anyhow!("mcp_servers.{}.env must be a TOML table", args.name)
                })?;
            for (k, v) in &spec.env {
                env_table.insert(k, toml_value(v.clone()));
            }
        }
    }

    let out = doc.to_string() + "\n";
    if args.dry_run {
        print!("{out}");
        return Ok(());
    }
    write_atomic(config_path, out.as_bytes())?;
    Ok(())
}

fn load_json_object_or_empty(path: &Path) -> anyhow::Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let data = fs::read_to_string(path)?;
    if data.trim().is_empty() {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_str(&data)?;
    match v {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("config file must be a JSON object"),
    }
}

fn ensure_object_field<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    let slot = root.entry(key.to_string()).or_insert_with(|| json!({}));
    match slot {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("{key} must be a JSON object"),
    }
}

fn load_toml_document_or_empty(path: &Path) -> anyhow::Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let data = fs::read_to_string(path)?;
    if data.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    data.parse::<DocumentMut>()
        .map_err(|e| anyhow::anyhow!("invalid TOML: {e}"))
}

fn ensure_table(doc: &mut DocumentMut, key: &str) {
    if !doc.contains_key(key) {
        doc[key] = Item::Table(Table::new());
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
