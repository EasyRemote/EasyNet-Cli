// EasyNet CLI — `easynet pages` subcommand
// =========================================
//
// File: src/facade/cli/pages.rs
// Description: ergonomic wrapper around the Pages reference
//              system's abilities (RFC-006-B v0.6). Each
//              subcommand:
//
//                  create  → <user>.pages.publish
//                  list    → <user>.pages.list
//                  show    → <user>.pages.get
//                  delete  → <user>.pages.unpublish
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

use crate::support::local_invoke::invoke_local_ability;

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

fn current_user() -> anyhow::Result<String> {
    // Production: read username from `EASYNET_PAGES_USER` env or
    // `credentials.json`. M5 of the system-namespace migration
    // banned the `<self>` placeholder — an unpaired daemon has no
    // user-rooted ability surface, so the CLI MUST surface the
    // missing-identity error rather than silently dialling
    // `self.pages.*` (which the registry no longer answers).
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

fn run_create(a: CreateArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.pages.publish");
    let args_v = json!({
        "folder":     a.folder,
        "project_id": a.project_id,
        "visibility": a.visibility,
    });
    let result = invoke_local_ability(&ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages create failed: {e}"))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let project_uri = result
            .get("project_uri")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let url_root = result
            .get("url_root")
            .and_then(Value::as_str)
            .unwrap_or("?");
        println!("Published.");
        println!("  project_uri:  {project_uri}");
        println!("  url_root:     {url_root}");
    }
    Ok(())
}

fn run_list(a: ListArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.pages.list");
    let result = invoke_local_ability(&ability, json!({}))
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
    println!("{:<24} {:<10} URL", "PROJECT", "VISIBILITY");
    for p in projects {
        let id = p.get("project_id").and_then(Value::as_str).unwrap_or("?");
        let vis = p.get("visibility").and_then(Value::as_str).unwrap_or("?");
        let url = p.get("url_root").and_then(Value::as_str).unwrap_or("?");
        println!("{id:<24} {vis:<10} {url}");
    }
    Ok(())
}

fn run_show(a: ShowArgs) -> anyhow::Result<()> {
    let user = current_user()?;
    let ability = format!("{user}.pages.get");
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_local_ability(&ability, args_v)
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
        "  project_uri:  {}",
        result
            .get("project_uri")
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
    let user = current_user()?;
    let ability = format!("{user}.pages.unpublish");
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_local_ability(&ability, args_v)
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
    let user = current_user()?;
    let ability = format!("{user}.pages.get");
    let args_v = json!({ "project_id": a.project_id });
    let result = invoke_local_ability(&ability, args_v)
        .map_err(|e| anyhow::anyhow!("pages url failed: {e}"))?;
    let url = result
        .get("url_root")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("daemon returned no url_root"))?;
    println!("{url}");
    Ok(())
}
