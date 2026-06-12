// EasyNet CLI — `easynet agent` publish surface
// Split from facade/cli/agent.rs (F-033 / T4.6); bodies are move-only.

use console::style;

use super::*;

pub(super) fn run_publish(args: PublishArgs) -> anyhow::Result<()> {
    if !args.dry_run {
        // Live publishing is gated until the cross-repo publish
        // spec + implementation lands. Returning a clear error
        // here — rather than silently calling through to a
        // future Axon path — keeps the flag's contract honest:
        // today every successful `agent publish` is a dry-run.
        anyhow::bail!(
            "only '--dry-run' is supported in this release. Live publishing through \
             Axon lands in a later PR. Re-run with `--dry-run` to preview the \
             `<agent>.<ability>` tools that would be registered."
        );
    }

    let dir = open_registered_agent(&args.name)?;
    let manifests = dir.list_ability_manifests()?;

    eprintln!();
    eprintln!(
        "  {} {}  {}",
        style("dry-run:").yellow(),
        style(format!("agent publish {}", args.name)).white().bold(),
        style(format!("root={}", dir.root().display())).dim(),
    );
    eprintln!();

    if manifests.is_empty() {
        eprintln!(
            "  {}",
            style("Nothing to advertise: abilities/ is empty or missing.").dim(),
        );
        eprintln!();
        return Ok(());
    }

    // Emit one line per planned ToolSpec registration. The
    // lines are `<qualified>\t<input_schema_shape>\t<output>` so
    // a downstream consumer (`diff`, an ops script) can parse
    // them with awk. The decorative styling only affects TTY
    // output; `console::style` degrades to plain ASCII when the
    // sink is not a terminal.
    eprintln!(
        "  {:<28} {:<18} {}",
        style("QUALIFIED NAME").dim(),
        style("INPUT SHAPE").dim(),
        style("OUTPUT SHAPE").dim(),
    );
    eprintln!("  {}", style("─".repeat(72)).dim());

    for m in &manifests {
        let qualified = m.qualified_name(&args.name);
        // Render a one-line shape summary for each schema. A
        // full JSON Schema tree would flood the terminal; the
        // summary is "object(keys=prompt,context)" style. That
        // line is enough to spot a schema regression at a
        // glance; full content lives on disk for anyone who
        // wants to inspect it.
        let input_shape = summarize_schema(m.input_schema());
        let output_shape = m
            .output_schema()
            .map(summarize_schema)
            .unwrap_or_else(|| "-".to_string());
        eprintln!(
            "  {:<28} {:<18} {}",
            style(qualified).cyan(),
            style(input_shape).white(),
            style(output_shape).dim(),
        );
    }

    eprintln!();
    eprintln!(
        "  {} {}",
        style("would advertise").green(),
        style(format!(
            "{} ability{} in the node roster label",
            manifests.len(),
            if manifests.len() == 1 { "" } else { "s" }
        ))
        .white()
        .bold(),
    );
    eprintln!(
        "  {}",
        style("(dry-run — no Axon calls, no registry mutation)").dim(),
    );
    eprintln!();
    Ok(())
}

/// One-line shape summary for a JSON Schema root — used by the
/// publish dry-run table. Deliberately coarse: the intent is "spot
/// a regression at a glance", not "fully re-render the schema".
pub(super) fn summarize_schema(schema: &serde_json::Value) -> String {
    let obj = match schema.as_object() {
        Some(o) => o,
        None => return format!("{:?}", schema),
    };
    let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    // Dead-by-contract: AbilityManifest::validate() rejects any
    // input_schema or output_schema whose top-level is not an
    // object, so both schemas reaching this helper are objects.
    // Kept as a belt-and-braces fallback so a future API widening
    // ("accept a top-level $ref") doesn't panic the dry-run table;
    // the render degrades to a single type word instead.
    if ty != "object" {
        return ty.to_string();
    }
    let mut keys: Vec<&str> = obj
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort();
    let required: std::collections::HashSet<&str> = obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    // Mark required keys with a trailing `!` so the summary
    // distinguishes "prompt (required)" from "context (optional)"
    // without expanding the column width.
    let rendered: Vec<String> = keys
        .iter()
        .map(|k| {
            if required.contains(k) {
                format!("{k}!")
            } else {
                (*k).to_string()
            }
        })
        .collect();
    if rendered.is_empty() {
        "object".to_string()
    } else {
        format!("object({})", rendered.join(","))
    }
}
