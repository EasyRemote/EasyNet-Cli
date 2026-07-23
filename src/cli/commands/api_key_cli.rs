// EasyNet CLI — `easynet api-key` subcommand
// =============================================
//
// File: src/cli/api_key_cli.rs
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

use crate::daemon::ability::builtins::governance::api_key;
use crate::support::platform::local_invoke::invoke_local_ability;

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
    // `legacy self alias` placeholder — an unpaired daemon has no
    // user-rooted ability surface, so the CLI MUST surface the
    // missing-identity error rather than silently dialling
    // `self.api_key.*` (which the registry no longer answers).
    if let Some(v) = std::env::var("EASYNET_PAGES_USER")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(v);
    }
    if let Some(credentials) = crate::daemon::persistence::config::load_credentials_optional()? {
        return Ok(credentials.username_slug()?.to_string());
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
    let key_ura = result.get("key_ura").and_then(Value::as_str).unwrap_or("?");

    if !a.no_cache {
        api_key::write_local_default_token(token)?;
    }

    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("API key created.");
        println!("  key_ura: {key_ura}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config::{save_credentials, state_dir, Credentials};
    use std::fs;

    fn paired_credentials(username: Option<&str>) -> Credentials {
        Credentials {
            node_id: "node".into(),
            credential_token: "token".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            realm: "localhost".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: username.map(str::to_string),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    #[test]
    fn current_user_accepts_explicit_dev_override() {
        let _home = HomeGuard::new();
        std::env::set_var("EASYNET_PAGES_USER", " alice ");

        let user = current_user().expect("env override");

        assert_eq!(user, "alice");
    }

    #[test]
    fn current_user_reads_valid_paired_credentials() {
        let _home = HomeGuard::new();
        save_credentials(&paired_credentials(Some("alice"))).expect("save credentials");

        let user = current_user().expect("paired credentials");

        assert_eq!(user, "alice");
    }

    #[test]
    fn current_user_reports_unpaired_only_when_credentials_file_is_absent() {
        let _home = HomeGuard::new();

        let error = current_user().expect_err("missing credentials");

        assert!(
            error.to_string().contains("no user identity bound"),
            "unexpected missing-credential error: {error:#}"
        );
    }

    #[test]
    fn current_user_rejects_malformed_existing_credentials() {
        let _home = HomeGuard::new();
        let dir = state_dir();
        fs::create_dir_all(&dir).expect("create state dir");
        fs::write(dir.join("credentials.json"), "{").expect("write malformed credentials");

        let error = current_user().expect_err("malformed credentials");

        assert!(
            error.to_string().contains("parse credentials"),
            "malformed credentials must fail closed instead of looking unpaired: {error:#}"
        );
    }

    #[test]
    fn current_user_rejects_credentials_without_username() {
        let _home = HomeGuard::new();
        let dir = state_dir();
        fs::create_dir_all(&dir).expect("create state dir");
        fs::write(
            dir.join("credentials.json"),
            r#"{
  "node_id": "node",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "localhost",
  "deploy_signature": "",
  "user_id": "user-alice"
}
"#,
        )
        .expect("write incomplete credentials");

        let error = current_user().expect_err("missing username");

        assert!(
            error.to_string().contains("missing username"),
            "missing username must fail closed instead of looking unpaired: {error:#}"
        );
    }
}
