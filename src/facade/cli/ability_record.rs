// EasyNet CLI
// ===========
//
// File: src/facade/cli/ability_record.rs
// Description: Ergonomic resource-backed recording wrapper.

use std::time::Duration;

use anyhow::Context;
use clap::Args;
use serde_json::{json, Value};

use crate::support::local_invoke::{
    invoke_local_ability, invoke_local_ability_target_stream_with_subject, LocalAbilityTarget,
};
use crate::support::{output, timeouts};

#[derive(Debug, Args)]
pub struct RecordArgs {
    /// Canonical mic.subscribe Ability URA returned by `easynet ability list`.
    pub ability_ura: String,
    /// JSON object passed to mic.subscribe as initial arguments.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Explicit mic resource URA. Omit to use the first local mic from meta.list_resources.
    #[arg(long, value_name = "URA")]
    pub subject: Option<String>,
    /// Stop after this many mic frames. Defaults to roughly a short recording window.
    #[arg(long, value_name = "N", default_value_t = 250)]
    pub max_frames: usize,
    /// Per-stream transport deadline in seconds. '0' inherits the runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Print captured frames. Defaults to summary-only to avoid dumping PCM to the terminal.
    #[arg(long)]
    pub print_frames: bool,
}

pub fn run(args: RecordArgs) -> anyhow::Result<()> {
    if args.max_frames == 0 {
        anyhow::bail!("--max-frames must be greater than 0");
    }
    let ability_selector =
        crate::ura::AbilitySelector::parse(&args.ability_ura).context("parse <ability-ura>")?;
    if ability_selector.public_name() != "mic.subscribe" {
        anyhow::bail!(
            "ability record currently supports mic.subscribe only; got {}",
            ability_selector.public_name()
        );
    }

    let arguments: Value = match args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };
    let subject = match args
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(subject) => subject.to_string(),
        None => default_mic_resource_ura()?,
    };
    let timeout_ms = timeouts::effective_ms(args.timeout)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    let target = LocalAbilityTarget::from_selector(&ability_selector);
    let frames = invoke_local_ability_target_stream_with_subject(
        &target,
        arguments,
        Some(subject.clone()),
        Duration::from_millis(timeout_ms),
        Some(args.max_frames),
    )?;

    if args.print_frames {
        for frame in &frames {
            println!("{}", serde_json::to_string(&frame.payload)?);
        }
    }
    output::success(&format!(
        "{} -> recorded {} frame(s) from {subject}",
        ability_selector.ability_ura(),
        frames.len()
    ));
    Ok(())
}

fn default_mic_resource_ura() -> anyhow::Result<String> {
    let response = invoke_local_ability("meta.list_resources", json!({"types": ["mic"]}))
        .context("invoke meta.list_resources(types=[\"mic\"])")?;
    let resources = response
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("meta.list_resources response missing resources array"))?;
    resources
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("mic"))
        .filter_map(|entry| entry.get("resource_ura").and_then(Value::as_str))
        .map(str::trim)
        .find(|resource_ura| !resource_ura.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no mic resource is registered on this daemon; restart the daemon so media \
                 resource bootstrap can scan devices, or pass --subject with a mic resource_ura"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_rejects_zero_frame_budget_before_ipc() {
        let err = run(RecordArgs {
            ability_ura: "easynet:///r/acme/ability/device.dev.mic.subscribe".to_string(),
            args: None,
            subject: Some("easynet:///r/acme/resource/device.dev/streams/mic.1".to_string()),
            max_frames: 0,
            timeout: 60,
            print_frames: false,
        })
        .expect_err("zero max frame count must fail before daemon IPC");
        assert!(format!("{err}").contains("--max-frames"));
    }
}
