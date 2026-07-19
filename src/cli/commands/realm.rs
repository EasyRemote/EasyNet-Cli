// EasyNet CLI — Realm diagnostics
// ===============================
//
// File: src/cli/commands/realm.rs
// Description: Read-only Realm resolution and inspection diagnostics.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use clap::{Args, Subcommand};

use crate::cli::commands::profile;
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct RealmArgs {
    #[command(subcommand)]
    pub action: RealmAction,
}

#[derive(Debug, Subcommand)]
pub enum RealmAction {
    /// Resolve a Realm alias/domain to the issuer the CLI would use.
    Resolve(RealmResolveArgs),
    /// Inspect local Realm resolution facts.
    Inspect(RealmResolveArgs),
}

#[derive(Debug, Args)]
pub struct RealmResolveArgs {
    /// Realm alias or domain.
    pub realm: String,
    /// Explicit Hub/Auth endpoint override for diagnostics.
    #[arg(long)]
    pub hub: Option<String>,
}

pub fn run(args: RealmArgs) -> anyhow::Result<()> {
    match args.action {
        RealmAction::Resolve(args) | RealmAction::Inspect(args) => run_resolve(args),
    }
}

fn run_resolve(args: RealmResolveArgs) -> anyhow::Result<()> {
    let resolved = profile::resolve_realm(&args.realm, args.hub.as_deref())?;
    output::kv_section_stdout(&[
        ("realm", resolved.realm_alias.as_str()),
        ("issuer", resolved.issuer.as_str()),
        ("source", resolved.discovery_source.as_str()),
    ]);
    if let Some(realm_id) = resolved.realm_id.as_deref() {
        output::kv_section_stdout(&[("realm_id", realm_id)]);
    }
    Ok(())
}
