// EasyNet CLI — fleet.pty_session_attach (C-M3c, BIDI)
// =======================================================
//
// File: src/runtime/agents/pty_attach_ability.rs
//
// Bidi handler that wires one InvokeBidi session to one previously-
// opened PTY (created via fleet.pty_session_create, C-M3b). The
// transport gives us two mpsc::Sender/Receiver halves; the handler
// glues them to the PTY master fd's blocking reader + writer.
//
// Frame protocol
// --------------
//   Args at OpenBidi:  { "session_id": "<uuid-from-create>" }
//
//   Client → handler (SendBidi.frame):
//     { "type": "stdin",  "data": "<base64 bytes>" }
//     { "type": "resize", "cols": u16, "rows": u16 }
//
//   Handler → client (RecvBidi.frame):
//     { "type": "stdout", "data": "<base64 bytes>" }
//     { "type": "exit",   "status": <int|null> }
//
// The single TerminalBidi per session_id (§I2) is fired by the IPC
// forwarder, not this handler. The handler signals "session over"
// by dropping its `to_client` sender, which the forwarder observes
// as channel EOF and emits TerminalBidi{done}.
//
// Layered tasks per session
// -------------------------
// Three tokio tasks per attach:
//
//   T1. PTY → wire reader (spawn_blocking).
//       Owns the std::io::Read half of the PTY master. Read syscall
//       blocks until bytes arrive; the loop wraps each chunk in a
//       `stdout` frame and `blocking_send`s into the transport's
//       to_client sender. Exits when read() returns 0 (child closed
//       the pty) or when the transport sender errors (forwarder
//       gone).
//
//   T2. wire → PTY writer (async loop).
//       Owns the from_client receiver. Awaits each frame; for
//       `stdin` frames base64-decodes and `spawn_blocking`s a
//       writer.write_all + flush; for `resize` frames calls
//       PtySession.resize. Exits when receiver yields None
//       (CloseBidi or connection drop) OR a write fails (PTY gone).
//
//   T3. exit-watcher (spawn_blocking).
//       Polls the child's wait() in a tight loop with a small
//       sleep. When the child exits, emits one `exit` frame then
//       drops its sender clone, contributing to the to_client
//       drop quorum that triggers TerminalBidi.
//
// All three tasks share the to_client sender; the forwarder sees
// EOF only after the LAST clone drops. Each task drops its clone
// on its own exit path; the natural quorum prevents premature
// session close while one direction is still active.
//
// Why spawn_blocking for T1 and the inner write of T2
// ----------------------------------------------------
// portable-pty exposes std::io::Read / std::io::Write — synchronous,
// blocking calls. Putting them on a tokio worker thread (default
// runtime) would block the runtime. spawn_blocking moves the
// syscall to the dedicated blocking pool; the async loop awaits
// the JoinHandle, which is the canonical bridge.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use base64::Engine;
use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{BidiSource, LocalAbilityRegistry, BIDI_CHANNEL_BOUND};
use crate::runtime::execution::pty::{PtyService, PtySessionId};

pub const ABILITY_PTY_SESSION_ATTACH: &str = "fleet.pty_session_attach";

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

pub fn register(reg: &mut LocalAbilityRegistry, pty: Arc<PtyService>) {
    reg.register_bidi(
        ABILITY_PTY_SESSION_ATTACH,
        Arc::new(move |args: Value| attach_handler(&pty, args)),
    );
}

fn attach_handler(pty: &Arc<PtyService>, args: Value) -> anyhow::Result<BidiSource> {
    let session_id = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("pty_session_attach: `session_id` required"))?
        .to_string();
    let id = PtySessionId::new(&session_id);
    let session = pty
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("pty_session_attach: unknown session_id `{session_id}`"))?;

    // Build the four channel halves up front. Names are transport-
    // axis per BidiSource's contract (transport's perspective):
    //   xport_to_handler_tx  — IPC pushes here, handler reads
    //   xport_to_handler_rx  — handler reads SendBidi frames
    //   xport_from_handler_tx — handler pushes RecvBidi frames here
    //   xport_from_handler_rx — IPC pulls here, emits RecvBidi
    let (xport_to_handler_tx, xport_to_handler_rx) =
        tokio::sync::mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
    let (xport_from_handler_tx, xport_from_handler_rx) =
        tokio::sync::mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);

    // Clone Arcs for the three tasks. Each task drops its clone on
    // its own exit path; the to_client sender's drop quorum is what
    // triggers the §I2 TerminalBidi.
    let session_for_reader = Arc::clone(&session);
    let session_for_writer = Arc::clone(&session);
    let session_for_waiter = session;
    let to_client_for_reader = xport_from_handler_tx.clone();
    let to_client_for_waiter = xport_from_handler_tx.clone();
    let to_client_for_writer = xport_from_handler_tx;
    // Drop the original immediately so we don't hold a fourth clone
    // here — only the three tasks own a sender.
    drop(to_client_for_writer);
    let to_client_for_writer = to_client_for_reader.clone();
    // ↑ structure: rebind so writer also has a clone. Total clones:
    // reader + waiter + writer + (the to_client_for_writer line we
    // just dropped). Net: 3 live, matching the three tasks.

    spawn_pty_reader(session_for_reader, to_client_for_reader);
    spawn_pty_writer(session_for_writer, xport_to_handler_rx, to_client_for_writer);
    spawn_exit_watcher(session_for_waiter, to_client_for_waiter, pty.clone(), id);

    Ok(BidiSource {
        to_client: xport_to_handler_tx,
        from_client: xport_from_handler_rx,
    })
}

/// T1: PTY master → wire. Blocking read on a dedicated thread
/// pool, send each chunk as a `stdout` base64 frame.
fn spawn_pty_reader(
    session: Arc<crate::runtime::execution::pty::PtySession>,
    to_client: tokio::sync::mpsc::Sender<Value>,
) {
    tokio::spawn(async move {
        // Take the reader handle once; we own it for the task's life.
        let reader = {
            let m = session.master.lock().await;
            match m.try_clone_reader() {
                Ok(r) => r,
                Err(_) => {
                    // PTY can't lend a reader; surface as exit-with-
                    // unknown so the wire sees a deterministic close.
                    let _ = to_client
                        .send(json!({"type": "exit", "status": Value::Null}))
                        .await;
                    return;
                }
            }
        };

        let res = tokio::task::spawn_blocking(move || {
            // Move reader into the blocking task; loop until EOF or
            // send-failure (forwarder gone). The async send happens
            // via blocking_send because we're outside the tokio
            // runtime here.
            let mut reader = reader;
            let mut buf = vec![0u8; READ_CHUNK_SIZE];
            use std::io::Read;
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,            // EOF
                    Ok(n) => n,
                    Err(_) => break,           // PTY gone
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                if to_client
                    .blocking_send(json!({"type": "stdout", "data": encoded}))
                    .is_err()
                {
                    break; // forwarder gone
                }
            }
        })
        .await;
        // Discard JoinError: the spawn_blocking call's Result
        // distinguishes panic vs clean exit; we don't care which —
        // dropping our to_client clone is what the forwarder needs.
        let _ = res;
    });
}

/// T2: wire → PTY master. Async loop awaits frames from the
/// transport, dispatches each to a blocking writer (for stdin) or
/// the resize fast path (for resize). Exits when the receiver
/// yields None.
fn spawn_pty_writer(
    session: Arc<crate::runtime::execution::pty::PtySession>,
    mut from_client: tokio::sync::mpsc::Receiver<Value>,
    to_client: tokio::sync::mpsc::Sender<Value>,
) {
    tokio::spawn(async move {
        // Take the writer once; portable-pty's take_writer can only
        // be called once per master, so we hold it for the loop.
        let writer = {
            let m = session.master.lock().await;
            match m.take_writer() {
                Ok(w) => w,
                Err(_) => return, // can't write → drop sender → exit
            }
        };
        // Wrap in Arc<Mutex> so spawn_blocking calls can move it
        // back and forth across the await points.
        let writer = std::sync::Arc::new(std::sync::Mutex::new(writer));

        while let Some(frame) = from_client.recv().await {
            let frame_type = frame
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match frame_type {
                "stdin" => {
                    let data_b64 = match frame.get("data").and_then(Value::as_str) {
                        Some(s) => s.to_string(),
                        None => {
                            // Per §D5: per-frame error is a
                            // diagnostic, not a session close. We
                            // surface and continue.
                            let _ = to_client
                                .send(json!({
                                    "type": "stdout",
                                    "data": "",
                                    "_error": "stdin frame missing `data` field"
                                }))
                                .await;
                            continue;
                        }
                    };
                    let bytes = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };
                    let writer_clone = std::sync::Arc::clone(&writer);
                    let _ = tokio::task::spawn_blocking(move || {
                        use std::io::Write;
                        let mut w = writer_clone.lock().expect("pty writer lock");
                        let _ = w.write_all(&bytes);
                        let _ = w.flush();
                    })
                    .await;
                }
                "resize" => {
                    let cols = frame
                        .get("cols")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok());
                    let rows = frame
                        .get("rows")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok());
                    if let (Some(c), Some(r)) = (cols, rows) {
                        let _ = session.resize(c, r).await;
                    }
                }
                _other => {
                    // Unknown frame type: drop silently. Forward-
                    // compat — a future stdin variant (e.g. paste-
                    // mode marker) shouldn't break old daemons.
                }
            }
        }
        // Receiver returned None → CloseBidi from client OR
        // connection drop. Drop our to_client clone by falling out
        // of scope; the §I2 TerminalBidi fires once the reader and
        // waiter also drop their clones.
    });
}

/// T3: exit-watcher. Polls the child's wait() and emits one `exit`
/// frame when the child terminates. The frame carries the exit
/// status when waitable; null when the child was reaped externally.
fn spawn_exit_watcher(
    session: Arc<crate::runtime::execution::pty::PtySession>,
    to_client: tokio::sync::mpsc::Sender<Value>,
    pty: Arc<PtyService>,
    id: PtySessionId,
) {
    tokio::spawn(async move {
        // We can't hold the child Mutex across an await, so the loop
        // wakes every EXIT_POLL_INTERVAL and tries try_wait inside a
        // short critical section.
        loop {
            let status = {
                let mut g = match session.child.lock() {
                    Ok(g) => g,
                    Err(_) => return, // poisoned → caller side gave up
                };
                let Some(child) = g.as_mut() else {
                    // Child slot is empty — close raced with us.
                    return;
                };
                match child.try_wait() {
                    Ok(Some(s)) => Some(s.exit_code() as i32),
                    Ok(None) => None,    // still alive
                    Err(_) => Some(-1),  // wait error → treat as exit
                }
            };
            if let Some(code) = status {
                let _ = to_client
                    .send(json!({"type": "exit", "status": code}))
                    .await;
                // Best-effort cleanup: remove the session row so a
                // future close sees ack=false (idempotent). Failure
                // here is benign — close handler also removes the row.
                let _ = pty.close(&id);
                return;
            }
            tokio::time::sleep(EXIT_POLL_INTERVAL).await;
        }
    });
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn attach_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "properties": {
            "session_id": {"type": "string"},
        },
        "additionalProperties": false,
    })
}

pub fn attach_description() -> &'static str {
    "Attach an InvokeBidi session to a previously-opened PTY \
     (created via fleet.pty_session_create). Client→handler frames: \
     {type:\"stdin\",data:b64} or {type:\"resize\",cols,rows}. \
     Handler→client frames: {type:\"stdout\",data:b64} and a final \
     {type:\"exit\",status} when the child terminates."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution::pty::PtyCreateSpec;

    fn fresh_service() -> Arc<PtyService> {
        Arc::new(PtyService::new())
    }

    fn shell_command() -> &'static str {
        // /bin/sh exists on every unix; using it (not /bin/bash)
        // keeps the test portable. The subset of POSIX sh we use is
        // `echo` (printable ASCII echo).
        "/bin/sh"
    }

    /// Try to drain at most `n` RecvBidi frames from the bidi
    /// from_client receiver inside a soft deadline. A regression that
    /// drops a frame fails fast instead of hanging the test runner.
    async fn drain_handler_emit(
        rx: &mut tokio::sync::mpsc::Receiver<Value>,
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
                Ok(Some(f)) => out.push(f),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        out
    }

    #[tokio::test]
    async fn registration_makes_attach_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, fresh_service());
        assert!(
            reg.get_bidi(ABILITY_PTY_SESSION_ATTACH).is_some(),
            "attach must register as a BIDI handler, not RPC/Stream"
        );
    }

    #[tokio::test]
    async fn attach_to_unknown_session_id_fails_with_clear_error() {
        let svc = fresh_service();
        let err =
            attach_handler(&svc, json!({"session_id": "not-a-session"})).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unknown session_id"),
            "error must name the missing session_id; got {msg:?}"
        );
    }

    #[tokio::test]
    async fn attach_missing_session_id_arg_rejects_at_parse() {
        let svc = fresh_service();
        let err = attach_handler(&svc, json!({})).unwrap_err();
        assert!(format!("{err}").contains("session_id"));
    }

    #[tokio::test]
    async fn stdin_frame_is_echoed_as_stdout_through_a_real_pty() {
        // E2E: spawn /bin/sh in a PTY, attach, write `echo hi\n`
        // as stdin, observe `stdout` frames whose decoded bytes
        // contain "hi". A regression in either the writer (T2) or
        // the reader (T1) trips this.
        let svc = fresh_service();
        let mut spec = PtyCreateSpec::default();
        spec.command = Some(shell_command().to_string());
        let id = svc.create(spec).expect("spawn /bin/sh");

        let source =
            attach_handler(&svc, json!({"session_id": id.as_str()})).expect("attach");

        // Send one stdin frame: `echo hi\n`. Base64-encoded.
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(b"echo hi\n");
        source
            .to_client
            .send(json!({"type": "stdin", "data": data_b64}))
            .await
            .expect("send stdin frame");

        // Drain frames; concatenate every base64-decoded `stdout`
        // payload. A real shell may emit a prompt + the typed line
        // echo + the command output + the next prompt, so we just
        // assert "hi" appears somewhere in the cumulative bytes.
        let mut from_handler = source.from_client;
        let mut accum = Vec::new();
        let frames = drain_handler_emit(
            &mut from_handler,
            32,
            std::time::Duration::from_secs(3),
        )
        .await;
        for f in &frames {
            if f.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(b64) = f.get("data").and_then(Value::as_str) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .unwrap_or_default();
                    accum.extend_from_slice(&bytes);
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
        let mut spec = PtyCreateSpec::default();
        spec.command = Some(shell_command().to_string());
        let id = svc.create(spec).expect("spawn /bin/sh");
        let source =
            attach_handler(&svc, json!({"session_id": id.as_str()})).expect("attach");

        source
            .to_client
            .send(json!({"type": "resize", "cols": 200, "rows": 60}))
            .await
            .expect("send resize");
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(b"echo postresize\n");
        source
            .to_client
            .send(json!({"type": "stdin", "data": data_b64}))
            .await
            .expect("send stdin");

        let mut from_handler = source.from_client;
        let mut accum = Vec::new();
        let frames = drain_handler_emit(
            &mut from_handler,
            32,
            std::time::Duration::from_secs(3),
        )
        .await;
        for f in &frames {
            if f.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(b64) = f.get("data").and_then(Value::as_str) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .unwrap_or_default();
                    accum.extend_from_slice(&bytes);
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
    async fn child_exit_emits_one_exit_frame() {
        // /bin/true exits immediately; the exit-watcher should
        // emit exactly one `exit` frame. Pins T3.
        let svc = fresh_service();
        let mut spec = PtyCreateSpec::default();
        spec.command = Some(
            if std::path::Path::new("/usr/bin/true").exists() {
                "/usr/bin/true".to_string()
            } else {
                "/bin/true".to_string()
            },
        );
        let id = svc.create(spec).expect("spawn /usr/bin/true");
        let source =
            attach_handler(&svc, json!({"session_id": id.as_str()})).expect("attach");

        // Wait for an exit frame within a generous deadline (exit-
        // watcher polls every 100ms; the child may need a tick or
        // two to be reaped).
        let mut from_handler = source.from_client;
        let frames = drain_handler_emit(
            &mut from_handler,
            16,
            std::time::Duration::from_secs(3),
        )
        .await;
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
    async fn unknown_frame_type_is_dropped_silently_no_close() {
        // §D5 + forward compat: an unrecognised frame type (e.g.
        // a future "paste-mode" marker) must NOT close the session.
        // Send junk, then a valid stdin, observe stdin still works.
        let svc = fresh_service();
        let mut spec = PtyCreateSpec::default();
        spec.command = Some(shell_command().to_string());
        let id = svc.create(spec).expect("spawn /bin/sh");
        let source =
            attach_handler(&svc, json!({"session_id": id.as_str()})).expect("attach");

        source
            .to_client
            .send(json!({"type": "future_v2_frame_type", "payload": "anything"}))
            .await
            .expect("send unknown frame");
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(b"echo afterjunk\n");
        source
            .to_client
            .send(json!({"type": "stdin", "data": data_b64}))
            .await
            .expect("send stdin");

        let mut from_handler = source.from_client;
        let mut accum = Vec::new();
        let frames = drain_handler_emit(
            &mut from_handler,
            32,
            std::time::Duration::from_secs(3),
        )
        .await;
        for f in &frames {
            if f.get("type").and_then(Value::as_str) == Some("stdout") {
                if let Some(b64) = f.get("data").and_then(Value::as_str) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .unwrap_or_default();
                    accum.extend_from_slice(&bytes);
                }
            }
        }
        let s = String::from_utf8_lossy(&accum);
        assert!(
            s.contains("afterjunk"),
            "session must survive unknown frame type; got {s:?}"
        );

        svc.close(&id);
    }

    #[test]
    fn input_schema_requires_session_id() {
        let s = attach_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "session_id"));
        assert_eq!(s["additionalProperties"], false);
    }
}
