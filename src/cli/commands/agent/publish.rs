//! `easynet agent publish` live capability publication surface.
//!
//! The command never reconstructs abilities from manifests. Non-dry-run first
//! asks the daemon to reconcile the selected Agent, then both modes render the
//! canonical `meta.list_abilities` projection for that Agent owner.

use console::style;
use serde_json::{json, Value};

use super::*;

pub(super) fn run_publish(args: PublishArgs) -> anyhow::Result<()> {
    let action_gateway = agent_command_gateway();
    let read_gateway = agent_read_gateway();
    let owner_ura = resolve_agent_owner_ura(read_gateway.as_ref(), &args.name)?;
    if !args.dry_run {
        action_gateway.invoke("agent.refresh", json!({"name": &args.name}))?;
    }
    let response = read_gateway.list_agent_abilities(&owner_ura)?;
    let mut abilities = response
        .get("abilities")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("meta.list_abilities returned no `abilities` array"))?;
    abilities.sort_by_key(descriptor_sort_key);

    eprintln!();
    eprintln!(
        "  {} {}  {}",
        if args.dry_run {
            style("dry-run:").yellow()
        } else {
            style("published:").green()
        },
        style(format!("agent publish {}", args.name)).white().bold(),
        style(format!("owner={owner_ura}")).dim(),
    );
    eprintln!();

    if abilities.is_empty() {
        eprintln!(
            "  {}",
            style("No committed live abilities are published for this Agent.").dim(),
        );
        eprintln!();
        return Ok(());
    }

    eprintln!(
        "  {:<28} {:<8} {:<10} {}",
        style("QUALIFIED NAME").dim(),
        style("MODE").dim(),
        style("VERSION").dim(),
        style("INPUT SHAPE").dim(),
    );
    eprintln!("  {}", style("-".repeat(82)).dim());

    for descriptor in &abilities {
        let public_name = descriptor
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let call_mode = descriptor
            .get("call_mode")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let version = descriptor
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let input = descriptor
            .pointer("/schema_summary/input")
            .unwrap_or(&Value::Null);
        eprintln!(
            "  {:<28} {:<8} {:<10} {}",
            style(format!("{}.{}", args.name, public_name)).cyan(),
            style(call_mode).white(),
            style(version).dim(),
            style(summarize_schema(input)).white(),
        );
    }

    eprintln!();
    eprintln!(
        "  {}",
        if args.dry_run {
            style(format!(
                "observed {} committed live descriptor(s); no state was mutated",
                abilities.len()
            ))
            .dim()
        } else {
            style(format!(
                "reconciled and published {} live descriptor(s)",
                abilities.len()
            ))
            .green()
        },
    );
    eprintln!();
    Ok(())
}

fn resolve_agent_owner_ura(gateway: &dyn AgentReadGateway, name: &str) -> anyhow::Result<String> {
    let response = gateway.list_agents()?;
    response
        .get("agents")
        .and_then(Value::as_array)
        .and_then(|agents| {
            agents
                .iter()
                .find(|agent| agent.get("name").and_then(Value::as_str) == Some(name))
        })
        .and_then(|agent| agent.get("ura"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("agent '{name}' is not registered or has no canonical URA"))
}

fn descriptor_sort_key(descriptor: &Value) -> (String, String, String) {
    (
        descriptor
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        descriptor
            .get("call_mode")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        descriptor
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    )
}

pub(super) fn summarize_schema(schema: &Value) -> String {
    let Some(object) = schema.as_object() else {
        return if schema.is_null() {
            "-".to_string()
        } else {
            format!("{schema:?}")
        };
    };
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("?");
    if kind != "object" {
        return kind.to_string();
    }
    let mut keys = object
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    let required = object
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let rendered = keys
        .into_iter()
        .map(|key| {
            if required.contains(key) {
                format!("{key}!")
            } else {
                key.to_string()
            }
        })
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        "object".to_string()
    } else {
        format!("object({})", rendered.join(","))
    }
}
