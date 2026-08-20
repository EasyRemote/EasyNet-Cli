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

use anyhow::Context;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::daemon::ability::builtins::governance::api_key;
use crate::daemon::ability::builtins::resources::pages::{PagesIdentity, PagesUserRootIdentity};
use crate::support::platform::local_invoke::{
    LocalDaemonSystemAbilityIssuer, LocalRuntimeApiKeyInventoryReadIssuer,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiKeyPrincipal {
    user: String,
    subject_ura: String,
}

impl ApiKeyPrincipal {
    const SUBJECT_RESOURCE_PATH: &'static str = "api-key/manage";

    fn from_user_root_identity(identity: PagesUserRootIdentity) -> anyhow::Result<Self> {
        let subject_ura = crate::core::ura::resource_dot_ura(
            &identity.realm,
            &format!("user.{}", identity.user),
            Self::SUBJECT_RESOURCE_PATH,
        );
        crate::core::ura::parse_ura(&subject_ura)
            .map_err(|error| anyhow::anyhow!("api-key subject URA is invalid: {error}"))?;
        Ok(Self {
            user: identity.user,
            subject_ura,
        })
    }

    fn ability(&self, action: &str) -> String {
        format!("{}.api_key.{action}", self.user)
    }
}

fn current_api_key_principal() -> anyhow::Result<ApiKeyPrincipal> {
    if let Some(identity) = PagesIdentity::try_from_env()?.user_root_identity()? {
        return ApiKeyPrincipal::from_user_root_identity(identity);
    }
    anyhow::bail!(
        "no user identity bound to this daemon — run 'easynet device pair' first \
         (or set EASYNET_PAGES_USER and EASYNET_PAGES_REALM for dev rigs)"
    )
}

fn invoke_api_key_manage(
    principal: &ApiKeyPrincipal,
    ability: &str,
    args: Value,
) -> anyhow::Result<Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(ability, args, &principal.subject_ura)
        .with_context(|| format!("invoke {ability}"))
}

pub fn run(args: ApiKeyArgs) -> anyhow::Result<()> {
    match args.command {
        ApiKeyCommand::Create(a) => run_create(a),
        ApiKeyCommand::List(a) => run_list(a),
        ApiKeyCommand::Revoke(a) => run_revoke(a),
    }
}

fn run_create(a: CreateArgs) -> anyhow::Result<()> {
    let principal = current_api_key_principal()?;
    let ability = principal.ability("create");
    let mut args = json!({});
    if let Some(label) = a.label {
        args["label"] = json!(label);
    }
    let result = invoke_api_key_manage(&principal, &ability, args)?;
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
    let principal = current_api_key_principal()?;
    let ability = principal.ability("list");
    let result = LocalRuntimeApiKeyInventoryReadIssuer::list_api_keys(&ability, json!({}))?;
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
    let principal = current_api_key_principal()?;
    let ability = principal.ability("revoke");
    let result = invoke_api_key_manage(&principal, &ability, json!({ "id_prefix": a.id_prefix }))?;
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

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }

        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

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
    fn current_api_key_principal_accepts_complete_dev_override() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::set("EASYNET_PAGES_USER", " alice ");
        let _realm = EnvGuard::set("EASYNET_PAGES_REALM", " localhost ");
        save_credentials(&paired_credentials(Some("alice")))
            .expect("dev override still requires an immutable runtime owner");

        let principal = current_api_key_principal().expect("env override");

        assert_eq!(principal.user, "alice");
        assert_eq!(
            principal.subject_ura,
            "easynet:///r/localhost/resource/user.alice/api-key/manage"
        );
        assert_eq!(principal.ability("create"), "alice.api_key.create");
    }

    #[test]
    fn current_api_key_principal_rejects_partial_dev_override() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::set("EASYNET_PAGES_USER", " alice ");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");

        let error = current_api_key_principal().expect_err("partial env override");

        assert!(
            error.to_string().contains("requires an explicit realm"),
            "partial env override must fail closed: {error:#}"
        );
    }

    #[test]
    fn current_api_key_principal_reads_valid_paired_credentials() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        save_credentials(&paired_credentials(Some("alice"))).expect("save credentials");

        let principal = current_api_key_principal().expect("paired credentials");

        assert_eq!(principal.user, "alice");
        assert_eq!(
            principal.subject_ura,
            "easynet:///r/localhost/resource/user.alice/api-key/manage"
        );
    }

    #[test]
    fn current_api_key_principal_reports_unpaired_only_when_credentials_file_is_absent() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");

        let error = current_api_key_principal().expect_err("missing credentials");

        assert!(
            error.to_string().contains("no user identity bound"),
            "unexpected missing-credential error: {error:#}"
        );
    }

    #[test]
    fn current_api_key_principal_rejects_malformed_existing_credentials() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let dir = state_dir();
        fs::create_dir_all(&dir).expect("create state dir");
        fs::write(dir.join("credentials.json"), "{").expect("write malformed credentials");

        let error = current_api_key_principal().expect_err("malformed credentials");

        assert!(
            error.to_string().contains("parse credentials"),
            "malformed credentials must fail closed instead of looking unpaired: {error:#}"
        );
    }

    #[test]
    fn current_api_key_principal_rejects_credentials_without_username() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
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

        let error = current_api_key_principal().expect_err("missing username");

        assert!(
            error.to_string().contains("missing username"),
            "missing username must fail closed instead of looking unpaired: {error:#}"
        );
    }
}
