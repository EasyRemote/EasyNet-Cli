// EasyNet CLI
// ===========
//
// File: src/cli/ability_bidi.rs
// Description: `easynet ability bidi <ability-ura> [--args JSON]`.

use std::time::Duration;

use anyhow::Context;
use clap::{Args, ValueEnum};
use serde_json::{json, Value};

use crate::cli::commands::invocation_tuple::{
    require_causal_root, required_nonce_hex, required_subject,
};
use crate::support::platform::local_invoke::{
    invoke_local_ability_target_bidi_json_frames_explicit_root, LocalAbilityTarget, LocalBidiFrame,
};
use crate::support::platform::{output, timeouts};

const DEFAULT_BIDI_DRAIN_FRAMES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BidiOutputFormat {
    /// One compact JSON object per line.
    Ndjson,
    /// One pretty JSON array after the bounded drain completes.
    Json,
}

#[derive(Debug, Args)]
pub struct BidiArgs {
    /// Canonical Ability URA returned by `easynet ability list`.
    pub ability_ura: String,
    /// JSON object passed as the bidi session's initial arguments.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Optional JSON frame to send after open. Can be repeated.
    #[arg(long = "input", value_name = "JSON")]
    pub input_frames: Vec<String>,
    /// Per-session transport deadline in seconds. '0' inherits the runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// AXIOM envelope subject, expressed as a canonical resource URA.
    #[arg(long, value_name = "URA")]
    pub subject: Option<String>,
    /// Explicit 16-byte invocation nonce as 32 hex characters.
    #[arg(long, value_name = "HEX")]
    pub nonce_hex: Option<String>,
    /// Declare this bidi session as a root invocation with an empty causal parent set.
    #[arg(long)]
    pub causal_root: bool,
    /// Stop after this many down frames. Defaults to a bounded diagnostic sample.
    #[arg(long, value_name = "N")]
    pub max_frames: Option<usize>,
    /// Wait until the ability emits a terminal receipt instead of using a bounded sample.
    #[arg(long)]
    pub until_terminal: bool,
    /// Print full transport frame objects. By default only frame payloads print.
    #[arg(long)]
    pub raw: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = BidiOutputFormat::Ndjson)]
    pub format: BidiOutputFormat,
}

pub fn run(args: BidiArgs) -> anyhow::Result<()> {
    if args.max_frames == Some(0) {
        anyhow::bail!("--max-frames must be greater than 0 when provided");
    }
    let ability_selector = crate::core::ura::AbilitySelector::parse(&args.ability_ura)
        .context("parse <ability-ura>")?;
    let arguments: Value = match args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };
    let input_frames = args
        .input_frames
        .iter()
        .map(|raw| serde_json::from_str(raw).context("parse --input JSON"))
        .collect::<anyhow::Result<Vec<Value>>>()?;
    let timeout_ms = timeouts::effective_ms(args.timeout)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    let target = LocalAbilityTarget::from_selector(&ability_selector);
    let surface = "local ability bidi";
    let subject = required_subject(args.subject.as_deref(), surface)?;
    let invocation_nonce = required_nonce_hex(args.nonce_hex.as_deref(), surface)?;
    require_causal_root(args.causal_root, surface)?;
    let frames = invoke_local_ability_target_bidi_json_frames_explicit_root(
        &target,
        arguments,
        subject,
        invocation_nonce,
        Duration::from_millis(timeout_ms),
        input_frames,
        drain_limit(args.max_frames, args.until_terminal),
    )?;
    print_frames(&frames, args.raw, args.format)?;
    output::success(&format!(
        "{} -> bidi drained {} frame(s) (local daemon)",
        ability_selector.ability_ura(),
        frames.len()
    ));
    Ok(())
}

fn print_frames(
    frames: &[LocalBidiFrame],
    raw: bool,
    format: BidiOutputFormat,
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
        BidiOutputFormat::Ndjson => {
            for value in values {
                println!("{}", serde_json::to_string(&value)?);
            }
        }
        BidiOutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&values)?);
        }
    }
    Ok(())
}

fn drain_limit(max_frames: Option<usize>, until_terminal: bool) -> Option<usize> {
    if until_terminal {
        None
    } else {
        Some(max_frames.unwrap_or(DEFAULT_BIDI_DRAIN_FRAMES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_bidi_frame_shape_preserves_terminal_flag() {
        let frame = LocalBidiFrame {
            sequence: 2,
            content_type: "application/json".to_string(),
            terminal: true,
            payload: json!({"type": "closed"}),
        };
        let raw = json!({
            "sequence": frame.sequence,
            "content_type": frame.content_type,
            "terminal": frame.terminal,
            "payload": frame.payload,
        });
        assert_eq!(raw["sequence"], 2);
        assert_eq!(raw["terminal"], true);
        assert_eq!(raw["payload"]["type"], "closed");
    }

    #[test]
    fn zero_max_frames_is_rejected_before_ipc() {
        let err = run(BidiArgs {
            ability_ura: "easynet:///r/acme/ability/device.dev.remote_desktop.attach".to_string(),
            args: None,
            input_frames: Vec::new(),
            timeout: 60,
            subject: None,
            nonce_hex: None,
            causal_root: false,
            max_frames: Some(0),
            until_terminal: false,
            raw: false,
            format: BidiOutputFormat::Ndjson,
        })
        .expect_err("zero max frame count must fail before daemon IPC");
        assert!(format!("{err}").contains("--max-frames"));
    }

    #[test]
    fn bidi_default_drain_is_bounded_for_long_lived_sessions() {
        assert_eq!(
            drain_limit(None, false),
            Some(DEFAULT_BIDI_DRAIN_FRAMES),
            "long-lived bidi abilities such as remote_desktop.attach must not hang by default"
        );
        assert_eq!(drain_limit(Some(4), false), Some(4));
        assert_eq!(drain_limit(Some(4), true), None);
    }
}
