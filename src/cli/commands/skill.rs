// EasyNet CLI — `skill` command: daemon-hosted skill management.
// =================================================================
//
// File: src/cli/skill.rs
// Description: `easynet skill {install,list,upgrade,remove}`.
//
// The CLI layer is intentionally thin: it maps terminal arguments to
// daemon-hosted Axon abilities and renders their responses. The
// filesystem package-store implementation lives in
// `runtime::skill_store`, which is what the daemon abilities call.

use clap::{Args, Subcommand};
use console::style;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::daemon::resources::skills::store::{format_bytes, InstallRecord};
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

fn invoke_daemon_skill_install(args: &InstallArgs) -> anyhow::Result<InstallRecord> {
    let response = crate::support::platform::local_invoke::invoke_local_ability(
        "skill.install",
        json!({
            "source": args.source,
            "agent": args.agent,
            "pin": args.pin,
        }),
    )?;
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

#[derive(Debug, Deserialize)]
struct SkillListResponse {
    #[serde(default)]
    items: Vec<InstallRecord>,
}

fn invoke_daemon_skill_list(args: &ListArgs) -> anyhow::Result<Vec<InstallRecord>> {
    let response = crate::support::platform::local_invoke::invoke_local_ability(
        "skill.list",
        json!({
            "owner_agent_id": args.agent,
            "agent_ura": args.agent_ura,
            "subject_ura": args.subject_ura,
        }),
    )?;
    let decoded: SkillListResponse = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("skill.list returned invalid payload: {err}"))?;
    Ok(decoded.items)
}

fn run_upgrade(args: UpgradeArgs) -> anyhow::Result<()> {
    let record = invoke_daemon_skill_upgrade(&args)?;
    emit_upgrade_result(&args, &record)
}

fn invoke_daemon_skill_upgrade(args: &UpgradeArgs) -> anyhow::Result<InstallRecord> {
    let response = crate::support::platform::local_invoke::invoke_local_ability(
        "skill.upgrade",
        json!({
            "name": args.name,
            "agent": args.agent,
            "to": args.to,
        }),
    )?;
    decode_skill_record_response(response, "skill.upgrade")
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    crate::support::platform::local_invoke::invoke_local_ability(
        "skill.remove",
        json!({
            "name": args.name,
            "agent": args.agent,
        }),
    )?;
    output::success(&format!(
        "Removed skill '{}' from agent '{}'",
        args.name, args.agent
    ));
    Ok(())
}

fn decode_skill_record_response(
    response: serde_json::Value,
    ability: &str,
) -> anyhow::Result<InstallRecord> {
    let record = response
        .get("record")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{ability} response missing `record`"))?;
    serde_json::from_value(record)
        .map_err(|err| anyhow::anyhow!("{ability} returned invalid record: {err}"))
}

fn emit_install_result(args: &InstallArgs, rec: &InstallRecord) -> anyhow::Result<()> {
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

fn emit_upgrade_result(args: &UpgradeArgs, rec: &InstallRecord) -> anyhow::Result<()> {
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
