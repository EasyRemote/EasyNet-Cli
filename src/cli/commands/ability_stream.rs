// EasyNet CLI
// ===========
//
// File: src/cli/ability_stream.rs
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

use crate::cli::commands::invocation_tuple::{
    require_causal_root, required_nonce_hex, required_subject, AbilityInvocationRef,
};
use crate::support::platform::local_invoke::{
    invoke_local_ability_target_stream_explicit_root, LocalAbilityTarget, LocalStreamFrame,
};
use crate::support::platform::{output, timeouts};

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
    /// Pin the stream invocation to a remote Device or Hub URA through the
    /// local daemon's canonical InvokeStream RPC.
    #[arg(long, short = 'n', value_name = "URA")]
    pub node: Option<String>,
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
    /// Explicit 16-byte invocation nonce as 32 hex characters.
    #[arg(long, value_name = "HEX")]
    pub nonce_hex: Option<String>,
    /// Declare this stream as a root invocation with an empty causal parent set.
    #[arg(long)]
    pub causal_root: bool,
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
    let ability_ref = AbilityInvocationRef::parse(&args.ability_ura)?;
    let ability_selector = ability_ref.selector();
    let node_ura: Option<String> = match args.node.as_deref().map(str::trim) {
        None => None,
        Some("") => anyhow::bail!(
            "--node was given but empty; omit the flag to stream locally, \
             or pass a real `easynet:///r/<tenant>/device/<node>` URA"
        ),
        Some(node) => {
            #[cfg(feature = "axon-pb")]
            {
                Some(crate::daemon::invocation::routing::remote_invoke::parse_node_ura(node)?)
            }
            #[cfg(not(feature = "axon-pb"))]
            {
                let _ = node;
                anyhow::bail!(
                    "remote stream pinning via --node requires the `axon-pb` feature, \
                     which is not enabled in this build. Re-build with \
                     `--features axon-pb` and try again."
                )
            }
        }
    };
    let arguments: Value = match args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };
    let timeout_ms = timeouts::effective_ms(args.timeout)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    let target = LocalAbilityTarget::from_selector(ability_selector);
    let frames = match node_ura.as_deref() {
        #[cfg(feature = "axon-pb")]
        Some(remote_node) => {
            let credentials = crate::daemon::persistence::config::load_credentials()
                .context("remote ability stream requires paired device credentials")?;
            let caller_ura =
                crate::support::platform::remote_device::caller_device_ura(&credentials)?;
            let remote_target = ability_ref
                .remote_target_for_mode(remote_node, crate::daemon::ability::CallMode::Stream)?;
            let surface = "remote ability stream with --node";
            let subject = required_subject(args.subject.as_deref(), surface)?.to_string();
            let invocation_nonce = required_nonce_hex(args.nonce_hex.as_deref(), surface)?;
            require_causal_root(args.causal_root, surface)?;
            let request =
                crate::daemon::invocation::routing::remote_invoke::RemoteInvocationTuplePlan::public_explicit(
                    &remote_target,
                    caller_ura,
                    subject,
                    invocation_nonce,
                    crate::daemon::invocation::routing::remote_invoke::declared_root_causal_context(),
                    arguments,
                    Duration::from_millis(timeout_ms),
                )?
                .into_request()?;
            crate::daemon::invocation::routing::remote_invoke::invoke_remote_target_stream(
                request,
                args.max_frames,
            )?
        }
        #[cfg(not(feature = "axon-pb"))]
        Some(_) => unreachable!("--node bail handled above when axon-pb is off"),
        None => {
            if ability_ref.is_descriptor_ref() {
                anyhow::bail!(
                    "local ability stream does not accept descriptor refs; omit `@version` \
                     for local dispatch or pass `--node` for remote descriptor-bound origin proof"
                );
            }
            let surface = "local ability stream";
            let subject = required_subject(args.subject.as_deref(), surface)?;
            let invocation_nonce = required_nonce_hex(args.nonce_hex.as_deref(), surface)?;
            require_causal_root(args.causal_root, surface)?;
            invoke_local_ability_target_stream_explicit_root(
                &target,
                arguments,
                subject,
                invocation_nonce,
                Duration::from_millis(timeout_ms),
                args.max_frames,
            )?
        }
    };
    print_frames(&frames, args.raw, args.format)?;
    let fulfilled_by = node_ura
        .as_deref()
        .map(|node| format!("canonical InvokeStream target={node}"))
        .unwrap_or_else(|| "local daemon".to_string());
    output::success(&format!(
        "{} -> streamed {} frame(s) ({fulfilled_by})",
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
            node: None,
            args: None,
            timeout: 60,
            subject: None,
            nonce_hex: None,
            causal_root: false,
            max_frames: Some(0),
            raw: false,
            format: StreamOutputFormat::Ndjson,
        })
        .expect_err("zero max frame count must fail before daemon IPC");
        assert!(format!("{err}").contains("--max-frames"));
    }
}
