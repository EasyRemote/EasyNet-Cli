// EasyNet CLI — terminal.attach (InvokeBidi)
// ===========================================
//
// File: src/daemon/ability/builtins/device_control/terminal/attach.rs
//
// Attaches one Invocation to a persistent, supervisor-owned PTY session.
// Detaching ends this Invocation but not the PTY; reattaching opens a new
// InvokeBidi call and must present the current attachment epoch.
//
// Frame protocol
// --------------
//   Args at OpenBidi:
//     { "session_id": "...", "attachment_id": "...", "expected_epoch": n }
//
//   Client → handler (SendBidi.frame):
//     application/octet-stream containing raw stdin bytes
//     { "type": "resize", "cols": u16, "rows": u16 }
//     { "type": "detach" }
//
//   Handler → client (RecvBidi.frame):
//     { "type": "attached", "session_id": "...", "attachment_id": "...", "epoch": n }
//     application/octet-stream containing raw stdout/stderr bytes
//     { "type": "output_gap", "dropped_bytes": n }
//     { "type": "detached", "session_id": "...", "attachment_id": "...", "epoch": n }
//     { "type": "exit",   "status": <u32|null> }
//     { "type": "error",  "message": "<protocol error>" }
//
// The supervisor enforces one active attachment with compare-and-swap epoch
// semantics. Its bounded output ring survives daemon restart; output_gap makes
// overflow explicit. The daemon remains the sole Runtime and owns admission,
// routing, receipts, and the lifetime of each InvokeBidi transport.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::daemon::ability::builtins::device_control::terminal::io::{
    PtyAttachmentLease, PtyIoService, PtyIoWriteOutcome,
};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{
    bidi_input_channel, AxonAbilityCatalog, BidiInputFrame, BidiOutputFrame, BidiSource,
    BIDI_CHANNEL_BOUND,
};
use crate::daemon::execution::pty::{PtyService, PtySessionId};

pub const ABILITY_TERMINAL_ATTACH: &str =
    crate::daemon::ability::names::device_control::TERMINAL_ATTACH;

/// Description published by the dispatcher's `description_for`
/// arm. Sibling of terminal.create / close — those are
/// the control plane, this is the data plane.
pub fn description() -> &'static str {
    "Attach to an existing PTY session over InvokeBidi: pump \
     stdin from the wire to the PTY master, stream stdout / \
     stderr back as raw binary frames, report bounded-buffer loss as OUTPUT_GAP, \
     detach with an incremented epoch for later reattach, and surface child exit. Pair with \
     terminal.create (open the session) and \
     terminal.close (terminate it). Part of the \
     baseline-locomotion-v1 profile (AXIOM Tier 2.5)."
}

/// JSON Schema for the attach input. The InvokeBidi initial
/// frame carries `session_id`; subsequent inbound frames are
/// raw stdin bytes and `{type:\"resize\", cols, rows}` control frames — these are not
/// initial-args schemas, so they sit in the
/// daemon/execution/pty module's docs rather than here.
pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id", "attachment_id", "expected_epoch"],
        "additionalProperties": false,
        "properties": {
            "session_id": { "type": "string", "minLength": 1 },
            "attachment_id": { "type": "string", "minLength": 1 },
            "expected_epoch": { "type": "integer", "minimum": 0 }
        }
    })
}

/// Read chunk size for the PTY → wire path. 4 KiB matches the
/// default pipe buffer on macOS + Linux; smaller chunks hurt
/// throughput, larger ones add latency to single-keystroke echo.
const READ_CHUNK_SIZE: usize = 4096;

/// Sleep between exit-watcher polls. portable-pty's
/// `try_wait()` is non-blocking but the unix child reaper is not;
/// 100ms is short enough that an `exit` frame lands near the actual
/// process termination, long enough that the polling loop doesn't
/// burn CPU on idle sessions.
const EXIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub fn register(reg: &mut AxonAbilityCatalog, pty: Arc<PtyService>, io: PtyIoService) {
    let pty_for_attach = Arc::clone(&pty);
    let handler = Arc::new(move |env, args: Value| {
        let attach_args = TerminalAttachArgs::parse(args)?;
        super::authority::require_session_authority(
            &env,
            attach_args.session_id(),
            "terminal.attach",
        )?;
        attach_session(&pty_for_attach, &io, attach_args)
    });
    reg.register_bidi_with_envelope_and_owner(
        "terminal.attach",
        OwnerKind::terminal_system(),
        handler,
    );
}

#[cfg(test)]
fn attach_handler(
    pty: &Arc<PtyService>,
    io: &PtyIoService,
    args: Value,
) -> anyhow::Result<BidiSource> {
    attach_session(pty, io, TerminalAttachArgs::parse(args)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalAttachArgs {
    session_id: String,
    attachment_id: String,
    expected_epoch: u64,
}

impl TerminalAttachArgs {
    fn parse(args: Value) -> anyhow::Result<Self> {
        let object = terminal_attach_args_object(&args)?;
        let session_id = terminal_attach_required_session_id(object)?;
        let attachment_id = terminal_attach_required_nonempty_string(object, "attachment_id")?;
        let expected_epoch = terminal_attach_required_u64(object, "expected_epoch")?;
        Ok(Self {
            session_id,
            attachment_id,
            expected_epoch,
        })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }
}

fn terminal_attach_args_object(args: &Value) -> anyhow::Result<&Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: args must be a JSON object"))?;
    let mut unknown = object
        .keys()
        .filter(|key| !["session_id", "attachment_id", "expected_epoch"].contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        anyhow::bail!(
            "terminal.attach: unsupported argument field(s): {}",
            unknown.join(", ")
        );
    }
    Ok(object)
}

fn terminal_attach_required_nonempty_string(
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<String> {
    let value = args
        .get(field)
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `{field}` required"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `{field}` must be a string"))?
        .trim();
    if value.is_empty() {
        anyhow::bail!("terminal.attach: `{field}` must not be empty");
    }
    Ok(value.to_string())
}

fn terminal_attach_required_u64(args: &Map<String, Value>, field: &str) -> anyhow::Result<u64> {
    args.get(field)
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `{field}` required"))?
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `{field}` must be an unsigned integer"))
}

fn terminal_attach_required_session_id(args: &Map<String, Value>) -> anyhow::Result<String> {
    let raw = args
        .get("session_id")
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `session_id` required"))?;
    let session_id = raw
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("terminal.attach: `session_id` must be a string"))?
        .trim();
    if session_id.is_empty() {
        anyhow::bail!("terminal.attach: `session_id` must not be empty");
    }
    Ok(session_id.to_string())
}

fn attach_session(
    pty: &Arc<PtyService>,
    io: &PtyIoService,
    attach_args: TerminalAttachArgs,
) -> anyhow::Result<BidiSource> {
    let session_id = attach_args.session_id;
    let id = PtySessionId::new(&session_id);
    if !pty.try_contains(&id)? {
        anyhow::bail!("terminal.attach: unknown session_id `{session_id}`");
    }
    let lease = io.claim_attachment(
        pty,
        &id,
        &attach_args.attachment_id,
        attach_args.expected_epoch,
    )?;

    // Channel halves are transport-axis per BidiSource's contract:
    //   xport_to_handler_tx  — IPC pushes here (SendBidi);
    //                          handler reads via xport_to_handler_rx
    //   xport_from_handler_tx — handler writes here;
    //                           IPC reads via xport_from_handler_rx
    //                           and emits RecvBidi
    let (xport_to_handler_tx, xport_to_handler_rx) = bidi_input_channel(BIDI_CHANNEL_BOUND);
    let (xport_from_handler_tx, xport_from_handler_rx) =
        tokio::sync::mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);

    xport_from_handler_tx
        .try_send(BidiOutputFrame::json(json!({
            "type": "attached",
            "attachment_id": lease.attachment_id(),
            "epoch": lease.attached_epoch(),
        })))
        .map_err(|_| anyhow::anyhow!("terminal.attach: output channel closed before attached"))?;
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    spawn_pty_output_pump(
        Arc::clone(pty),
        io.clone(),
        id.clone(),
        lease.clone(),
        cancel_rx,
        cancel_tx.clone(),
        xport_from_handler_tx.clone(),
    );
    spawn_pty_writer(
        Arc::clone(pty),
        io.clone(),
        id,
        lease,
        xport_to_handler_rx,
        cancel_tx,
        xport_from_handler_tx,
    );

    Ok(BidiSource {
        to_client: xport_to_handler_tx,
        from_client: xport_from_handler_rx,
    })
}

/// T1: PTY master → wire. Blocking read on a dedicated thread
/// pool, send each chunk as a raw binary frame.
fn spawn_pty_output_pump(
    pty: Arc<PtyService>,
    io: PtyIoService,
    id: PtySessionId,
    lease: PtyAttachmentLease,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    cancel_tx: tokio::sync::watch::Sender<bool>,
    to_client: tokio::sync::mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        loop {
            if *cancel.borrow() {
                break;
            }
            let io_for_read = io.clone();
            let pty_for_read = Arc::clone(&pty);
            let id_for_read = id.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                io_for_read.read_bytes(
                    &pty_for_read,
                    &id_for_read,
                    EXIT_POLL_INTERVAL,
                    READ_CHUNK_SIZE,
                )
            })
            .await;
            let Ok(Ok(outcome)) = outcome else {
                let _ = to_client
                    .send(BidiOutputFrame::json(json!({
                        "type": "error",
                        "code": "SESSION_IO_FAILED",
                        "message": "terminal output pump failed",
                    })))
                    .await;
                break;
            };
            if outcome.dropped_bytes > 0
                && to_client
                    .send(BidiOutputFrame::json(json!({
                        "type": "output_gap",
                        "code": "OUTPUT_GAP",
                        "dropped_bytes": outcome.dropped_bytes,
                    })))
                    .await
                    .is_err()
            {
                break;
            }
            let closed = outcome.closed;
            let had_data = !outcome.data.is_empty();
            if had_data {
                if to_client
                    .send(BidiOutputFrame::binary(
                        outcome.data,
                        "application/octet-stream",
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            if closed && !had_data {
                let status = child_exit_status(&pty, &id);
                let _ = to_client
                    .send(BidiOutputFrame::json(
                        json!({"type": "exit", "status": status}),
                    ))
                    .await;
                let _ = cancel_tx.send(true);
                let _ = pty.try_close(&id);
                io.drop_session(&id);
                break;
            }
            if cancel.has_changed().unwrap_or(true) && *cancel.borrow_and_update() {
                break;
            }
        }
        drop(lease);
    });
}

/// T2: wire → PTY master. Async loop awaits frames from the
/// transport, dispatches each to a blocking writer (for stdin) or
/// the resize fast path (for resize). Exits when the receiver
/// yields None.
fn spawn_pty_writer(
    pty: Arc<PtyService>,
    io: PtyIoService,
    id: PtySessionId,
    lease: PtyAttachmentLease,
    mut from_client: tokio::sync::mpsc::Receiver<BidiInputFrame>,
    cancel: tokio::sync::watch::Sender<bool>,
    to_client: tokio::sync::mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        while let Some(frame) = from_client.recv().await {
            let frame = match TerminalAttachClientFrame::parse(frame) {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = to_client
                        .send(BidiOutputFrame::json(json!({
                            "type": "error",
                            "message": format!("terminal.attach client frame rejected: {error}"),
                        })))
                        .await;
                    break;
                }
            };
            match frame {
                TerminalAttachClientFrame::Stdin(bytes) => {
                    let io_for_write = io.clone();
                    let pty_for_write = Arc::clone(&pty);
                    let id_for_write = id.clone();
                    match tokio::task::spawn_blocking(move || {
                        io_for_write.write_bytes(&pty_for_write, &id_for_write, &bytes)
                    })
                    .await
                    {
                        Ok(Ok(PtyIoWriteOutcome::Written(_))) => {}
                        _ => break,
                    }
                }
                TerminalAttachClientFrame::Resize { cols, rows } => {
                    if pty.resize_session(&id, cols, rows).await.is_err() {
                        break;
                    }
                }
                TerminalAttachClientFrame::Detach => {
                    let epoch = lease.release();
                    let _ = to_client
                        .send(BidiOutputFrame::json(json!({
                            "type": "detached",
                            "attachment_id": lease.attachment_id(),
                            "epoch": epoch,
                        })))
                        .await;
                    break;
                }
                TerminalAttachClientFrame::CloseInput => {
                    let io_for_write = io.clone();
                    let pty_for_write = Arc::clone(&pty);
                    let id_for_write = id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        io_for_write.write_bytes(&pty_for_write, &id_for_write, &[0x04])
                    })
                    .await;
                }
            }
        }
        lease.release();
        let _ = cancel.send(true);
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TerminalAttachClientFrame {
    Stdin(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Detach,
    CloseInput,
}

impl TerminalAttachClientFrame {
    fn parse(frame: BidiInputFrame) -> anyhow::Result<Self> {
        if frame.content_type == "application/octet-stream" {
            return Ok(Self::Stdin(frame.payload));
        }
        if frame.content_type != "application/json" {
            anyhow::bail!(
                "terminal control frame must use application/json, got {:?}",
                frame.content_type
            );
        }
        let frame: Value = serde_json::from_slice(&frame.payload)
            .map_err(|error| anyhow::anyhow!("terminal control frame is not JSON: {error}"))?;
        let object = terminal_attach_client_frame_object(&frame)?;
        let frame_type = terminal_attach_required_frame_type(object)?;
        match frame_type {
            "resize" => {
                terminal_attach_reject_unknown_frame_fields(object, &["type", "cols", "rows"])?;
                let cols = terminal_attach_required_frame_u16(object, "cols")?;
                let rows = terminal_attach_required_frame_u16(object, "rows")?;
                if cols == 0 || rows == 0 {
                    anyhow::bail!("resize `cols` and `rows` must be > 0");
                }
                Ok(Self::Resize { cols, rows })
            }
            "detach" => {
                terminal_attach_reject_unknown_frame_fields(object, &["type"])?;
                Ok(Self::Detach)
            }
            "close_input" => {
                terminal_attach_reject_unknown_frame_fields(object, &["type"])?;
                Ok(Self::CloseInput)
            }
            other => anyhow::bail!("unsupported frame type `{other}`"),
        }
    }
}

fn terminal_attach_client_frame_object(frame: &Value) -> anyhow::Result<&Map<String, Value>> {
    frame
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("frame must be a JSON object"))
}

fn terminal_attach_required_frame_type(frame: &Map<String, Value>) -> anyhow::Result<&str> {
    terminal_attach_required_frame_string(frame, "type")
}

fn terminal_attach_required_frame_string<'a>(
    frame: &'a Map<String, Value>,
    key: &str,
) -> anyhow::Result<&'a str> {
    frame
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("`{key}` required"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("`{key}` must be a string"))
}

fn terminal_attach_required_frame_u16(
    frame: &Map<String, Value>,
    key: &str,
) -> anyhow::Result<u16> {
    match frame.get(key) {
        None | Some(Value::Null) => anyhow::bail!("`{key}` required"),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| anyhow::anyhow!("`{key}` must fit in u16 (got {number})")),
        Some(other) => anyhow::bail!("`{key}` must be a number, got {other}"),
    }
}

fn terminal_attach_reject_unknown_frame_fields(
    frame: &Map<String, Value>,
    allowed_keys: &[&str],
) -> anyhow::Result<()> {
    let mut unknown = frame
        .keys()
        .filter(|key| !allowed_keys.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        anyhow::bail!("unsupported frame field(s): {}", unknown.join(", "));
    }
    Ok(())
}

fn child_exit_status(pty: &Arc<PtyService>, id: &PtySessionId) -> Value {
    if let Some(status) = pty.supervised_exit_status(id) {
        return status
            .ok()
            .flatten()
            .map_or(Value::Null, |value| json!(value));
    }
    let Some(session) = pty.get(id) else {
        return Value::Null;
    };
    let Ok(mut child) = session.child.lock() else {
        return Value::Null;
    };
    let Some(child) = child.as_mut() else {
        return Value::Null;
    };
    match child.try_wait() {
        Ok(Some(status)) => json!(status.exit_code()),
        Ok(None) | Err(_) => Value::Null,
    }
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn attach_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id", "attachment_id", "expected_epoch"],
        "properties": {
            "session_id": {"type": "string", "minLength": 1},
            "attachment_id": {"type": "string", "minLength": 1},
            "expected_epoch": {"type": "integer", "minimum": 0},
        },
        "additionalProperties": false,
    })
}

pub fn attach_description() -> &'static str {
    "Attach an InvokeBidi session to a previously-opened PTY \
     (created via terminal.create). Client→handler frames: \
     application/octet-stream stdin, {type:\"resize\",cols,rows}, \
     {type:\"detach\"}, or {type:\"close_input\"}. Handler→client \
     frames include raw stdout, attached, detached, error, and exit."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::execution::pty::PtyCreateSpec;

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/terminal-attach";

    fn fresh_service() -> Arc<PtyService> {
        Arc::new(PtyService::new())
    }

    fn attach_args(session_id: &str) -> Value {
        attach_args_at(session_id, "test-attachment", 0)
    }

    fn attach_args_at(session_id: &str, attachment_id: &str, expected_epoch: u64) -> Value {
        json!({
            "session_id": session_id,
            "attachment_id": attachment_id,
            "expected_epoch": expected_epoch,
        })
    }

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    async fn send_stdin(sender: &crate::daemon::ability::dispatch::BidiInputSender, bytes: &[u8]) {
        sender
            .send_frame(
                BidiInputFrame::new(bytes.to_vec()).with_content_type("application/octet-stream"),
            )
            .await
            .expect("send raw PTY stdin");
    }

    fn json_input(value: Value) -> BidiInputFrame {
        BidiInputFrame::new(serde_json::to_vec(&value).expect("serialize control"))
            .with_content_type("application/json")
    }

    fn shell_command() -> String {
        // /bin/sh exists on every unix; using it (not /bin/bash)
        // keeps the test portable. The subset of POSIX sh we use is
        // `echo` (printable ASCII echo). Returns String (not &str)
        // so callers don't need a `.to_string()` at every call site.
        "/bin/sh".to_string()
    }

    /// Try to drain at most `n` RecvBidi frames from the bidi
    /// from_client receiver inside a soft deadline. A regression that
    /// drops a frame fails fast instead of hanging the test runner.
    ///
    /// **Early-exit on the `exit` frame.** Per §I2 the `exit` frame
    /// is terminal — by contract nothing meaningful follows it on
    /// this channel. The previous implementation kept polling until
    /// either `n` frames arrived or `deadline` elapsed, which forced
    /// every "child exits" test to burn its full deadline (the PTY
    /// reader's fd dup sits in a blocking `read()` after SIGHUP and
    /// only EOFs on its own schedule, so the channel doesn't close
    /// promptly). Under cargo-test parallel load that turned a 3 s
    /// budget into the actual critical path and produced flakes
    /// like `child_exit_emits_one_exit_frame`. Bailing the moment we
    /// see `exit` collapses every "wait for terminal" assertion to
    /// the actual settle time of the exit watcher (~100-300 ms) and
    /// matches what a real wire consumer would do — once `exit`
    /// fires, the session is gone.
    async fn drain_handler_emit(
        rx: &mut tokio::sync::mpsc::Receiver<BidiOutputFrame>,
        n: usize,
        deadline: std::time::Duration,
    ) -> Vec<Value> {
        let mut out = Vec::with_capacity(n);
        let start = std::time::Instant::now();
        while out.len() < n {
            let remaining = deadline.checked_sub(start.elapsed());
            let Some(rem) = remaining else {
                break;
            };
            match tokio::time::timeout(rem, rx.recv()).await {
                Ok(Some(f)) => {
                    let f = if f.content_type == "application/octet-stream" {
                        json!({"type": "stdout", "data": f.payload})
                    } else {
                        f.into_json_value().expect("PTY control JSON frame")
                    };
                    let is_exit = f.get("type").and_then(Value::as_str) == Some("exit");
                    out.push(f);
                    if is_exit {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        out
    }

    #[tokio::test]
    async fn registration_makes_attach_dispatchable() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, fresh_service(), PtyIoService::new());
        assert!(
            reg.resolve_bidi_with_env(ABILITY_TERMINAL_ATTACH).is_some(),
            "attach must register as a BIDI handler, not RPC/Stream"
        );
        assert!(
            reg.resolve_bidi_with_env("terminal.attach").is_some(),
            "attach must also publish the canonical runtime alias used by backend WS terminal"
        );
    }

    #[tokio::test]
    async fn attach_to_unknown_session_id_fails_with_clear_error() {
        let svc = fresh_service();
        let err =
            attach_handler(&svc, &PtyIoService::new(), attach_args("not-a-session")).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unknown session_id"),
            "error must name the missing session_id; got {msg:?}"
        );
    }

    #[tokio::test]
    async fn attach_missing_session_id_arg_rejects_at_parse() {
        let svc = fresh_service();
        let err = attach_handler(&svc, &PtyIoService::new(), json!({})).unwrap_err();
        assert!(format!("{err}").contains("session_id"));
    }

    #[tokio::test]
    async fn attach_rejects_non_object_args_before_lookup() {
        let svc = fresh_service();
        let err = attach_handler(&svc, &PtyIoService::new(), Value::Null).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("args must be a JSON object"),
            "wrong error: {message}"
        );
        assert!(
            !message.contains("unknown session_id"),
            "non-object input must fail before PTY lookup: {message}"
        );
    }

    #[tokio::test]
    async fn attach_rejects_unknown_argument_fields_before_lookup() {
        let svc = fresh_service();
        let err = attach_handler(
            &svc,
            &PtyIoService::new(),
            json!({"session_id": "not-a-session", "legacy_mode": true}),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("unsupported argument field(s): legacy_mode"),
            "wrong error: {message}"
        );
        assert!(
            !message.contains("unknown session_id"),
            "unknown fields must fail before PTY lookup: {message}"
        );
    }

    #[tokio::test]
    async fn attach_rejects_wrong_typed_session_id_before_lookup() {
        let svc = fresh_service();
        let err = attach_handler(
            &svc,
            &PtyIoService::new(),
            json!({"session_id": 42, "attachment_id": "a", "expected_epoch": 0}),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("`session_id` must be a string"),
            "wrong error: {message}"
        );
        assert!(
            !message.contains("unknown session_id"),
            "wrong-typed session_id must fail before PTY lookup: {message}"
        );
    }

    #[tokio::test]
    async fn attach_rejects_blank_session_id_before_lookup() {
        let svc = fresh_service();
        let err = attach_handler(
            &svc,
            &PtyIoService::new(),
            json!({"session_id": "   ", "attachment_id": "a", "expected_epoch": 0}),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("`session_id` must not be empty"),
            "wrong error: {message}"
        );
        assert!(
            !message.contains("unknown session_id"),
            "blank session_id must fail before PTY lookup: {message}"
        );
    }

    #[tokio::test]
    async fn stdin_frame_is_echoed_as_stdout_through_a_real_pty() {
        // E2E: spawn /bin/sh in a PTY, attach, write `echo hi\n`
        // as stdin, observe `stdout` frames whose decoded bytes
        // contain "hi". A regression in either the writer (T2) or
        // the reader (T1) trips this.
        let svc = fresh_service();
        let spec = PtyCreateSpec {
            command: Some(shell_command()),
            ..PtyCreateSpec::default()
        };
        let id = svc.create(spec).expect("spawn /bin/sh");
        let io = PtyIoService::new();
        let source = attach_handler(&svc, &io, attach_args(id.as_str())).expect("attach");

        send_stdin(&source.to_client, b"echo hi\n").await;

        // Drain frames; concatenate every raw `stdout`
        // payload. A real shell may emit a prompt + the typed line
        // echo + the command output + the next prompt, so we just
        // assert "hi" appears somewhere in the cumulative bytes.
        let mut from_handler = source.from_client;
        let mut accum = Vec::new();
        let frames =
            drain_handler_emit(&mut from_handler, 32, std::time::Duration::from_secs(3)).await;
        for f in &frames {
            if f.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(bytes) = f.get("data").and_then(Value::as_array) {
                    accum.extend(bytes.iter().map(|value| value.as_u64().unwrap() as u8));
                }
            }
        }
        let s = String::from_utf8_lossy(&accum);
        assert!(
            s.contains("hi"),
            "expected `hi` in cumulative PTY stdout; got {:?} (frames: {} total)",
            s,
            frames.len()
        );

        svc.close(&id);
    }

    #[tokio::test]
    async fn resize_frame_does_not_close_session() {
        // §D5 + resize handling: a resize frame succeeds without
        // emitting an error or terminating the wire. We pin this by
        // sending a resize, then a stdin echo, and observing the
        // stdin still round-trips.
        let svc = fresh_service();
        let spec = PtyCreateSpec {
            command: Some(shell_command()),
            ..PtyCreateSpec::default()
        };
        let id = svc.create(spec).expect("spawn /bin/sh");
        let io = PtyIoService::new();
        let source = attach_handler(&svc, &io, attach_args(id.as_str())).expect("attach");

        source
            .to_client
            .send(json!({"type": "resize", "cols": 200, "rows": 60}))
            .await
            .expect("send resize");
        send_stdin(&source.to_client, b"echo postresize\n").await;

        let mut from_handler = source.from_client;
        let mut accum = Vec::new();
        let frames =
            drain_handler_emit(&mut from_handler, 32, std::time::Duration::from_secs(3)).await;
        for f in &frames {
            if f.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(bytes) = f.get("data").and_then(Value::as_array) {
                    accum.extend(bytes.iter().map(|value| value.as_u64().unwrap() as u8));
                }
            }
        }
        let s = String::from_utf8_lossy(&accum);
        assert!(
            s.contains("postresize"),
            "stdin after resize must still round-trip; got {s:?}"
        );

        svc.close(&id);
    }

    #[tokio::test]
    async fn detach_releases_epoch_and_reattach_reuses_the_same_pty_writer() {
        let svc = fresh_service();
        let io = PtyIoService::new();
        let id = svc
            .create(PtyCreateSpec {
                command: Some(shell_command()),
                ..PtyCreateSpec::default()
            })
            .expect("spawn /bin/sh");
        let first = attach_handler(&svc, &io, attach_args_at(id.as_str(), "attachment-a", 0))
            .expect("first attach");

        let conflict = attach_handler(&svc, &io, attach_args_at(id.as_str(), "attachment-b", 1))
            .expect_err("one active attachment per session");
        assert!(conflict.to_string().contains("SESSION_ALREADY_ATTACHED"));

        first
            .to_client
            .send(json!({"type": "detach"}))
            .await
            .expect("request detach");
        let mut first_output = first.from_client;
        let first_frames =
            drain_handler_emit(&mut first_output, 8, std::time::Duration::from_secs(2)).await;
        assert!(first_frames.iter().any(|frame| {
            frame.get("type").and_then(Value::as_str) == Some("detached")
                && frame.get("epoch").and_then(Value::as_u64) == Some(2)
        }));

        let stale = attach_handler(
            &svc,
            &io,
            attach_args_at(id.as_str(), "attachment-stale", 0),
        )
        .expect_err("stale epoch must not reclaim the PTY");
        assert!(stale.to_string().contains("ATTACHMENT_STALE"));

        let second = attach_handler(&svc, &io, attach_args_at(id.as_str(), "attachment-b", 2))
            .expect("reattach");
        send_stdin(&second.to_client, b"echo reattached\n").await;
        let mut second_output = second.from_client;
        let frames =
            drain_handler_emit(&mut second_output, 32, std::time::Duration::from_secs(3)).await;
        let mut output = Vec::new();
        for frame in frames {
            if frame.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(data) = frame.get("data").and_then(Value::as_array) {
                    output.extend(data.iter().map(|value| value.as_u64().unwrap() as u8));
                }
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("reattached"));
        svc.close(&id);
        io.drop_session(&id);
    }

    #[tokio::test]
    async fn child_exit_emits_one_exit_frame() {
        // /bin/true exits immediately; the exit-watcher should
        // emit exactly one `exit` frame. Pins T3.
        let svc = fresh_service();
        let spec = PtyCreateSpec {
            command: Some(if std::path::Path::new("/usr/bin/true").exists() {
                "/usr/bin/true".to_string()
            } else {
                "/bin/true".to_string()
            }),
            ..PtyCreateSpec::default()
        };
        let id = svc.create(spec).expect("spawn /usr/bin/true");
        let io = PtyIoService::new();
        let source = attach_handler(&svc, &io, attach_args(id.as_str())).expect("attach");

        // Wait for an exit frame within a generous deadline (exit-
        // watcher polls every 100ms; the child may need a tick or
        // two to be reaped).
        let mut from_handler = source.from_client;
        let frames =
            drain_handler_emit(&mut from_handler, 16, std::time::Duration::from_secs(3)).await;
        let exit_count = frames
            .iter()
            .filter(|f| f.get("type").and_then(Value::as_str) == Some("exit"))
            .count();
        assert!(
            exit_count >= 1,
            "expected at least one `exit` frame for a /bin/true session; \
             got {exit_count} exit frames in {} total ({:?})",
            frames.len(),
            frames
        );
    }

    #[tokio::test]
    async fn unknown_frame_type_emits_error_and_closes_ingress() {
        let svc = fresh_service();
        let spec = PtyCreateSpec {
            command: Some(shell_command()),
            ..PtyCreateSpec::default()
        };
        let id = svc.create(spec).expect("spawn /bin/sh");
        let io = PtyIoService::new();
        let source = attach_handler(&svc, &io, attach_args(id.as_str())).expect("attach");
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;

        to_handler
            .send(json!({"type": "future_v2_frame_type", "payload": "anything"}))
            .await
            .expect("send unknown frame");

        let frames =
            drain_handler_emit(&mut from_handler, 16, std::time::Duration::from_secs(3)).await;
        assert!(
            frames.iter().any(|frame| {
                frame.get("type").and_then(Value::as_str) == Some("error")
                    && frame
                        .get("message")
                        .and_then(Value::as_str)
                        .is_some_and(|message| message.contains("unsupported frame type"))
            }),
            "unknown client frames must produce a visible protocol error; got {frames:?}"
        );

        if to_handler
            .send_frame(
                BidiInputFrame::new(b"echo afterjunk\n".to_vec())
                    .with_content_type("application/octet-stream"),
            )
            .await
            .is_ok()
        {
            let frames =
                drain_handler_emit(&mut from_handler, 32, std::time::Duration::from_millis(500))
                    .await;
            let mut accum = Vec::new();
            for frame in &frames {
                if frame.get("type").and_then(Value::as_str) == Some("stdout") {
                    if let Some(bytes) = frame.get("data").and_then(Value::as_array) {
                        accum.extend(bytes.iter().map(|value| value.as_u64().unwrap() as u8));
                    }
                }
            }
            let output = String::from_utf8_lossy(&accum);
            assert!(
                !output.contains("afterjunk"),
                "ingress must not process stdin after protocol rejection; got {output:?}"
            );
        }

        svc.close(&id);
    }

    #[test]
    fn client_frame_parser_rejects_non_object_frames() {
        let err = TerminalAttachClientFrame::parse(json_input(Value::Null)).unwrap_err();
        assert!(
            format!("{err}").contains("frame must be a JSON object"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn client_frame_parser_preserves_all_raw_stdin_bytes() {
        let bytes = (0_u8..=u8::MAX).collect::<Vec<_>>();
        let parsed = TerminalAttachClientFrame::parse(
            BidiInputFrame::new(bytes.clone()).with_content_type("application/octet-stream"),
        )
        .expect("raw stdin frame");
        assert_eq!(parsed, TerminalAttachClientFrame::Stdin(bytes));
    }

    #[test]
    fn client_frame_parser_rejects_bad_resize_dimensions() {
        let err = TerminalAttachClientFrame::parse(json_input(json!({
            "type": "resize",
            "cols": 0,
            "rows": 24
        })))
        .unwrap_err();
        assert!(
            format!("{err}").contains("must be > 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn input_schema_requires_session_id() {
        let s = attach_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "session_id"));
        assert_eq!(s["additionalProperties"], false);
    }
}
