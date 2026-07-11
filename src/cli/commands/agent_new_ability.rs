// EasyNet CLI - Agent-owned ability factories
// ===========================================
//
// File: src/cli/agent_new_ability.rs
// Description: `easynet agent <agent> new-ability ...` - create
//              deterministic ability manifests under one agent root.
//
// Protocol Responsibility:
//   Abilities are owned by Agents. This module keeps the CLI grammar
//   object-shaped (`agent <id-or-ura> new-ability ...`) and writes only
//   local `<agent-root>/abilities/*.ability.toml` manifests; runtime
//   publication remains the existing `agent refresh` / daemon
//   advertise path.
//
// Implementation Approach:
//   Parse the scoped tail with a small Clap sub-parser, project source
//   catalogues (HTTP endpoint, OpenAPI operation, MCP tool) into
//   `AbilityManifest`, then atomically materialise the TOML.
//
// Usage Contract:
//   No global `ability api add` entry point is exposed here because a
//   generated ability without an Agent owner is not a complete EasyNet
//   object.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::{error::ErrorKind, Args, Parser, Subcommand};
use serde_json::{json, Value};

use crate::cli::commands::agent::{CostKindArg, McpAddArgs};
use crate::daemon::ability::manifest::{
    AbilityExec, AbilityManifest, CostMeta, HttpExec, ShellExec,
};
use crate::daemon::execution::mission::directory::AgentDirectory;
use crate::daemon::persistence::agent_registry as agents;
use crate::daemon::persistence::config;
use crate::support::platform::output;

pub(crate) fn run_scoped(selector: &str, tail: &[String]) -> anyhow::Result<()> {
    let agent_name = resolve_agent_selector(selector)?;
    let cli = match ScopedAgentCli::try_parse_from(
        std::iter::once(format!("easynet agent {selector}")).chain(tail.iter().cloned()),
    ) {
        Ok(cli) => cli,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            err.print()?;
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    match cli.action {
        ScopedAgentAction::NewAbility(args) => run_new_ability(&agent_name, args),
    }
}

#[derive(Debug, Parser)]
struct ScopedAgentCli {
    #[command(subcommand)]
    action: ScopedAgentAction,
}

#[derive(Debug, Subcommand)]
enum ScopedAgentAction {
    /// Create a new ability manifest owned by this agent.
    #[command(name = "new-ability")]
    NewAbility(NewAbilityArgs),
}

#[derive(Debug, Args)]
struct NewAbilityArgs {
    #[command(subcommand)]
    source: NewAbilitySource,
}

#[derive(Debug, Subcommand)]
enum NewAbilitySource {
    /// Create one HTTP-backed ability from an explicit endpoint.
    Api(ApiArgs),
    /// Create one HTTP-backed ability from an OpenAPI operation.
    #[command(name = "from-openapi")]
    FromOpenapi(FromOpenApiArgs),
    /// Bind configured upstream MCP tools into this agent.
    Mcp(McpArgs),
    /// Create one shell-backed ability from a local command.
    Script(ScriptArgs),
}

#[derive(Debug, Args)]
struct ScriptArgs {
    #[command(subcommand)]
    action: ScriptAction,
}

#[derive(Debug, Subcommand)]
enum ScriptAction {
    /// Add one local command as an ability.
    Add(ScriptAddArgs),
}

#[derive(Debug, Args)]
struct ScriptAddArgs {
    /// Ability verb to create under the selected agent.
    pub name: String,
    /// Command argv after `--`. argv[0] is the program; `{{ arg }}`
    /// placeholders become input-schema properties and are rendered
    /// per-element (no `sh -c`, no word splitting).
    #[arg(last = true, required = true)]
    pub argv: Vec<String>,
    /// Human-readable ability description.
    #[arg(long)]
    pub description: Option<String>,
    /// Optional JSON schema file for ability input. When omitted,
    /// schema is inferred from `{{ arg }}` placeholders.
    #[arg(long)]
    pub input_schema: Option<PathBuf>,
    /// Stdout decoder. Today the runtime supports `utf8_trim`.
    #[arg(long)]
    pub stdout: Option<String>,
    /// OS-level sandbox profile: none | net_only | pure_compute.
    #[arg(long)]
    pub sandbox: Option<String>,
    /// Per-invocation timeout in seconds.
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Print the manifest without writing it.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an existing manifest of the same name.
    #[arg(long)]
    pub overwrite: bool,
    /// Optional explicit cost bucket.
    #[arg(long, value_enum)]
    pub cost_kind: Option<CostKindArg>,
    /// Free-form human label accompanying `--cost-kind`.
    #[arg(long, requires = "cost_kind")]
    pub cost_label: Option<String>,
}

#[derive(Debug, Args)]
struct ApiArgs {
    #[command(subcommand)]
    action: ApiAction,
}

#[derive(Debug, Subcommand)]
enum ApiAction {
    /// Add one HTTP endpoint as an ability.
    Add(ApiAddArgs),
}

#[derive(Debug, Args)]
struct McpArgs {
    #[command(subcommand)]
    action: McpAction,
}

#[derive(Debug, Subcommand)]
enum McpAction {
    /// Add upstream MCP tools as deterministic abilities on this agent.
    Add(ScopedMcpAddArgs),
}

#[derive(Debug, Args)]
struct ScopedMcpAddArgs {
    /// Optional upstream MCP server name from mcp_clients.json. Omit
    /// to bind tools from every configured server.
    #[arg(long)]
    server: Option<String>,
    /// Optional upstream tool name. Repeat to bind a subset.
    #[arg(long = "tool")]
    tools: Vec<String>,
    /// Optional prefix for generated ability verbs.
    #[arg(long, default_value = "mcp")]
    prefix: String,
    /// Path to mcp_clients.json.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print the manifests that would be written.
    #[arg(long)]
    dry_run: bool,
    /// Replace an existing generated manifest.
    #[arg(long)]
    overwrite: bool,
    /// Continue when one upstream fails tools/list.
    #[arg(long)]
    skip_unreachable: bool,
    /// Optional explicit cost bucket for generated manifests.
    #[arg(long, value_enum)]
    cost_kind: Option<CostKindArg>,
    /// Free-form human label accompanying `--cost-kind`.
    #[arg(long, requires = "cost_kind")]
    cost_label: Option<String>,
}

#[derive(Debug, Args)]
struct ApiAddArgs {
    /// Ability verb to create under the selected agent.
    pub name: String,
    /// HTTP method.
    #[arg(long, default_value = "GET")]
    pub method: String,
    /// URL template. `{{ arg }}` placeholders become input-schema
    /// properties and are URL-encoded by the runtime executor.
    #[arg(long)]
    pub url: String,
    /// Human-readable ability description.
    #[arg(long)]
    pub description: Option<String>,
    /// Header template. Repeatable. Accepts `Name: value` or
    /// `Name=value`.
    #[arg(long = "header")]
    pub headers: Vec<String>,
    /// Optional request body template.
    #[arg(long)]
    pub body: Option<String>,
    /// Optional JSON schema file for ability input. When omitted,
    /// schema is inferred from `{{ arg }}` placeholders.
    #[arg(long)]
    pub input_schema: Option<PathBuf>,
    /// Response decoder. Today the runtime supports `text_trim`.
    #[arg(long, default_value = "text_trim")]
    pub response: String,
    /// Per-invocation timeout in seconds.
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Print the manifest without writing it.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an existing manifest of the same name.
    #[arg(long)]
    pub overwrite: bool,
    /// Optional explicit cost bucket.
    #[arg(long, value_enum)]
    pub cost_kind: Option<CostKindArg>,
    /// Free-form human label accompanying `--cost-kind`.
    #[arg(long, requires = "cost_kind")]
    pub cost_label: Option<String>,
}

#[derive(Debug, Args)]
struct FromOpenApiArgs {
    /// OpenAPI JSON/YAML file path or HTTPS URL.
    pub spec: String,
    /// Select by operationId.
    #[arg(long)]
    pub operation_id: Option<String>,
    /// Select by path when operationId is absent.
    #[arg(long)]
    pub path: Option<String>,
    /// Select by method when operationId is absent.
    #[arg(long)]
    pub method: Option<String>,
    /// Override generated ability name.
    #[arg(long)]
    pub name: Option<String>,
    /// Override OpenAPI servers[0].url.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Prefix used when generating the ability name.
    #[arg(long, default_value = "api")]
    pub prefix: String,
    /// Print the manifest without writing it.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an existing manifest of the same name.
    #[arg(long)]
    pub overwrite: bool,
    /// Optional explicit cost bucket.
    #[arg(long, value_enum)]
    pub cost_kind: Option<CostKindArg>,
    /// Free-form human label accompanying `--cost-kind`.
    #[arg(long, requires = "cost_kind")]
    pub cost_label: Option<String>,
}

fn run_new_ability(agent_name: &str, args: NewAbilityArgs) -> anyhow::Result<()> {
    match args.source {
        NewAbilitySource::Api(api) => match api.action {
            ApiAction::Add(add) => run_api_add(agent_name, add),
        },
        NewAbilitySource::FromOpenapi(args) => run_from_openapi(agent_name, args),
        NewAbilitySource::Script(script) => match script.action {
            ScriptAction::Add(add) => run_script_add(agent_name, add),
        },
        NewAbilitySource::Mcp(mcp) => match mcp.action {
            McpAction::Add(add) => crate::cli::commands::agent::run_mcp_add(McpAddArgs {
                name: agent_name.to_string(),
                server: add.server,
                tools: add.tools,
                prefix: add.prefix,
                config: add.config,
                dry_run: add.dry_run,
                overwrite: add.overwrite,
                skip_unreachable: add.skip_unreachable,
                cost_kind: add.cost_kind,
                cost_label: add.cost_label,
            }),
        },
    }
}

fn run_api_add(agent_name: &str, args: ApiAddArgs) -> anyhow::Result<()> {
    let dir = open_registered_agent(agent_name)?;
    let manifest = api_manifest_for(&args)?;
    write_manifest(agent_name, &dir, &manifest, args.overwrite, args.dry_run)
}

fn run_script_add(agent_name: &str, args: ScriptAddArgs) -> anyhow::Result<()> {
    let dir = open_registered_agent(agent_name)?;
    let manifest = script_manifest_for(&args)?;
    write_manifest(agent_name, &dir, &manifest, args.overwrite, args.dry_run)
}

fn run_from_openapi(agent_name: &str, args: FromOpenApiArgs) -> anyhow::Result<()> {
    let dir = open_registered_agent(agent_name)?;
    let spec = load_openapi_spec(&args.spec)?;
    let op = select_openapi_operation(
        &spec,
        args.operation_id.as_deref(),
        args.path.as_deref(),
        args.method.as_deref(),
    )?;
    let manifest = openapi_manifest_for(&spec, &op, &args)?;
    write_manifest(agent_name, &dir, &manifest, args.overwrite, args.dry_run)
}

fn resolve_agent_selector(selector: &str) -> anyhow::Result<String> {
    match crate::core::ura::parse_ura(selector) {
        Ok(parsed) => {
            if parsed.kind != crate::core::ura::URAKind::Agent {
                anyhow::bail!(
                    "agent selector URA must have kind=agent, got {:?}",
                    parsed.kind
                );
            }
            let Some((_, agent_id)) = parsed.agent_ids() else {
                anyhow::bail!("agent selector URA is missing agent_id");
            };
            Ok(agent_id.to_string())
        }
        Err(_) => Ok(selector.to_string()),
    }
}

fn open_registered_agent(name: &str) -> anyhow::Result<AgentDirectory> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(name).ok_or_else(|| {
        anyhow::anyhow!(
            "agent '{name}' is not registered; run 'easynet agent list' to see registered names"
        )
    })?;
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(name));
    if !root.exists() {
        anyhow::bail!("agent '{name}' has no on-disk root at {}", root.display());
    }
    AgentDirectory::open(&root)
}

fn api_manifest_for(args: &ApiAddArgs) -> anyhow::Result<AbilityManifest> {
    let headers = parse_headers(&args.headers)?;
    let input_schema = match &args.input_schema {
        Some(path) => read_json_schema(path)?,
        None => infer_input_schema(
            std::iter::once(args.url.as_str())
                .chain(headers.values().map(String::as_str))
                .chain(args.body.as_deref()),
        ),
    };
    let description = args
        .description
        .clone()
        .unwrap_or_else(|| format!("Call HTTP API {} {}", args.method, args.url));
    let mut manifest = AbilityManifest::new(args.name.clone(), description, input_schema)?
        .with_exec(AbilityExec::Http(HttpExec {
            method: args.method.to_ascii_uppercase(),
            url: args.url.clone(),
            headers: if headers.is_empty() {
                None
            } else {
                Some(headers)
            },
            body: args.body.clone(),
            response: Some(args.response.clone()),
        }))?
        .with_output_schema(http_output_schema())?;
    if let Some(timeout) = args.timeout {
        manifest = manifest.with_timeout_seconds(timeout)?;
    }
    if let Some(cost) = build_cost_meta(args.cost_kind, args.cost_label.as_deref())? {
        manifest = manifest.with_cost(cost)?;
    }
    Ok(manifest)
}

fn script_manifest_for(args: &ScriptAddArgs) -> anyhow::Result<AbilityManifest> {
    let input_schema = match &args.input_schema {
        Some(path) => read_json_schema(path)?,
        None => infer_input_schema(args.argv.iter().map(String::as_str)),
    };
    let description = args
        .description
        .clone()
        .unwrap_or_else(|| format!("Run local command `{}`", args.argv.join(" ")));
    let mut manifest = AbilityManifest::new(args.name.clone(), description, input_schema)?
        .with_exec(AbilityExec::Shell(ShellExec {
            argv: args.argv.clone(),
            stdout: args.stdout.clone(),
            sandbox: args.sandbox.clone(),
        }))?
        .with_output_schema(shell_output_schema())?;
    if let Some(timeout) = args.timeout {
        manifest = manifest.with_timeout_seconds(timeout)?;
    }
    if let Some(cost) = build_cost_meta(args.cost_kind, args.cost_label.as_deref())? {
        manifest = manifest.with_cost(cost)?;
    }
    Ok(manifest)
}

#[derive(Debug, Clone)]
struct OpenApiOperation {
    path: String,
    method: String,
    operation: Value,
    path_item: Value,
}

fn openapi_manifest_for(
    spec: &Value,
    op: &OpenApiOperation,
    args: &FromOpenApiArgs,
) -> anyhow::Result<AbilityManifest> {
    let base_url = args
        .base_url
        .clone()
        .or_else(|| first_server_url(spec))
        .ok_or_else(|| {
            anyhow::anyhow!("OpenAPI spec has no servers[0].url; pass --base-url explicitly")
        })?;
    let parameters = openapi_parameters(spec, &op.path_item, &op.operation);
    let body_schema = openapi_json_request_body_schema(spec, &op.operation);
    let ability_name = args.name.clone().unwrap_or_else(|| {
        op.operation
            .get("operationId")
            .and_then(Value::as_str)
            .map(slug_segment)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slug_segment(&format!("{}_{}_{}", args.prefix, op.method, op.path)))
    });

    let mut headers = BTreeMap::new();
    let body = if body_schema.is_some() {
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Some("{{ body }}".to_string())
    } else {
        None
    };
    let url = openapi_url_template(&base_url, &op.path, &parameters);
    let input_schema = openapi_input_schema(&parameters, body_schema);
    let description = op
        .operation
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| op.operation.get("description").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| format!("Call OpenAPI operation {} {}", op.method, op.path));

    let mut manifest = AbilityManifest::new(ability_name, description, input_schema)?
        .with_exec(AbilityExec::Http(HttpExec {
            method: op.method.to_ascii_uppercase(),
            url,
            headers: if headers.is_empty() {
                None
            } else {
                Some(headers)
            },
            body,
            response: Some("text_trim".to_string()),
        }))?
        .with_output_schema(http_output_schema())?;
    if let Some(cost) = build_cost_meta(args.cost_kind, args.cost_label.as_deref())? {
        manifest = manifest.with_cost(cost)?;
    }
    Ok(manifest)
}

fn write_manifest(
    agent_name: &str,
    dir: &AgentDirectory,
    manifest: &AbilityManifest,
    overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let body = manifest.to_toml_string()?;
    let path = dir
        .abilities_dir()
        .join(format!("{}.ability.toml", manifest.name()));

    if dry_run {
        println!("--- {}", path.display());
        print!("{body}");
        output::success(&format!(
            "dry-run: ability '{}' would be written for agent '{}'",
            manifest.name(),
            agent_name
        ));
        return Ok(());
    }

    std::fs::create_dir_all(dir.abilities_dir()).map_err(|e| {
        anyhow::anyhow!(
            "create abilities directory {}: {e}",
            dir.abilities_dir().display()
        )
    })?;
    if path.exists() && !overwrite {
        anyhow::bail!(
            "refusing to overwrite existing ability manifest {}; pass --overwrite to replace it",
            path.display()
        );
    }
    config::atomic_write(&path, body.as_bytes())
        .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
    output::success(&format!(
        "added ability '{}' to agent '{}'",
        manifest.name(),
        agent_name
    ));
    output::detail("path", &path.display().to_string());
    output::info(
        "Run 'easynet agent refresh' if a running catalogue surface needs to publish it immediately.",
    );
    Ok(())
}

fn parse_headers(items: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for raw in items {
        let (name, value) = raw
            .split_once(':')
            .or_else(|| raw.split_once('='))
            .ok_or_else(|| {
                anyhow::anyhow!("header must be `Name: value` or `Name=value`: {raw}")
            })?;
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("header name must not be empty: {raw}");
        }
        out.insert(name.to_string(), value.trim().to_string());
    }
    Ok(out)
}

fn read_json_schema(path: &Path) -> anyhow::Result<Value> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read input schema {}: {e}", path.display()))?;
    let schema: Value = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse input schema {} as JSON: {e}", path.display()))?;
    Ok(toml_safe_json_value(schema))
}

fn infer_input_schema<'a>(templates: impl Iterator<Item = &'a str>) -> Value {
    let mut names = BTreeSet::new();
    for template in templates {
        collect_placeholders(template, &mut names);
    }
    if names.is_empty() {
        return json!({
            "type": "object",
            "additionalProperties": true
        });
    }
    let properties = names
        .iter()
        .map(|name| {
            (
                name.clone(),
                json!({
                    "type": "string",
                    "description": format!("Template value for `{name}`")
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": names.into_iter().collect::<Vec<_>>(),
        "additionalProperties": false
    })
}

fn collect_placeholders(template: &str, names: &mut BTreeSet<String>) {
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let name = after[..end].trim();
        if !name.is_empty() {
            names.insert(name.to_string());
        }
        rest = &after[end + 2..];
    }
}

pub(super) fn build_cost_meta(
    cost_kind: Option<CostKindArg>,
    cost_label: Option<&str>,
) -> anyhow::Result<Option<CostMeta>> {
    let Some(kind) = cost_kind else {
        return Ok(None);
    };
    Ok(Some(CostMeta {
        kind: kind.into_core(),
        label: cost_label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }))
}

fn load_openapi_spec(source: &str) -> anyhow::Result<Value> {
    let raw = if source.starts_with("https://") || source.starts_with("http://") {
        ureq::get(source)
            .call()
            .map_err(|e| anyhow::anyhow!("fetch OpenAPI spec {source}: {e}"))?
            .into_string()
            .map_err(|e| anyhow::anyhow!("read OpenAPI spec body {source}: {e}"))?
    } else {
        std::fs::read_to_string(source)
            .map_err(|e| anyhow::anyhow!("read OpenAPI spec {source}: {e}"))?
    };
    parse_openapi_document(&raw)
}

fn parse_openapi_document(raw: &str) -> anyhow::Result<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Ok(v);
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(raw)
        .map_err(|e| anyhow::anyhow!("parse OpenAPI as JSON/YAML: {e}"))?;
    serde_json::to_value(yaml).map_err(|e| anyhow::anyhow!("convert OpenAPI YAML to JSON: {e}"))
}

fn select_openapi_operation(
    spec: &Value,
    operation_id: Option<&str>,
    path_filter: Option<&str>,
    method_filter: Option<&str>,
) -> anyhow::Result<OpenApiOperation> {
    let mut matches = Vec::new();
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("OpenAPI spec missing `paths` object"))?;
    for (path, path_item) in paths {
        let Some(path_obj) = path_item.as_object() else {
            continue;
        };
        for method in ["get", "post", "put", "delete", "patch", "head"] {
            let Some(operation) = path_obj.get(method) else {
                continue;
            };
            let by_operation = operation_id
                .map(|wanted| {
                    operation
                        .get("operationId")
                        .and_then(Value::as_str)
                        .map(|id| id == wanted)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            let by_path_method = path_filter.map(|wanted| wanted == path).unwrap_or(false)
                && method_filter
                    .map(|wanted| wanted.eq_ignore_ascii_case(method))
                    .unwrap_or(false);
            let no_filter =
                operation_id.is_none() && path_filter.is_none() && method_filter.is_none();
            if by_operation || by_path_method || no_filter {
                matches.push(OpenApiOperation {
                    path: path.clone(),
                    method: method.to_string(),
                    operation: resolve_maybe_ref(spec, operation),
                    path_item: path_item.clone(),
                });
            }
        }
    }
    if matches.len() == 1 {
        Ok(matches.remove(0))
    } else if matches.is_empty() {
        anyhow::bail!("no OpenAPI operation matched; pass --operation-id or --path plus --method");
    } else {
        anyhow::bail!(
            "OpenAPI selection matched {} operations; pass --operation-id or --path plus --method",
            matches.len()
        );
    }
}

fn openapi_parameters(spec: &Value, path_item: &Value, operation: &Value) -> Vec<Value> {
    path_item
        .get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            operation
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .map(|p| resolve_maybe_ref(spec, p))
        .collect()
}

fn openapi_json_request_body_schema(spec: &Value, operation: &Value) -> Option<Value> {
    let body = resolve_maybe_ref(spec, operation.get("requestBody")?);
    let schema = body
        .get("content")?
        .get("application/json")?
        .get("schema")?;
    Some(toml_safe_json_value(resolve_maybe_ref(spec, schema)))
}

fn openapi_input_schema(parameters: &[Value], body_schema: Option<Value>) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = BTreeSet::new();
    for p in parameters {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        let schema = p
            .get("schema")
            .cloned()
            .map(toml_safe_json_value)
            .unwrap_or_else(|| json!({"type": "string"}));
        properties.insert(name.to_string(), schema);
        if p.get("required").and_then(Value::as_bool).unwrap_or(false)
            || p.get("in").and_then(Value::as_str) == Some("path")
        {
            required.insert(name.to_string());
        }
    }
    if let Some(schema) = body_schema {
        properties.insert("body".to_string(), schema);
        required.insert("body".to_string());
    }
    let mut out = serde_json::Map::new();
    out.insert("type".to_string(), Value::String("object".to_string()));
    out.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        out.insert(
            "required".to_string(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    out.insert("additionalProperties".to_string(), Value::Bool(false));
    Value::Object(out)
}

fn openapi_url_template(base_url: &str, path: &str, parameters: &[Value]) -> String {
    let mut rendered_path = convert_openapi_path_template(path);
    let query_names: Vec<String> = parameters
        .iter()
        .filter(|p| p.get("in").and_then(Value::as_str) == Some("query"))
        .filter_map(|p| p.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    if !query_names.is_empty() {
        let sep = if rendered_path.contains('?') {
            '&'
        } else {
            '?'
        };
        rendered_path.push(sep);
        rendered_path.push_str(
            &query_names
                .into_iter()
                .map(|n| format!("{n}={{{{ {n} }}}}"))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        rendered_path.trim_start_matches('/')
    )
}

fn convert_openapi_path_template(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut name = String::new();
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == '}' {
                    break;
                }
                name.push(next);
            }
            out.push_str("{{ ");
            out.push_str(name.trim());
            out.push_str(" }}");
        } else {
            out.push(ch);
        }
    }
    out
}

fn first_server_url(spec: &Value) -> Option<String> {
    spec.get("servers")
        .and_then(Value::as_array)?
        .first()?
        .get("url")?
        .as_str()
        .map(str::to_string)
}

fn resolve_maybe_ref(root: &Value, value: &Value) -> Value {
    let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
        return value.clone();
    };
    if !reference.starts_with("#/") {
        return value.clone();
    }
    let mut cursor = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        let key = segment.replace("~1", "/").replace("~0", "~");
        let Some(next) = cursor.get(&key) else {
            return value.clone();
        };
        cursor = next;
    }
    cursor.clone()
}

fn http_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "result": {},
            "fulfilled_by": { "type": "string" },
            "status": { "type": "integer" },
            "headers": { "type": "object" },
            "elapsed_ms": { "type": "integer" }
        },
        "required": ["result", "fulfilled_by", "status", "headers", "elapsed_ms"],
        "additionalProperties": true
    })
}

fn shell_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "result": {},
            "fulfilled_by": { "type": "string" },
            "exit_code": { "type": "integer" },
            "elapsed_ms": { "type": "integer" }
        },
        "required": ["result", "fulfilled_by", "exit_code", "elapsed_ms"],
        "additionalProperties": true
    })
}

fn toml_safe_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(k, v)| {
                    if v.is_null() {
                        None
                    } else {
                        Some((k, toml_safe_json_value(v)))
                    }
                })
                .collect(),
        ),
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
        if let Some(ch) = mapped {
            if ch == '_' || ch == '-' {
                if !last_was_sep && !out.is_empty() {
                    out.push('_');
                    last_was_sep = true;
                }
            } else {
                out.push(ch);
                last_was_sep = false;
            }
        }
    }
    while out.ends_with('_') || out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_add_infers_schema_from_templates() {
        let args = ApiAddArgs {
            name: "weather".to_string(),
            method: "GET".to_string(),
            url: "https://wttr.in/{{ city }}?format=4".to_string(),
            description: None,
            headers: vec!["X-Trace={{ trace_id }}".to_string()],
            body: None,
            input_schema: None,
            response: "text_trim".to_string(),
            timeout: Some(10),
            dry_run: false,
            overwrite: false,
            cost_kind: None,
            cost_label: None,
        };

        let manifest = api_manifest_for(&args).expect("manifest");

        assert_eq!(manifest.name(), "weather");
        assert_eq!(manifest.timeout_seconds(), Some(10));
        assert_eq!(
            manifest.input_schema()["properties"]["city"]["type"],
            json!("string")
        );
        assert_eq!(
            manifest.input_schema()["properties"]["trace_id"]["type"],
            json!("string")
        );
        match manifest.exec().expect("exec") {
            AbilityExec::Http(exec) => {
                assert_eq!(exec.method, "GET");
                assert!(exec.url.contains("{{ city }}"));
                assert_eq!(
                    exec.headers.as_ref().unwrap().get("X-Trace").unwrap(),
                    "{{ trace_id }}"
                );
            }
            other => panic!("expected http exec, got {other:?}"),
        }
    }

    #[test]
    fn script_add_builds_shell_manifest_with_inferred_schema() {
        let args = ScriptAddArgs {
            name: "weather".to_string(),
            argv: vec![
                "curl".to_string(),
                "-s".to_string(),
                "https://wttr.in/{{ city }}?format=j1".to_string(),
            ],
            description: None,
            input_schema: None,
            stdout: None,
            sandbox: Some("net_only".to_string()),
            timeout: Some(30),
            dry_run: false,
            overwrite: false,
            cost_kind: None,
            cost_label: None,
        };

        let manifest = script_manifest_for(&args).expect("manifest");

        assert_eq!(manifest.name(), "weather");
        assert_eq!(manifest.timeout_seconds(), Some(30));
        assert_eq!(
            manifest.input_schema()["properties"]["city"]["type"],
            json!("string")
        );
        assert_eq!(manifest.input_schema()["required"], json!(["city"]));
        match manifest.exec().expect("exec") {
            AbilityExec::Shell(exec) => {
                assert_eq!(exec.argv[0], "curl");
                assert!(exec.argv[2].contains("{{ city }}"));
                assert_eq!(exec.sandbox.as_deref(), Some("net_only"));
            }
            other => panic!("expected shell exec, got {other:?}"),
        }
        let toml = manifest.to_toml_string().expect("toml");
        let reparsed = AbilityManifest::from_toml_str(&toml).expect("round-trip");
        assert_eq!(&reparsed, &manifest);
    }

    #[test]
    fn script_add_rejects_unknown_sandbox_profile() {
        let args = ScriptAddArgs {
            name: "bad".to_string(),
            argv: vec!["echo".to_string(), "hi".to_string()],
            description: None,
            input_schema: None,
            stdout: None,
            sandbox: Some("full_isolation".to_string()),
            timeout: None,
            dry_run: false,
            overwrite: false,
            cost_kind: None,
            cost_label: None,
        };

        let err = script_manifest_for(&args).expect_err("unknown sandbox must fail");
        assert!(err.to_string().contains("sandbox"), "got: {err}");
    }

    #[test]
    fn openapi_import_builds_http_manifest_from_operation_id() {
        let spec = parse_openapi_document(
            r#"
openapi: "3.0.0"
servers:
  - url: https://api.example.test
paths:
  /users/{id}:
    get:
      operationId: getUser
      summary: Get one user
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
        - name: verbose
          in: query
          schema:
            type: boolean
"#,
        )
        .expect("yaml");
        let op = select_openapi_operation(&spec, Some("getUser"), None, None).expect("op");
        let args = FromOpenApiArgs {
            spec: "unused.yaml".to_string(),
            operation_id: Some("getUser".to_string()),
            path: None,
            method: None,
            name: None,
            base_url: None,
            prefix: "api".to_string(),
            dry_run: false,
            overwrite: false,
            cost_kind: None,
            cost_label: None,
        };

        let manifest = openapi_manifest_for(&spec, &op, &args).expect("manifest");

        assert_eq!(manifest.name(), "getuser");
        assert_eq!(
            manifest.input_schema()["properties"]["id"]["type"],
            json!("string")
        );
        assert_eq!(
            manifest.input_schema()["properties"]["verbose"]["type"],
            json!("boolean")
        );
        match manifest.exec().expect("exec") {
            AbilityExec::Http(exec) => {
                assert_eq!(exec.method, "GET");
                assert_eq!(
                    exec.url,
                    "https://api.example.test/users/{{ id }}?verbose={{ verbose }}"
                );
            }
            other => panic!("expected http exec, got {other:?}"),
        }
    }

    #[test]
    fn agent_ura_selector_resolves_to_agent_id() {
        assert_eq!(
            resolve_agent_selector("easynet:///r/acme/agent/alice.backend-engineer").unwrap(),
            "backend-engineer"
        );
        assert_eq!(resolve_agent_selector("codex").unwrap(), "codex");
    }
}
