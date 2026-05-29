// EasyNet CLI — `easynet api-key` subcommand
// =============================================
//
// File: src/facade/cli/api_key_cli.rs
// Description: thin wrapper around `<user>.api_key.{create, list, revoke}`
//              abilities. CLI mints / lists / revokes OpenAI-compat
//              bearer tokens. The `create` subcommand also writes
//              the freshly-minted token to a local cache file at
//              `~/.easynet/api_keys.local.toml` so subsequent
//              `easynet llm-api` calls find a default key without
//              the operator passing `--key`.
//
// Conformance: RFC-006-C v0.1 INV-2.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::runtime::agents::api_key_ability;
use crate::support::local_invoke::invoke_local_ability;

#[derive(Debug, Args)]
pub struct ApiKeyArgs {
    #[command(subcommand)]
    pub command: ApiKeyCommand,
}

#[derive(Debug, Subcommand)]
pub enum ApiKeyCommand {
    /// Mint a new API key for OpenAI-compatible access.
    Create(CreateArgs),
    /// List API keys (hashes only — tokens are never re-shown).
    List(ListArgs),
    /// Revoke a key by its id_prefix.
    Revoke(RevokeArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Optional human-readable label.
    #[arg(long)]
    pub label: Option<String>,
    /// Skip writing the local default-token cache.
    #[arg(long)]
    pub no_cache: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RevokeArgs {
    pub id_prefix: String,
}

fn current_user() -> anyhow::Result<String> {
    // Production: read username from `EASYNET_PAGES_USER` env (e2e
    // / multi-user dev rigs) or `credentials.json` (paired
    // device). M5 of the system-namespace migration banned the
    // `<self>` placeholder — an unpaired daemon has no
    // user-rooted ability surface, so the CLI MUST surface the
    // missing-identity error rather than silently dialling
    // `self.api_key.*` (which the registry no longer answers).
    if let Some(v) = std::env::var("EASYNET_PAGES_USER")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return Ok(v);
    }
    if let Some(v) = crate::persistence::config::load_credentials()
        .ok()
        .and_then(|c| c.username)
        .filter(|s| !s.is_empty())
    {
        return Ok(v);
    }
    anyhow::bail!(
        "no user identity bound to this daemon — run 'easynet device pair' first \
         (or set EASYNET_PAGES_USER for dev rigs)"
    )
}

pub fn run(args: ApiKeyArgs) -> anyhow::Result<()> {
    match args.command {
        ApiKeyCommand::Create(a) => run_create(a),
        ApiKeyCommand::List(a) => run_list(a),
        ApiKeyCommand::Revoke(a) => run_revoke(a),
    }
}

fn run_create(a: CreateArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.api_key.create");
    let mut args = json!({});
    if let Some(label) = a.label {
        args["label"] = json!(label);
    }
    let result = invoke_local_ability(&ability, args)?;
    let token = result
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("daemon returned no token"))?;
    let key_uri = result.get("key_uri").and_then(Value::as_str).unwrap_or("?");

    if !a.no_cache {
        api_key_ability::write_local_default_token(token)?;
    }

    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("API key created.");
        println!("  key_uri: {key_uri}");
        println!("  token:   {token}");
        if !a.no_cache {
            println!("  cached:  ~/.easynet/api_keys.local.toml (used by 'easynet llm-api')");
        }
        println!("  ⚠ this is the only time the token is shown — save it now.");
    }
    Ok(())
}

fn run_list(a: ListArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.api_key.list");
    let result = invoke_local_ability(&ability, json!({}))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let empty: Vec<Value> = Vec::new();
    let keys = result
        .get("keys")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if keys.is_empty() {
        println!("No API keys.");
        return Ok(());
    }
    println!(
        "{:<14} {:<10} {:<22} CREATED",
        "ID_PREFIX", "STATUS", "LABEL"
    );
    for k in keys {
        let id_prefix = k.get("id_prefix").and_then(Value::as_str).unwrap_or("?");
        let revoked = k.get("revoked").and_then(Value::as_bool).unwrap_or(false);
        let label = k
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("<no label>");
        let created = k.get("created_at").and_then(Value::as_u64).unwrap_or(0);
        println!(
            "{id_prefix:<14} {:<10} {label:<22} {created}",
            if revoked { "revoked" } else { "active" }
        );
    }
    Ok(())
}

fn run_revoke(a: RevokeArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.api_key.revoke");
    let result = invoke_local_ability(&ability, json!({ "id_prefix": a.id_prefix }))?;
    let revoked = result
        .get("revoked")
        .and_then(Value::as_str)
        .unwrap_or(&a.id_prefix);
    println!("Revoked {revoked}.");
    Ok(())
}
