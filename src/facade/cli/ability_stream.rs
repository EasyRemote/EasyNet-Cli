// EasyNet CLI
// ===========
//
// File: src/facade/cli/ability_stream.rs
// Description: `easynet ability stream <ability-ura> [--args JSON]`.
//
// This is the stream-mode sibling of `ability invoke`: it submits a
// complete Axon InvokeStream request to the local daemon and prints the
// decoded JSON frames. Long-lived interactive surfaces use bidi abilities;
// this command remains for ordinary server-stream ability output.

use std::time::Duration;

use anyhow::Context;
use clap::{Args, ValueEnum};
use serde_json::{json, Value};

use crate::support::local_invoke::{
    invoke_local_ability_target_stream_with_subject, LocalAbilityTarget, LocalStreamFrame,
};
use crate::support::{output, timeouts};

/// Output shape for `ability stream`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StreamOutputFormat {
    /// One compact JSON object per line. Best for frontend adapters
    /// and shell pipelines that consume frames incrementally.
    Ndjson,
    /// One pretty JSON array after the stream drains.
    Json,
}

/// Arguments for the generic local daemon InvokeStream CLI.
#[derive(Debug, Args)]
pub struct StreamArgs {
    /// Canonical Ability URA returned by `easynet ability list`.
    pub ability_ura: String,
    /// JSON object passed to the stream ability as its arguments.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Per-stream transport deadline in seconds. '0' inherits the
    /// runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// AXIOM envelope subject, expressed as a canonical resource URA.
    #[arg(long, value_name = "URA")]
    pub subject: Option<String>,
    /// Stop after this many frames even if the daemon stream is live.
    /// Omit to wait for the daemon's terminal frame.
    #[arg(long, value_name = "N")]
    pub max_frames: Option<usize>,
    /// Print full transport frame objects. By default only each
    /// frame's business payload is printed.
    #[arg(long)]
    pub raw: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = StreamOutputFormat::Ndjson)]
    pub format: StreamOutputFormat,
}

pub fn run(args: StreamArgs) -> anyhow::Result<()> {
    if args.max_frames == Some(0) {
        anyhow::bail!("--max-frames must be greater than 0 when provided");
    }
    let ability_selector =
        crate::ura::AbilitySelector::parse(&args.ability_ura).context("parse <ability-ura>")?;
    let arguments: Value = match args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };
    let timeout_ms = timeouts::effective_ms(args.timeout)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    let target = LocalAbilityTarget::from_selector(&ability_selector);
    let frames = invoke_local_ability_target_stream_with_subject(
        &target,
        arguments,
        args.subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        Duration::from_millis(timeout_ms),
        args.max_frames,
    )?;
    print_frames(&frames, args.raw, args.format)?;
    output::success(&format!(
        "{} -> streamed {} frame(s) (local daemon)",
        ability_selector.ability_ura(),
        frames.len()
    ));
    Ok(())
}

fn print_frames(
    frames: &[LocalStreamFrame],
    raw: bool,
    format: StreamOutputFormat,
) -> anyhow::Result<()> {
    let values: Vec<Value> = frames
        .iter()
        .map(|frame| {
            if raw {
                json!({
                    "sequence": frame.sequence,
                    "content_type": frame.content_type,
                    "terminal": frame.terminal,
                    "payload": frame.payload,
                })
            } else {
                frame.payload.clone()
            }
        })
        .collect();
    match format {
        StreamOutputFormat::Ndjson => {
            for value in values {
                println!("{}", serde_json::to_string(&value)?);
            }
        }
        StreamOutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&values)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_payload_frames_as_json_array() {
        let frames = [LocalStreamFrame {
            sequence: 7,
            content_type: "application/json".to_string(),
            terminal: true,
            payload: json!({"type": "stream.frame"}),
        }];
        let values: Vec<Value> = frames.iter().map(|frame| frame.payload.clone()).collect();
        assert_eq!(values, vec![json!({"type": "stream.frame"})]);
    }

    #[test]
    fn raw_frame_shape_preserves_transport_metadata() {
        let frame = LocalStreamFrame {
            sequence: 3,
            content_type: "application/json".to_string(),
            terminal: false,
            payload: json!({"ok": true}),
        };
        let raw = json!({
            "sequence": frame.sequence,
            "content_type": frame.content_type,
            "terminal": frame.terminal,
            "payload": frame.payload,
        });
        assert_eq!(raw["sequence"], 3);
        assert_eq!(raw["terminal"], false);
        assert_eq!(raw["payload"]["ok"], true);
    }

    #[test]
    fn zero_max_frames_is_rejected_before_ipc() {
        let err = run(StreamArgs {
            ability_ura: "easynet:///r/acme/ability/device.local.test.stream".to_string(),
            args: None,
            timeout: 60,
            subject: None,
            max_frames: Some(0),
            raw: false,
            format: StreamOutputFormat::Ndjson,
        })
        .expect_err("zero max frame count must fail before daemon IPC");
        assert!(format!("{err}").contains("--max-frames"));
    }
}
