// EasyNet CLI — `easynet pages` subcommand
// =========================================
//
// File: src/cli/commands/pages.rs
// Description: ergonomic wrapper around the Pages reference
//              system's abilities (RFC-006-B v0.6). Each
//              subcommand:
//
//                  create  → PagesAbility::Publish
//                  list    → PagesAbility::List
//                  show    → PagesAbility::Get
//                  delete  → PagesAbility::Unpublish
//                  url     → local lookup, no ability call
//
// CLI shape mirrors `easynet device` / `easynet ability`:
// human-readable by default, `--json` for scripting, exit 0 on
// success, exit non-zero with stderr explanation on failure.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::daemon::ability::builtins::resources::pages::{PagesIdentity, PagesUserRootIdentity};
use crate::daemon::invocation::routing::target::{CallMode, SystemInvocationTargetIssuer};
use crate::support::platform::local_invoke::{LocalAbilityTarget, LocalDaemonSystemAbilityIssuer};

/// User-owned Pages ability verbs exposed by the local daemon.
///
/// What this is: the one CLI-side projection from the human `pages`
/// commands to daemon-local Pages ability keys.
///
/// What this is not: it is not an Axon `/ability/` URA builder. Axon
/// currently treats User URAs as owners of resources and agents, not
/// direct Ability publishers, so the Pages runtime still registers
/// user-owned abilities as local daemon registry keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PagesAbilityVerb {
    Publish,
    List,
    Get,
    Unpublish,
}

impl PagesAbilityVerb {
    fn public_name(self) -> &'static str {
        match self {
            Self::Publish => "pages.publish",
            Self::List => "project_list",
            Self::Get => "pages.get",
            Self::Unpublish => "pages.unpublish",
        }
    }
}

/// Typed local selector for user-scoped Pages abilities.
///
/// Invariant 1: `user` is non-empty and comes from the daemon's
/// paired identity or `EASYNET_PAGES_USER` dev override.
///
/// Invariant 2: `local_registry_ability` is the only place in the CLI
/// facade that selects the owner-local `pages.<verb>` registry key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PagesAbility {
    user: String,
    verb: PagesAbilityVerb,
}

impl PagesAbility {
    fn for_user(user: &str, verb: PagesAbilityVerb) -> anyhow::Result<Self> {
        let user = user.trim();
        if user.is_empty() {
            anyhow::bail!("pages ability selector requires a non-empty user identity");
        }
        Ok(Self {
            user: user.to_string(),
            verb,
        })
    }

    fn local_registry_ability(&self) -> String {
        match self.verb {
            PagesAbilityVerb::List => "pages.project_list".to_string(),
            _ => self.verb.public_name().to_string(),
        }
    }

    fn local_target(&self, realm: &str) -> anyhow::Result<LocalAbilityTarget> {
        let callee = crate::core::ura::agent_ura(realm, &self.user, "pages");
        LocalAbilityTarget::new(self.local_registry_ability(), callee)
    }
}

#[derive(Debug, Args)]
pub struct PagesArgs {
    #[command(subcommand)]
    pub command: PagesCommand,
}

#[derive(Debug, Subcommand)]
pub enum PagesCommand {
    // The actual public URL the Hub serves is path-based:
    //   https://<realm>/web/<user>/<project_id>/...
    // (see `backend/internal/handler/pages_public/serve.go`). The
    // `<project>.<user>.pages.localhost:<port>` form an earlier
    // draft of this comment mentioned is the *daemon's* in-process
    // HTTP listener for local dev, not the production URL — routing
    // there requires a wildcard DNS / TLS cert that production does
    // not (and likely will not) ship. Keep `--help` aligned with
    // what the operator can actually `curl`: the hub's `/web/` path.
    /// Publish a folder of static bytes as a website. Mints a
    /// resource URA easynet:///r/<realm>/resource/<user>.<project_id>/
    /// and registers the project's <user>.<project_id>.page.fetch
    /// ability. The Hub serves traffic at https://<realm>/web/<user>/<project_id>/.
    Create(CreateArgs),

    /// List currently-published projects on this daemon.
    List(ListArgs),

    /// Print one project's detail (folder, visibility, URL, etc.).
    Show(ShowArgs),

    /// Unpublish a project. Releases the folder fd and unregisters
    /// the fetch ability; subsequent HTTP requests return 503.
    Delete(DeleteArgs),

    /// Print the project's production URL root only — scriptable
    /// for shell composition like: open $(easynet pages url papers).
    Url(UrlArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Project identifier (URA-safe segment, alnum + underscore + dash, max 64).
    pub project_id: String,
    /// Absolute path to the folder to publish.
    #[arg(long)]
    pub folder: String,
    /// Visibility — public (default); private/scoped reserved for post-MVP and rejected with a clear error.
    #[arg(long, default_value = "public")]
    pub visibility: String,
    /// Emit JSON instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Emit JSON instead of the human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    pub project_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    pub project_id: String,
    /// Skip confirmation when stdin is a TTY.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct UrlArgs {
    pub project_id: String,
}

pub fn run(args: PagesArgs) -> anyhow::Result<()> {
    match args.command {
        PagesCommand::Create(a) => run_create(a),
        PagesCommand::List(a) => run_list(a),
        PagesCommand::Show(a) => run_show(a),
        PagesCommand::Delete(a) => run_delete(a),
        PagesCommand::Url(a) => run_url(a),
    }
}

fn current_pages_user_root_identity() -> anyhow::Result<PagesUserRootIdentity> {
    PagesIdentity::try_from_env()?
        .user_root_identity()?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no user identity bound to this daemon — run 'easynet device pair' first \
                 (or set EASYNET_PAGES_USER and EASYNET_PAGES_REALM for dev rigs)"
            )
        })
}

fn invoke_pages_ability(
    identity: &PagesUserRootIdentity,
    ability: &PagesAbility,
    args: Value,
) -> anyhow::Result<Value> {
    let target = ability.local_target(&identity.realm)?;
    let invocation = SystemInvocationTargetIssuer::local_target_root(&target, args, CallMode::Rpc)?;
    LocalDaemonSystemAbilityIssuer::invoke_issued_target_root_timeout(
        &invocation,
        std::time::Duration::from_secs(30),
    )
}

fn run_create(a: CreateArgs) -> anyhow::Result<()> {
    let identity = current_pages_user_root_identity()?;
    let ability = PagesAbility::for_user(&identity.user, PagesAbilityVerb::Publish)?;
    let args_v = json!({
        "folder":     a.folder,
        "project_id": a.project_id,
        "visibility": a.visibility,
    });
    let result = invoke_pages_ability(&identity, &ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages create failed: {e}"))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let project_ura = result
            .get("project_ura")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let url_root = result
            .get("url_root")
            .and_then(Value::as_str)
            .unwrap_or("?");
        println!("Published.");
        println!("  project_ura:  {project_ura}");
        println!("  url_root:     {url_root}");
    }
    Ok(())
}

fn run_list(a: ListArgs) -> anyhow::Result<()> {
    let identity = current_pages_user_root_identity()?;
    let ability = PagesAbility::for_user(&identity.user, PagesAbilityVerb::List)?;
    let result = invoke_pages_ability(&identity, &ability, json!({}))
        .map_err(|e| anyhow::anyhow!("pages list failed: {e}"))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let empty: Vec<Value> = Vec::new();
    let projects = result
        .get("projects")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if projects.is_empty() {
        println!("No published projects.");
        return Ok(());
    }
    // Two URL columns: LOCAL is the daemon's dev listener (the URL
    // that actually opens during local dev); PUBLIC is the Hub
    // production URL (resolves once the realm is publicly served).
    println!("{:<22} {:<8} {:<46} PUBLIC", "PROJECT", "VIS", "LOCAL");
    for p in projects {
        let id = p.get("project_id").and_then(Value::as_str).unwrap_or("?");
        let vis = p.get("visibility").and_then(Value::as_str).unwrap_or("?");
        let local = p
            .get("dev_listener_url_root")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let public = p.get("url_root").and_then(Value::as_str).unwrap_or("?");
        println!("{id:<22} {vis:<8} {local:<46} {public}");
    }
    Ok(())
}

fn run_show(a: ShowArgs) -> anyhow::Result<()> {
    let identity = current_pages_user_root_identity()?;
    let ability = PagesAbility::for_user(&identity.user, PagesAbilityVerb::Get)?;
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_pages_ability(&identity, &ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages show failed: {e}"))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    println!(
        "Project: {}",
        result
            .get("project_id")
            .and_then(Value::as_str)
            .unwrap_or("?")
    );
    println!(
        "  user:         {}",
        result.get("user").and_then(Value::as_str).unwrap_or("?")
    );
    println!(
        "  project_ura:  {}",
        result
            .get("project_ura")
            .and_then(Value::as_str)
            .unwrap_or("?")
    );
    println!(
        "  folder:       {}",
        result.get("folder").and_then(Value::as_str).unwrap_or("?")
    );
    println!(
        "  visibility:   {}",
        result
            .get("visibility")
            .and_then(Value::as_str)
            .unwrap_or("?")
    );
    println!(
        "  url_root:     {}",
        result
            .get("url_root")
            .and_then(Value::as_str)
            .unwrap_or("?")
    );
    // Dev-only daemon-local listener URL. Reachable from this host
    // only when EASYNET_PAGES_PORT is set and the daemon spawned
    // its in-process HTTP listener; intentionally omitted from
    // `pages list` and `pages url` because operators consuming
    // those surfaces want the production URL. `pages show` is the
    // verbose surface — we surface both.
    if let Some(dev) = result.get("dev_listener_url_root").and_then(Value::as_str) {
        println!("  dev_listener: {dev}");
    }
    println!(
        "  size_cap:     {} bytes",
        result
            .get("file_size_cap")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    Ok(())
}

fn run_delete(a: DeleteArgs) -> anyhow::Result<()> {
    if !a.force {
        // No real interactive prompt in v0 — `--force` is required
        // when the caller wants the destructive operation. This
        // matches the conservative default the test matrix expects
        // (Matrix B C7).
        anyhow::bail!(
            "delete is destructive — pass '--force' to confirm; this MVP does not prompt interactively"
        );
    }
    let identity = current_pages_user_root_identity()?;
    let ability = PagesAbility::for_user(&identity.user, PagesAbilityVerb::Unpublish)?;
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_pages_ability(&identity, &ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages delete failed: {e}"))?;
    let removed = result
        .get("removed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if removed {
        println!("Unpublished {}.", a.project_id);
        Ok(())
    } else {
        anyhow::bail!("delete returned without confirming removal: {result}");
    }
}

fn run_url(a: UrlArgs) -> anyhow::Result<()> {
    let identity = current_pages_user_root_identity()?;
    let ability = PagesAbility::for_user(&identity.user, PagesAbilityVerb::Get)?;
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_pages_ability(&identity, &ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages url failed: {e}"))?;
    let url = result
        .get("url_root")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("daemon returned no url_root"))?;
    println!("{url}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config::{self, Credentials};

    fn credentials(username: &str, realm: &str) -> Credentials {
        Credentials {
            node_id: "device-a".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "https://hub.example".to_string(),
            realm: realm.to_string(),
            deploy_signature: "sig".to_string(),
            hub_api_base: None,
            username: Some(username.to_string()),
            user_id: Some("user-a".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    #[test]
    fn pages_ability_projects_to_local_registry_key() {
        let ability =
            PagesAbility::for_user("alice", PagesAbilityVerb::Publish).expect("pages ability");
        assert_eq!(ability.local_registry_ability(), "pages.publish");
    }

    #[test]
    fn pages_ability_targets_pages_agent_callee() {
        let ability =
            PagesAbility::for_user("alice", PagesAbilityVerb::List).expect("pages ability");
        let target = ability.local_target("localhost").expect("local target");

        assert_eq!(target.dispatch_name(), "pages.project_list");
        assert_eq!(
            target.callee_ura(),
            "easynet:///r/localhost/agent/alice.pages"
        );
    }

    #[test]
    fn pages_ability_rejects_empty_user() {
        let err = PagesAbility::for_user("   ", PagesAbilityVerb::List)
            .expect_err("empty user must fail");
        assert!(format!("{err}").contains("non-empty user"));
    }

    #[test]
    fn pages_cli_identity_projects_credentials_user_and_realm() {
        let _home = HomeGuard::new();
        config::save_credentials(&credentials("alice", "localhost")).expect("save credentials");

        let identity = current_pages_user_root_identity().expect("pages identity");

        assert_eq!(identity.user, "alice");
        assert_eq!(identity.realm, "localhost");
    }

    #[test]
    fn pages_cli_identity_rejects_env_user_without_realm() {
        let _home = HomeGuard::new();
        std::env::set_var("EASYNET_PAGES_USER", "alice");

        let error = current_pages_user_root_identity()
            .expect_err("env user without realm must not default to public realm");

        assert!(
            error.to_string().contains("requires an explicit realm"),
            "wrong error: {error:#}"
        );
    }

    #[test]
    fn pages_cli_identity_rejects_malformed_credentials_instead_of_defaulting() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::state_dir().join("credentials.json"), b"{")
            .expect("malformed credentials");

        let error =
            current_pages_user_root_identity().expect_err("malformed credentials must fail closed");

        assert!(
            error.to_string().contains("parse credentials"),
            "wrong error: {error:#}"
        );
    }
}
