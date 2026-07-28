// EasyNet CLI — `skill` command: daemon-hosted skill management.
// =================================================================
//
// File: src/cli/skill.rs
// Description: `easynet skill {install,list,upgrade,remove}`.
//
// The CLI layer is intentionally thin: it maps terminal arguments to
// daemon-hosted Axon abilities and renders their responses. The
// filesystem package-store implementation lives in
// `daemon::resources::skills::store`, which is what the daemon abilities call.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde::Serialize;
use serde_json::{json, Value};

use crate::daemon::resources::skills::projection::{
    InstalledSkillProjection, SkillListResponse, SkillRecordResponse, SkillRemoveReceipt,
};
use crate::daemon::resources::skills::store::format_bytes;
use crate::support::platform::local_invoke::{
    LocalDaemonSystemAbilityIssuer, LocalRuntimeSkillCatalogueReadIssuer,
};
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub action: SkillAction,
}

#[derive(Debug, Subcommand)]
pub enum SkillAction {
    /// Install a skill from a marketplace source into an agent's skills/.
    Install(InstallArgs),
    /// List installed skills, optionally filtered by agent.
    List(ListArgs),
    /// Upgrade an installed skill to a newer ref.
    Upgrade(UpgradeArgs),
    /// Remove an installed skill from an agent.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Source URL: `github:<owner>/<repo>[@<ref>][:<subpath>]`.
    pub source: String,

    /// Agent name that will own this skill (see 'easynet agent list').
    #[arg(long)]
    pub agent: String,

    /// Override the ref in the source URL with a concrete tag / SHA.
    #[arg(long)]
    pub pin: Option<String>,

    /// Emit a single-line JSON blob on stdout with the installed
    /// skill's metadata (for machine consumers like the backend).
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Agent name. Omit to list skills across every registered agent.
    #[arg(long)]
    pub agent: Option<String>,

    /// Canonical owner Agent URA. Filters to that hosted agent.
    #[arg(long = "agent-ura")]
    pub agent_ura: Option<String>,

    /// Owner Agent URA or skill package Resource URA.
    #[arg(long = "subject-ura")]
    pub subject_ura: Option<String>,

    /// Emit a JSON array on stdout instead of a human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Skill name as installed under `<agent-root>/skills/<name>/`.
    pub name: String,

    /// Agent name that owns the skill.
    #[arg(long)]
    pub agent: String,

    /// Target ref — tag / SHA / branch. Omit for "latest upstream".
    #[arg(long)]
    pub to: Option<String>,

    /// Emit JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Skill name to remove.
    pub name: String,

    /// Agent name that owns the skill.
    #[arg(long)]
    pub agent: String,
}

pub fn run(args: SkillArgs) -> anyhow::Result<()> {
    match args.action {
        SkillAction::Install(a) => run_install(a),
        SkillAction::List(a) => run_list(a),
        SkillAction::Upgrade(a) => run_upgrade(a),
        SkillAction::Remove(a) => run_remove(a),
    }
}

fn run_install(args: InstallArgs) -> anyhow::Result<()> {
    let record = invoke_daemon_skill_install(&args)?;
    emit_install_result(&args, &record)
}

fn invoke_daemon_skill_install(args: &InstallArgs) -> anyhow::Result<InstalledSkillProjection> {
    let response = invoke_daemon_skill_mutation("skill.install", skill_install_payload(args))?;
    decode_skill_record_response(response, "skill.install")
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let rows = invoke_daemon_skill_list(&args)?;

    if args.json {
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }

    if rows.is_empty() {
        output::info("No skills installed.");
        return Ok(());
    }

    eprintln!();
    eprintln!(
        "  {:<24} {:<18} {:<40} {:<12}",
        style("SKILL").dim(),
        style("AGENT").dim(),
        style("SOURCE").dim(),
        style("SIZE").dim(),
    );
    eprintln!("  {}", style("─".repeat(98)).dim());
    for row in &rows {
        eprintln!(
            "  {:<24} {:<18} {:<40} {:<12}",
            style(&row.name).white().bold(),
            style(&row.agent_id).cyan(),
            style(row.source.to_url()).dim(),
            style(format_bytes(row.size_bytes)).dim(),
        );
    }
    eprintln!();
    Ok(())
}

fn invoke_daemon_skill_list(args: &ListArgs) -> anyhow::Result<Vec<InstalledSkillProjection>> {
    let response = LocalRuntimeSkillCatalogueReadIssuer::list_installed_skills(json!({
        "owner_agent_id": args.agent,
        "agent_ura": args.agent_ura,
        "subject_ura": args.subject_ura,
    }))?;
    let decoded: SkillListResponse = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("skill.list returned invalid payload: {err}"))?;
    Ok(decoded.items)
}

fn run_upgrade(args: UpgradeArgs) -> anyhow::Result<()> {
    let record = invoke_daemon_skill_upgrade(&args)?;
    emit_upgrade_result(&args, &record)
}

fn invoke_daemon_skill_upgrade(args: &UpgradeArgs) -> anyhow::Result<InstalledSkillProjection> {
    let response = invoke_daemon_skill_mutation("skill.upgrade", skill_upgrade_payload(args))?;
    decode_skill_record_response(response, "skill.upgrade")
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let response = invoke_daemon_skill_mutation("skill.remove", skill_remove_payload(&args))?;
    let receipt: SkillRemoveReceipt = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("skill.remove returned invalid receipt: {err}"))?;
    if !receipt.ok {
        anyhow::bail!("skill.remove returned non-ok receipt");
    }
    output::success(&format!(
        "Removed skill '{}' from agent '{}'",
        receipt.name, receipt.agent
    ));
    Ok(())
}

fn invoke_daemon_skill_mutation(ability: &str, args: Value) -> anyhow::Result<Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(ability, args)
        .with_context(|| format!("invoke {ability}"))
}

fn skill_install_payload(args: &InstallArgs) -> Value {
    json!({
        "source": args.source,
        "agent": args.agent,
        "pin": args.pin,
    })
}

fn skill_upgrade_payload(args: &UpgradeArgs) -> Value {
    json!({
        "name": args.name,
        "agent": args.agent,
        "to": args.to,
    })
}

fn skill_remove_payload(args: &RemoveArgs) -> Value {
    json!({
        "name": args.name,
        "agent": args.agent,
    })
}

fn decode_skill_record_response(
    response: serde_json::Value,
    ability: &str,
) -> anyhow::Result<InstalledSkillProjection> {
    let decoded: SkillRecordResponse = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("{ability} returned invalid response: {err}"))?;
    if !decoded.ok {
        anyhow::bail!("{ability} returned non-ok response");
    }
    Ok(decoded.record)
}

fn emit_install_result(args: &InstallArgs, rec: &InstalledSkillProjection) -> anyhow::Result<()> {
    if args.json {
        #[derive(Serialize)]
        struct MachineOut<'a> {
            name: &'a str,
            agent_id: &'a str,
            content_hash: &'a str,
            size_bytes: u64,
            installed_at: &'a str,
            #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
            ref_: Option<&'a String>,
        }
        let machine = MachineOut {
            name: &rec.name,
            agent_id: &rec.agent_id,
            content_hash: &rec.skill_tree_hash,
            size_bytes: rec.size_bytes,
            installed_at: &rec.installed_at,
            ref_: rec.source.ref_.as_ref(),
        };
        println!("{}", serde_json::to_string(&machine)?);
    } else {
        output::success(&format!(
            "Installed skill '{}' on agent '{}'",
            rec.name, rec.agent_id
        ));
        output::detail("source", &rec.source.to_url());
        output::detail("hash", &rec.skill_tree_hash);
        output::detail("size", &format_bytes(rec.size_bytes));
    }
    Ok(())
}

fn emit_upgrade_result(args: &UpgradeArgs, rec: &InstalledSkillProjection) -> anyhow::Result<()> {
    if args.json {
        println!("{}", serde_json::to_string(rec)?);
    } else {
        output::success(&format!(
            "Upgraded skill '{}' on agent '{}' to {}",
            rec.name,
            rec.agent_id,
            rec.source.ref_.as_deref().unwrap_or("latest"),
        ));
        output::detail("hash", &rec.skill_tree_hash);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_install_payload_preserves_public_wire_shape() {
        let payload = skill_install_payload(&InstallArgs {
            source: "github:owner/repo@v1:skills/demo".to_string(),
            agent: "codex".to_string(),
            pin: Some("abc123".to_string()),
            json: false,
        });

        assert_eq!(
            payload,
            json!({
                "source": "github:owner/repo@v1:skills/demo",
                "agent": "codex",
                "pin": "abc123",
            })
        );
    }

    #[test]
    fn skill_upgrade_payload_preserves_public_wire_shape() {
        let payload = skill_upgrade_payload(&UpgradeArgs {
            name: "demo".to_string(),
            agent: "codex".to_string(),
            to: Some("v2".to_string()),
            json: true,
        });

        assert_eq!(
            payload,
            json!({
                "name": "demo",
                "agent": "codex",
                "to": "v2",
            })
        );
    }

    #[test]
    fn skill_remove_payload_preserves_public_wire_shape() {
        let payload = skill_remove_payload(&RemoveArgs {
            name: "demo".to_string(),
            agent: "codex".to_string(),
        });

        assert_eq!(
            payload,
            json!({
                "name": "demo",
                "agent": "codex",
            })
        );
    }
}
