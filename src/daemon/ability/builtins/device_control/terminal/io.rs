// EasyNet CLI — terminal.{input,read,resize} ability handlers
// =====================================================================
//
// File: src/daemon/ability/builtins/device_control/terminal/io.rs
//
// Unary compatibility surface over the same supervisor-owned PTY state used
// by terminal.attach. It pairs with terminal lifecycle and InvokeBidi attach:
//
//   * `terminal.input`  (RPC) — push base64 stdin bytes
//   * `terminal.read`   (RPC) — drain stdout up to timeout
//   * `terminal.resize` (RPC) — set cols × rows
//
// Production delegates write/read/resize and attachment leases to the
// per-user session supervisor. Unit tests use the in-process implementation
// below so state-machine behavior is deterministic.
//
// Reader-buffer design
// --------------------
// portable-pty's reader is blocking; we can't poll it from a
// `tokio::time::timeout` directly. So per session we own:
//
//   * a dedicated std::thread that loops `Read::read` on the
//     PTY master fd and pushes chunks into a `VecDeque<u8>`
//     guarded by a Mutex + Condvar.
//   * `terminal.read` waits on the Condvar with a timeout;
//     returns the drained bytes (or empty if the timeout hit).
//
// The test reader is created lazily. Production has one supervisor reader and
// one bounded ring buffer per session, shared across detach/reattach and daemon
// restart.
//
// Coexistence rule
// ----------------
// Attachment claims are mutually exclusive and epoch-checked. Both unary reads
// and Bidi attaches consume the single supervisor buffer; two OS-level PTY
// readers are never created in production.
//
// Why a writer cache (instead of `take_writer()` per call)
// ---------------------------------------------------------
// portable-pty's `take_writer()` is one-shot — calling it twice
// returns "cannot take writer more than once". We cache the
// writer in the I/O state row so subsequent input calls hit it
// directly. The cache is created lazily on the first input call
// for the same reasons as the reader thread: bidi-attached
// sessions must not lose their writer to a stray unary call.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::execution::pty::{PtyService, PtySessionId};

pub const ABILITY_TERMINAL_INPUT: &str =
    crate::daemon::ability::names::device_control::TERMINAL_INPUT;
pub const ABILITY_TERMINAL_READ: &str =
    crate::daemon::ability::names::device_control::TERMINAL_READ;
pub const ABILITY_TERMINAL_RESIZE: &str =
    crate::daemon::ability::names::device_control::TERMINAL_RESIZE;

/// Default per-call read budget when the caller doesn't supply one.
/// 5s matches backend PTYDriver's poll cadence (~200 ms idle reads
/// stacked up on a single connection produce roughly this, and a
/// 5s ceiling means a paused PTY surfaces "no output" in a
/// human-perceptible window rather than wedging the connection).
const DEFAULT_READ_TIMEOUT_SECS: f64 = 5.0;
/// Hard ceiling on `timeout` to keep a buggy caller from pinning
/// a worker thread for hours. 60s is plenty for any legitimate
/// poll; longer sessions repeat the call.
const MAX_READ_TIMEOUT_SECS: f64 = 60.0;
/// Maximum bytes a single `terminal.read` call returns. Prevents
/// a runaway producer (a `yes` loop, e.g.) from generating multi-MB
/// payloads inside one Invoke envelope. The buffer keeps growing
/// in memory; the next read drains the rest.
const MAX_READ_CHUNK_BYTES: usize = 256 * 1024;
/// Hard cap on the per-session output buffer. A persistent producer
/// with no consumer would otherwise grow the buffer without bound
/// and OOM the daemon. When the buffer is full, the reader thread
/// drops the oldest bytes — same policy `tail -f` survivors use.
const OUTPUT_BUFFER_CAP_BYTES: usize = 4 * 1024 * 1024;
/// Per-thread read chunk. Mirrors terminal_attach_ability::READ_CHUNK_SIZE
/// so behaviour is consistent across the two surfaces.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Per-session I/O state. One row per session that has had an
/// input or read call (lazy init). Removed by `terminal.close`
/// — see `register`'s wrapper around the lifecycle close handler.
struct SessionIo {
    /// Output buffer + Condvar. The reader thread pushes here;
    /// `terminal.read` waits on `cv` with a timeout. The buffer
    /// also stores a "PTY ended" flag so the read handler can
    /// surface session_dead to the backend's typed-error path.
    output: Arc<(Mutex<OutputState>, Condvar)>,
    /// Cached writer (lazy). `take_writer()` is one-shot per master,
    /// so we hold it forever once obtained.
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    /// Reader-thread join handle. Kept so we can cleanly drop on
    /// session close. The thread itself terminates when the PTY
    /// returns EOF (child exited) — the `dropped` flag is the
    /// secondary stop signal for the rare case that close fires
    /// while the child still has output queued.
    _reader: thread::JoinHandle<()>,
    /// Set by close to tell the reader thread to stop after its
    /// next loop iteration. The thread also exits naturally on
    /// EOF; this flag is the close-side cooperative signal.
    dropped: Arc<std::sync::atomic::AtomicBool>,
    attachment: Mutex<AttachmentState>,
}

#[derive(Debug, Default)]
struct AttachmentState {
    epoch: u64,
    active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PtyIoReadOutcome {
    pub data: Vec<u8>,
    pub closed: bool,
    pub dropped_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PtyIoWriteOutcome {
    Written(usize),
    Closed,
}

struct PtyAttachmentLeaseInner {
    backend: PtyAttachmentLeaseBackend,
    attachment_id: String,
    attached_epoch: u64,
    released: std::sync::atomic::AtomicBool,
}

enum PtyAttachmentLeaseBackend {
    Local(Arc<SessionIo>),
    Supervised {
        pty: Arc<PtyService>,
        session_id: PtySessionId,
    },
}

impl PtyAttachmentLeaseInner {
    fn release(&self) -> u64 {
        if self
            .released
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return match &self.backend {
                PtyAttachmentLeaseBackend::Local(row) => {
                    row.attachment
                        .lock()
                        .expect("pty attachment state lock")
                        .epoch
                }
                PtyAttachmentLeaseBackend::Supervised { pty, session_id } => pty
                    .supervised_attachment(session_id)
                    .and_then(Result::ok)
                    .map(|(epoch, _)| epoch)
                    .unwrap_or(self.attached_epoch),
            };
        }
        match &self.backend {
            PtyAttachmentLeaseBackend::Local(row) => {
                let mut state = row.attachment.lock().expect("pty attachment state lock");
                if state.epoch == self.attached_epoch
                    && state.active.as_deref() == Some(self.attachment_id.as_str())
                {
                    state.active = None;
                    state.epoch = state.epoch.saturating_add(1);
                }
                state.epoch
            }
            PtyAttachmentLeaseBackend::Supervised { pty, session_id } => pty
                .supervised_release(session_id, &self.attachment_id, self.attached_epoch)
                .and_then(Result::ok)
                .unwrap_or(self.attached_epoch),
        }
    }
}

impl Drop for PtyAttachmentLeaseInner {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone)]
pub(crate) struct PtyAttachmentLease(Arc<PtyAttachmentLeaseInner>);

impl PtyAttachmentLease {
    pub(crate) fn attachment_id(&self) -> &str {
        &self.0.attachment_id
    }

    pub(crate) fn attached_epoch(&self) -> u64 {
        self.0.attached_epoch
    }

    pub(crate) fn release(&self) -> u64 {
        self.0.release()
    }
}

#[derive(Default)]
struct OutputState {
    buf: VecDeque<u8>,
    /// True when the reader thread observed EOF. Subsequent reads
    /// return whatever's left then surface `code: "session_dead"`
    /// so the backend's PTYDriver can map it to SessionDeadError.
    closed: bool,
}

/// Process-wide I/O state. Cloneable handle (the inner state lives
/// behind Arc<Mutex>). Held by every registered handler closure so
/// they share one session table.
#[derive(Clone)]
pub struct PtyIoService {
    inner: Arc<Mutex<HashMap<PtySessionId, Arc<SessionIo>>>>,
}

impl Default for PtyIoService {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyIoService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Drop the I/O state for a session. Called from the close
    /// wrapper so the reader thread exits and the writer fd is
    /// released. Idempotent: returns true when a row was present,
    /// false when the session never had any I/O calls.
    pub fn drop_session(&self, id: &PtySessionId) -> bool {
        let mut g = self.inner.lock().expect("pty io service lock");
        match g.remove(id) {
            Some(io) => {
                io.dropped.store(true, std::sync::atomic::Ordering::Release);
                // Wake any pending reader so it sees `closed` /
                // `dropped` and returns immediately.
                let (lock, cv) = &*io.output;
                if let Ok(mut s) = lock.lock() {
                    s.closed = true;
                    cv.notify_all();
                }
                true
            }
            None => false,
        }
    }

    /// Look up an existing I/O row without lazily creating one.
    /// Used by the `timeout = 0` non-blocking poll path so a caller
    /// asking for "current buffer state, don't block" never pays
    /// the OS-thread-spawn cost a fresh `get_or_init` incurs. On a
    /// contended cargo-test machine that spawn observably ran in
    /// the 50-200ms range — well outside the 500ms upper bound the
    /// `read_with_zero_timeout_returns_immediately_when_empty` test
    /// pins, which was the source of the suite-flake before this
    /// surface landed.
    fn get_existing(&self, id: &PtySessionId) -> Option<Arc<SessionIo>> {
        let g = self.inner.lock().expect("pty io service lock");
        g.get(id).cloned()
    }

    /// Look up or lazily create an I/O row for `id`. Returns the
    /// shared row. Creating a new row spawns the reader thread.
    fn get_or_init(
        &self,
        pty: &Arc<PtyService>,
        id: &PtySessionId,
    ) -> anyhow::Result<Arc<SessionIo>> {
        // Fast path: row already exists.
        {
            let g = self.inner.lock().expect("pty io service lock");
            if let Some(io) = g.get(id) {
                return Ok(Arc::clone(io));
            }
        }
        // Slow path: build the row.
        let session = pty
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("unknown session_id `{}`", id.as_str()))?;

        let output = Arc::new((Mutex::new(OutputState::default()), Condvar::new()));
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Reader thread. Uses portable-pty's blocking reader on a
        // dedicated std::thread (NOT a tokio task) because the
        // read is synchronous and will not yield. We dedicate the
        // thread to one session for its lifetime.
        let reader = {
            let session = Arc::clone(&session);
            let output = Arc::clone(&output);
            let dropped = Arc::clone(&dropped);
            thread::Builder::new()
                .name(format!("pty-io-reader-{}", id.as_str()))
                .spawn(move || {
                    // try_clone_reader on portable-pty returns a
                    // fresh fd dup; we own it for the thread's life.
                    // The async lock is async; bridge by blocking on
                    // a tiny dedicated runtime — same pattern the
                    // bidi attach handler uses.
                    let reader = match futures::executor::block_on(async {
                        let m = session.master.lock().await;
                        m.try_clone_reader()
                    }) {
                        Ok(r) => r,
                        Err(_) => {
                            // PTY can't lend a reader. Surface as
                            // closed so subsequent reads return
                            // session_dead immediately.
                            let (lock, cv) = &*output;
                            if let Ok(mut s) = lock.lock() {
                                s.closed = true;
                                cv.notify_all();
                            }
                            return;
                        }
                    };
                    let mut reader = reader;
                    let mut buf = vec![0u8; READ_CHUNK_BYTES];
                    use std::io::Read;
                    loop {
                        if dropped.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        let n = match reader.read(&mut buf) {
                            Ok(0) => {
                                // EOF: child closed, mark closed so
                                // the next read drains then errors.
                                let (lock, cv) = &*output;
                                if let Ok(mut s) = lock.lock() {
                                    s.closed = true;
                                    cv.notify_all();
                                }
                                return;
                            }
                            Ok(n) => n,
                            Err(_) => {
                                // I/O error → treat as EOF. Same
                                // policy as terminal_attach_ability.
                                let (lock, cv) = &*output;
                                if let Ok(mut s) = lock.lock() {
                                    s.closed = true;
                                    cv.notify_all();
                                }
                                return;
                            }
                        };
                        let (lock, cv) = &*output;
                        let mut s = lock.lock().expect("pty io output lock");
                        s.buf.extend(&buf[..n]);
                        // Drop oldest bytes when over cap so a
                        // runaway producer can't OOM the daemon.
                        // The drop is contiguous to keep the buffer
                        // a single ring-shaped run; cv.notify_all
                        // still fires so a waiting reader unblocks
                        // even on a near-instant overflow.
                        if s.buf.len() > OUTPUT_BUFFER_CAP_BYTES {
                            let excess = s.buf.len() - OUTPUT_BUFFER_CAP_BYTES;
                            s.buf.drain(..excess);
                        }
                        cv.notify_all();
                    }
                })
                .map_err(|e| anyhow::anyhow!("spawn pty-io-reader: {e}"))?
        };

        let io = Arc::new(SessionIo {
            output,
            writer: Mutex::new(None),
            _reader: reader,
            dropped,
            attachment: Mutex::new(AttachmentState::default()),
        });

        // Insert + double-check: if another thread won the race
        // we discard our row (its reader will exit when
        // `dropped` is set on the loser). This keeps the table
        // single-rowed without a global lock around get+spawn.
        let mut g = self.inner.lock().expect("pty io service lock");
        match g.get(id) {
            Some(existing) => {
                io.dropped.store(true, std::sync::atomic::Ordering::Release);
                let (lock, cv) = &*io.output;
                if let Ok(mut s) = lock.lock() {
                    s.closed = true;
                    cv.notify_all();
                }
                Ok(Arc::clone(existing))
            }
            None => {
                g.insert(id.clone(), Arc::clone(&io));
                Ok(io)
            }
        }
    }

    pub(crate) fn claim_attachment(
        &self,
        pty: &Arc<PtyService>,
        id: &PtySessionId,
        attachment_id: &str,
        expected_epoch: u64,
    ) -> anyhow::Result<PtyAttachmentLease> {
        if let Some(claimed) = pty.supervised_claim(id, attachment_id, expected_epoch) {
            let attached_epoch = claimed?;
            return Ok(PtyAttachmentLease(Arc::new(PtyAttachmentLeaseInner {
                backend: PtyAttachmentLeaseBackend::Supervised {
                    pty: Arc::clone(pty),
                    session_id: id.clone(),
                },
                attachment_id: attachment_id.to_string(),
                attached_epoch,
                released: std::sync::atomic::AtomicBool::new(false),
            })));
        }
        let row = self.get_or_init(pty, id)?;
        let attached_epoch = {
            let mut state = row.attachment.lock().expect("pty attachment state lock");
            if state.epoch != expected_epoch {
                anyhow::bail!(
                    "ATTACHMENT_STALE: session `{}` epoch is {}, caller expected {}",
                    id.as_str(),
                    state.epoch,
                    expected_epoch
                );
            }
            if let Some(active) = state.active.as_deref() {
                anyhow::bail!(
                    "SESSION_ALREADY_ATTACHED: session `{}` is attached as `{active}`",
                    id.as_str()
                );
            }
            state.epoch = state.epoch.saturating_add(1);
            state.active = Some(attachment_id.to_string());
            state.epoch
        };
        Ok(PtyAttachmentLease(Arc::new(PtyAttachmentLeaseInner {
            backend: PtyAttachmentLeaseBackend::Local(row),
            attachment_id: attachment_id.to_string(),
            attached_epoch,
            released: std::sync::atomic::AtomicBool::new(false),
        })))
    }

    pub(crate) fn attachment_snapshot(
        &self,
        pty: &Arc<PtyService>,
        id: &PtySessionId,
    ) -> (u64, Option<String>) {
        if let Some(snapshot) = pty.supervised_attachment(id) {
            return snapshot.unwrap_or((0, None));
        }
        let Some(row) = self.get_existing(id) else {
            return (0, None);
        };
        let state = row.attachment.lock().expect("pty attachment state lock");
        (state.epoch, state.active.clone())
    }

    pub(crate) fn write_bytes(
        &self,
        pty: &Arc<PtyService>,
        id: &PtySessionId,
        bytes: &[u8],
    ) -> anyhow::Result<PtyIoWriteOutcome> {
        if bytes.is_empty() {
            return Ok(PtyIoWriteOutcome::Written(0));
        }
        if let Some(written) = pty.supervised_write(id, bytes) {
            return Ok(if written? {
                PtyIoWriteOutcome::Written(bytes.len())
            } else {
                PtyIoWriteOutcome::Closed
            });
        }
        let row = self.get_or_init(pty, id)?;
        {
            let mut writer = row.writer.lock().expect("pty io writer slot");
            if writer.is_none() {
                let session = pty
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("unknown session_id `{}`", id.as_str()))?;
                *writer = Some(
                    futures::executor::block_on(async {
                        let master = session.master.lock().await;
                        master.take_writer()
                    })
                    .map_err(|error| anyhow::anyhow!("take_writer: {error}"))?,
                );
            }
        }
        let mut writer = row.writer.lock().expect("pty io writer slot");
        let writer = writer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("writer disappeared between init and use"))?;
        use std::io::Write;
        if writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .is_err()
        {
            let (lock, cv) = &*row.output;
            if let Ok(mut state) = lock.lock() {
                state.closed = true;
                cv.notify_all();
            }
            return Ok(PtyIoWriteOutcome::Closed);
        }
        Ok(PtyIoWriteOutcome::Written(bytes.len()))
    }

    pub(crate) fn read_bytes(
        &self,
        pty: &Arc<PtyService>,
        id: &PtySessionId,
        timeout: Duration,
        max_bytes: usize,
    ) -> anyhow::Result<PtyIoReadOutcome> {
        if let Some(outcome) = pty.supervised_read(id, timeout, max_bytes) {
            let outcome = outcome?;
            return Ok(PtyIoReadOutcome {
                data: outcome.data,
                closed: outcome.closed,
                dropped_bytes: outcome.dropped_bytes,
            });
        }
        let row = self.get_or_init(pty, id)?;
        let (lock, cv) = &*row.output;
        let mut state = lock.lock().expect("pty io output lock");
        let deadline = Instant::now() + timeout;
        while state.buf.is_empty() && !state.closed {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (next, timed_out) = cv
                .wait_timeout(state, deadline - now)
                .expect("pty io output wait");
            state = next;
            if timed_out.timed_out() {
                break;
            }
        }
        let take = state.buf.len().min(max_bytes);
        let data = state.buf.drain(..take).collect();
        Ok(PtyIoReadOutcome {
            data,
            closed: state.closed,
            dropped_bytes: 0,
        })
    }
}

/// Register the three RPCs on the dispatcher. Caller threads the
/// shared `PtyService` (lifecycle) + `PtyIoService` (I/O state) so
/// every handler observes the same session and state tables.
///
pub fn register(reg: &mut AxonAbilityCatalog, pty: Arc<PtyService>, io: PtyIoService) {
    let owner = OwnerKind::terminal_system();
    {
        let pty = Arc::clone(&pty);
        let io = io.clone();
        let handler = Arc::new(move |env, args: Value| {
            let input_args = TerminalInputArgs::parse(args)?;
            super::authority::require_session_authority(
                &env,
                input_args.session_id(),
                "terminal.input",
            )?;
            input_session(&pty, &io, input_args)
        });
        reg.register_rpc_with_envelope_and_owner("terminal.input", owner.clone(), handler);
    }
    {
        let pty = Arc::clone(&pty);
        let io = io.clone();
        let handler = Arc::new(move |env, args: Value| {
            let read_args = TerminalReadArgs::parse(args)?;
            super::authority::require_session_authority(
                &env,
                read_args.session_id(),
                "terminal.read",
            )?;
            read_session(&pty, &io, read_args)
        });
        reg.register_rpc_with_envelope_and_owner("terminal.read", owner.clone(), handler);
    }
    {
        let pty = Arc::clone(&pty);
        let handler = Arc::new(move |env, args: Value| {
            let resize_args = TerminalResizeArgs::parse(args)?;
            super::authority::require_session_authority(
                &env,
                resize_args.session_id(),
                "terminal.resize",
            )?;
            resize_session(&pty, resize_args)
        });
        reg.register_rpc_with_envelope_and_owner("terminal.resize", owner, handler);
    }
}

/// `terminal.input` handler.
///
/// Args: `{ session_id: string, data: string (base64) }`.
/// Returns: `{ ack: bool, bytes_written: int }`. ack=false +
/// `error` field on a malformed request; ack=false + `code:
/// "session_dead"` when the PTY reported EOF/error before this
/// write completed.
#[cfg(test)]
fn input_handler(pty: &Arc<PtyService>, io: &PtyIoService, args: Value) -> anyhow::Result<Value> {
    input_session(pty, io, TerminalInputArgs::parse(args)?)
}

fn input_session(
    pty: &Arc<PtyService>,
    io: &PtyIoService,
    input_args: TerminalInputArgs,
) -> anyhow::Result<Value> {
    let bytes = input_args.bytes;
    let id = PtySessionId::new(input_args.session_id);
    match io.write_bytes(pty, &id, &bytes)? {
        PtyIoWriteOutcome::Written(written) => Ok(json!({"ack": true, "bytes_written": written})),
        PtyIoWriteOutcome::Closed => Ok(json!({
            "ack": false,
            "bytes_written": 0,
            "code": "session_dead",
        })),
    }
}

/// `terminal.read` handler.
///
/// Args: `{ session_id: string, timeout?: number (seconds) }`.
/// Returns:
///   * `{ output: string (base64), bytes: int }` on success (output
///     may be empty when the timeout hit with no new bytes).
///   * `{ output: "", bytes: 0, code: "session_dead" }` after the
///     PTY reported EOF AND the buffer is fully drained — backend's
///     PTYDriver maps this to `SessionDeadError`.
///   * `{ output: "", bytes: 0, code: "session_not_found" }` when
///     the underlying PtyService row is gone (lifecycle close
///     fired before this call landed).
#[cfg(test)]
fn read_handler(pty: &Arc<PtyService>, io: &PtyIoService, args: Value) -> anyhow::Result<Value> {
    read_session(pty, io, TerminalReadArgs::parse(args)?)
}

fn read_session(
    pty: &Arc<PtyService>,
    io: &PtyIoService,
    read_args: TerminalReadArgs,
) -> anyhow::Result<Value> {
    let session_id = read_args.session_id;
    let id = PtySessionId::new(&session_id);

    if !pty.try_contains(&id)? {
        return Ok(json!({
            "output": "",
            "bytes": 0,
            "code": "session_not_found",
        }));
    }

    let timeout_secs = read_args.timeout_secs;

    // Non-blocking poll fast path. When the caller said `timeout = 0`
    // ("give me current state, don't block"), look up the row
    // without lazily spawning the reader thread. Two reasons:
    //
    //   1. OS-thread spawn under `cargo test --tests` parallel load
    //      observably ran 50-200ms — well past the 500ms slack the
    //      non-blocking contract test pins.
    //   2. A "first call" with timeout=0 cannot have any buffered
    //      bytes by definition (nobody read yet, but more
    //      importantly nobody _wrote_ to a buffer that doesn't
    //      exist), so returning empty + 0 bytes is the
    //      semantically-correct answer. The next call with
    //      timeout>0 lazily initialises the reader, restoring
    //      catch-up behaviour identical to the pre-fastpath path.
    if timeout_secs == 0.0 && !pty.is_supervised() {
        let Some(row) = io.get_existing(&id) else {
            return Ok(json!({
                "output": "",
                "bytes": 0,
            }));
        };
        let (lock, _cv) = &*row.output;
        let mut state = lock.lock().expect("pty io output lock");
        let take = state.buf.len().min(MAX_READ_CHUNK_BYTES);
        let chunk: Vec<u8> = state.buf.drain(..take).collect();
        let closed = state.closed;
        drop(state);
        let mut resp = json!({
            "bytes": chunk.len(),
            "output": base64::engine::general_purpose::STANDARD.encode(&chunk),
        });
        if closed && chunk.is_empty() {
            resp["code"] = json!("session_dead");
        }
        return Ok(resp);
    }

    let outcome = io.read_bytes(
        pty,
        &id,
        Duration::from_secs_f64(timeout_secs),
        MAX_READ_CHUNK_BYTES,
    )?;
    let chunk = outcome.data;
    let closed = outcome.closed;
    let dropped_bytes = outcome.dropped_bytes;

    let mut resp = json!({
        "bytes": chunk.len(),
        "output": base64::engine::general_purpose::STANDARD.encode(&chunk),
    });
    if dropped_bytes > 0 {
        resp["output_gap_bytes"] = json!(dropped_bytes);
    }
    // session_dead = closed AND buffer fully drained. The reader
    // does NOT echo session_dead while there's still queued
    // output, because the backend's caller will keep reading and
    // eventually see the empty + closed state.
    if closed && chunk.is_empty() {
        resp["code"] = json!("session_dead");
    }
    Ok(resp)
}

/// `terminal.resize` handler.
///
/// Args: `{ session_id: string, cols: int, rows: int }`.
/// Returns: `{ ack: bool }`. ack=false when the session_id is
/// unknown so a caller polling SIGWINCH on a closed session can
/// stop without special-casing.
#[cfg(test)]
fn resize_handler(pty: &Arc<PtyService>, args: Value) -> anyhow::Result<Value> {
    resize_session(pty, TerminalResizeArgs::parse(args)?)
}

fn resize_session(pty: &Arc<PtyService>, resize_args: TerminalResizeArgs) -> anyhow::Result<Value> {
    let id = PtySessionId::new(&resize_args.session_id);
    if !pty.try_contains(&id)? {
        return Ok(json!({"ack": false}));
    }
    futures::executor::block_on(pty.resize_session(&id, resize_args.cols, resize_args.rows))?;
    Ok(json!({"ack": true}))
}

// ── Argument parsing helpers ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalInputArgs {
    session_id: String,
    bytes: Vec<u8>,
}

impl TerminalInputArgs {
    fn parse(args: Value) -> anyhow::Result<Self> {
        let args = terminal_io_args_object(&args, "terminal.input", &["session_id", "data"])?;
        let session_id = terminal_io_required_session_id(args, "terminal.input")?;
        let data_b64 = terminal_io_required_string(args, "data", "terminal.input")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| anyhow::anyhow!("terminal.input: `data` base64 decode failed: {e}"))?;
        Ok(Self { session_id, bytes })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TerminalReadArgs {
    session_id: String,
    timeout_secs: f64,
}

impl TerminalReadArgs {
    fn parse(args: Value) -> anyhow::Result<Self> {
        let args = terminal_io_args_object(&args, "terminal.read", &["session_id", "timeout"])?;
        let session_id = terminal_io_required_session_id(args, "terminal.read")?;
        let timeout_secs = parse_timeout(args, "terminal.read")?;
        Ok(Self {
            session_id,
            timeout_secs,
        })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalResizeArgs {
    session_id: String,
    cols: u16,
    rows: u16,
}

impl TerminalResizeArgs {
    fn parse(args: Value) -> anyhow::Result<Self> {
        let args =
            terminal_io_args_object(&args, "terminal.resize", &["session_id", "cols", "rows"])?;
        let session_id = terminal_io_required_session_id(args, "terminal.resize")?;
        let cols = terminal_io_required_u16(args, "cols", "terminal.resize")?;
        let rows = terminal_io_required_u16(args, "rows", "terminal.resize")?;
        if cols == 0 || rows == 0 {
            anyhow::bail!("terminal.resize: cols and rows must be > 0");
        }
        Ok(Self {
            session_id,
            cols,
            rows,
        })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }
}

fn terminal_io_args_object<'a>(
    args: &'a Value,
    ability: &str,
    allowed_keys: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be an object"))?;
    let mut unknown = object
        .keys()
        .filter(|key| !allowed_keys.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        anyhow::bail!(
            "{ability}: unsupported argument field(s): {}",
            unknown.join(", ")
        );
    }
    Ok(object)
}

fn terminal_io_required_session_id(
    args: &Map<String, Value>,
    ability: &str,
) -> anyhow::Result<String> {
    let session_id = terminal_io_required_string(args, "session_id", ability)?.trim();
    if session_id.is_empty() {
        anyhow::bail!("{ability}: `session_id` must not be empty");
    }
    Ok(session_id.to_string())
}

fn terminal_io_required_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    ability: &str,
) -> anyhow::Result<&'a str> {
    args.get(key)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` required"))?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` must be a string"))
}

fn parse_timeout(args: &Map<String, Value>, ability: &str) -> anyhow::Result<f64> {
    let raw = match args.get("timeout") {
        None | Some(Value::Null) => return Ok(DEFAULT_READ_TIMEOUT_SECS),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("{ability}: `timeout` must be a number"))?,
    };
    if !raw.is_finite() || raw < 0.0 {
        anyhow::bail!("{ability}: `timeout` must be a non-negative finite number (got {raw})");
    }
    Ok(raw.min(MAX_READ_TIMEOUT_SECS))
}

fn terminal_io_required_u16(
    args: &Map<String, Value>,
    key: &str,
    ability: &str,
) -> anyhow::Result<u16> {
    match args.get(key) {
        None | Some(Value::Null) => anyhow::bail!("{ability}: `{key}` required"),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` must fit in u16 (got {n})")),
        Some(other) => anyhow::bail!("{ability}: `{key}` must be a number, got {other}"),
    }
}

// ── Discovery surfaces ───────────────────────────────────────────

pub fn input_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id", "data"],
        "additionalProperties": false,
        "properties": {
            "session_id": {"type": "string", "minLength": 1},
            "data": {"type": "string", "description": "base64-encoded stdin bytes"},
        },
    })
}

pub fn read_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "additionalProperties": false,
        "properties": {
            "session_id": {"type": "string", "minLength": 1},
            "timeout": {
                "type": "number",
                "minimum": 0,
                "maximum": MAX_READ_TIMEOUT_SECS,
                "description": "seconds to wait for output before returning empty",
            },
        },
    })
}

pub fn resize_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id", "cols", "rows"],
        "additionalProperties": false,
        "properties": {
            "session_id": {"type": "string", "minLength": 1},
            "cols": {"type": "integer", "minimum": 1, "maximum": 65535},
            "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
        },
    })
}

pub fn input_description() -> &'static str {
    "Push base64-encoded stdin bytes into a PTY session. Pairs \
     with terminal.read to form the unary RPC data plane \
     used by the EasyNet backend's PTYDriver."
}

pub fn read_description() -> &'static str {
    "Drain a PTY session's stdout up to a timeout. Returns base64 \
     bytes; sets `code: \"session_dead\"` once the child has \
     exited and the buffer is fully drained."
}

pub fn resize_description() -> &'static str {
    "Resize a PTY session's cols × rows. Returns ack=false on \
     unknown session_id so a caller polling SIGWINCH on a closed \
     session can stop without special-casing."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::execution::pty::PtyCreateSpec;

    fn fresh() -> (Arc<PtyService>, PtyIoService) {
        (Arc::new(PtyService::new()), PtyIoService::new())
    }

    fn spawn_sh(pty: &Arc<PtyService>) -> PtySessionId {
        // Use sh so the PTY echoes typed characters and runs
        // commands that produce stdout deterministically.
        pty.create(PtyCreateSpec {
            cols: 80,
            rows: 24,
            command: Some("/bin/sh".to_string()),
            command_args: vec![],
            cwd: None,
            env: HashMap::new(),
        })
        .expect("spawn /bin/sh")
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/terminal-io";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    #[test]
    fn registration_mounts_three_rpcs() {
        let mut reg = metadata_test_catalog();
        let (pty, io) = fresh();
        register(&mut reg, pty, io);
        assert!(reg.resolve_rpc_with_env(ABILITY_TERMINAL_INPUT).is_some());
        assert!(reg.resolve_rpc_with_env(ABILITY_TERMINAL_READ).is_some());
        assert!(reg.resolve_rpc_with_env(ABILITY_TERMINAL_RESIZE).is_some());
    }

    #[test]
    fn read_unknown_session_returns_session_not_found() {
        let (pty, io) = fresh();
        let resp = read_handler(&pty, &io, json!({"session_id": "nope"})).unwrap();
        assert_eq!(resp["code"], "session_not_found");
        assert_eq!(resp["bytes"], 0);
    }

    #[test]
    fn resize_unknown_session_returns_ack_false() {
        let (pty, _) = fresh();
        let resp =
            resize_handler(&pty, json!({"session_id": "nope", "cols": 80, "rows": 24})).unwrap();
        assert_eq!(resp["ack"], false);
    }

    #[test]
    fn input_unknown_session_errors() {
        let (pty, io) = fresh();
        let err = input_handler(&pty, &io, json!({"session_id": "nope", "data": "aGVsbG8="}))
            .unwrap_err();
        assert!(format!("{err}").contains("unknown session_id"));
    }

    #[test]
    fn input_rejects_missing_data() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let err = input_handler(&pty, &io, json!({"session_id": id.as_str()})).unwrap_err();
        assert!(format!("{err}").contains("`data`"));
        pty.close(&id);
    }

    #[test]
    fn input_rejects_unknown_fields_before_io_row_allocation() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let err = input_handler(
            &pty,
            &io,
            json!({"session_id": id.as_str(), "data": "aGk=", "legacy_mode": true}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.input: unsupported argument field(s): legacy_mode"),
            "unexpected error: {message}"
        );
        assert!(
            io.get_existing(&id).is_none(),
            "unknown fields must fail before writer allocation or I/O row creation"
        );
        pty.close(&id);
    }

    #[test]
    fn input_rejects_wrong_typed_session_id_before_io_row_allocation() {
        let (pty, io) = fresh();
        let err = input_handler(&pty, &io, json!({"session_id": 42, "data": "aGk="})).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.input: `session_id` must be a string"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn input_rejects_bad_base64() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let err = input_handler(
            &pty,
            &io,
            json!({"session_id": id.as_str(), "data": "@@@not-b64@@@"}),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("base64 decode"));
        pty.close(&id);
    }

    #[test]
    fn input_empty_data_acks_zero_bytes() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let resp =
            input_handler(&pty, &io, json!({"session_id": id.as_str(), "data": ""})).unwrap();
        assert_eq!(resp["ack"], true);
        assert_eq!(resp["bytes_written"], 0);
        pty.close(&id);
    }

    #[test]
    fn read_with_zero_timeout_returns_immediately_when_empty() {
        // A 0-second timeout is a non-blocking poll. With nothing
        // queued, returns empty without waiting.
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let start = Instant::now();
        let resp =
            read_handler(&pty, &io, json!({"session_id": id.as_str(), "timeout": 0})).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(resp["bytes"], 0);
        // Allow generous slack for slow CI but assert the call
        // didn't actually block for any meaningful duration.
        assert!(
            elapsed < Duration::from_millis(500),
            "0-timeout read blocked for {elapsed:?}"
        );
        pty.close(&id);
    }

    #[test]
    fn parse_timeout_clamps_to_max() {
        // A caller passing a billion seconds gets clamped to the
        // ceiling rather than letting the handler hang for years.
        let got = TerminalReadArgs::parse(json!({
            "session_id": "session-1",
            "timeout": 1_000_000.0
        }))
        .unwrap()
        .timeout_secs;
        assert_eq!(got, MAX_READ_TIMEOUT_SECS);
    }

    #[test]
    fn parse_timeout_rejects_negative() {
        let err = TerminalReadArgs::parse(json!({
            "session_id": "session-1",
            "timeout": -1.5
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("non-negative"));
    }

    #[test]
    fn parse_timeout_rejects_non_finite_when_decoded_from_string_form() {
        // serde_json's `json!()` macro sanitises f64::NAN/INFINITY
        // to `null`, so we can't construct a NaN-bearing Value via
        // the macro. Build the Number directly via from_f64 — it
        // returns None for non-finite inputs, exercising parse_timeout's
        // `as_f64()` branch with a missing-or-null timeout that
        // legitimately returns the default rather than an error.
        // To exercise the negative-or-NaN guard, use a string
        // "timeout" instead, which lands in the `as_f64() = None`
        // branch and surfaces a clean "must be a number" error.
        let mut obj = serde_json::Map::new();
        obj.insert("session_id".to_string(), json!("session-1"));
        obj.insert("timeout".to_string(), json!("not-a-number"));
        let err = TerminalReadArgs::parse(Value::Object(obj)).unwrap_err();
        assert!(format!("{err}").contains("number"));
    }

    #[test]
    fn read_rejects_unknown_fields_before_reader_allocation() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        let err = read_handler(
            &pty,
            &io,
            json!({"session_id": id.as_str(), "timeout": 0.05, "legacy_mode": true}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.read: unsupported argument field(s): legacy_mode"),
            "unexpected error: {message}"
        );
        assert!(
            io.get_existing(&id).is_none(),
            "unknown fields must fail before lazy reader allocation"
        );
        pty.close(&id);
    }

    #[test]
    fn read_rejects_blank_session_id_before_session_lookup() {
        let (pty, io) = fresh();
        let err = read_handler(&pty, &io, json!({"session_id": "   "})).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.read: `session_id` must not be empty"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn read_with_zero_timeout_does_not_lazily_spawn_reader() {
        // Non-blocking poll contract: a `timeout = 0` read on a
        // freshly-created session that has never been read before
        // returns 0 bytes WITHOUT spawning the reader thread / I/O
        // row. The next `timeout > 0` read still lazily initialises.
        // This is what makes the non-blocking path observably
        // bounded by the existing-row mutex acquisition latency
        // rather than by OS thread spawn.
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        // No row before the poll.
        assert!(io.get_existing(&id).is_none());
        let resp =
            read_handler(&pty, &io, json!({"session_id": id.as_str(), "timeout": 0})).unwrap();
        assert_eq!(resp["bytes"], 0);
        // Still no row after a zero-timeout poll.
        assert!(
            io.get_existing(&id).is_none(),
            "timeout=0 must not lazily initialise the reader row"
        );
        // A blocking read DOES initialise.
        let _ = read_handler(
            &pty,
            &io,
            json!({"session_id": id.as_str(), "timeout": 0.05}),
        )
        .unwrap();
        assert!(io.get_existing(&id).is_some());
        pty.close(&id);
    }

    #[test]
    fn drop_session_removes_row_and_signals_close() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);
        // Init the I/O row by reading once. Use a non-zero timeout
        // because `timeout = 0` now hits a non-blocking fast path
        // that does NOT lazily spawn the reader thread (see the
        // comment in `read_handler` for the rationale); a strictly
        // positive timeout still threads through `get_or_init` and
        // creates the row this test expects to drop below.
        let _ = read_handler(
            &pty,
            &io,
            json!({"session_id": id.as_str(), "timeout": 0.05}),
        )
        .unwrap();
        assert!(io.drop_session(&id), "first drop must report ack=true");
        assert!(
            !io.drop_session(&id),
            "second drop on same id must report ack=false (idempotent)"
        );
        pty.close(&id);
    }

    /// Real PTY round-trip: write a command, read its stdout.
    /// This is the integration-shaped test that proves the
    /// handler trio actually drives a child process — pre-fix
    /// the backend's PTYDriver would have called these names and
    /// gotten "no local handler" errors.
    #[test]
    fn round_trip_input_then_read_observes_command_output() {
        let (pty, io) = fresh();
        let id = spawn_sh(&pty);

        // Write a printf line that produces a deterministic
        // stdout marker. printf is more reliable than echo for
        // testing because some shells inject extra whitespace.
        let input = b"printf 'EASYNET_PTY_OK\\n'\n";
        let b64 = base64::engine::general_purpose::STANDARD.encode(input);
        let resp =
            input_handler(&pty, &io, json!({"session_id": id.as_str(), "data": b64})).unwrap();
        assert_eq!(resp["ack"], true);
        assert_eq!(resp["bytes_written"], input.len() as i64);

        // Drain output. We may need a couple of read cycles —
        // the shell prompt + the echo of the command + the
        // command's own output may arrive in chunks.
        let mut accum = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !accum.contains("EASYNET_PTY_OK") {
            let resp = read_handler(
                &pty,
                &io,
                json!({"session_id": id.as_str(), "timeout": 1.0}),
            )
            .unwrap();
            let bytes = resp["bytes"].as_u64().unwrap_or(0);
            if bytes > 0 {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(resp["output"].as_str().unwrap())
                    .unwrap();
                accum.push_str(&String::from_utf8_lossy(&raw));
            }
        }
        assert!(
            accum.contains("EASYNET_PTY_OK"),
            "expected printf marker in PTY output; got {accum:?}"
        );
        pty.close(&id);
    }

    /// Exit-then-read sequence: after the child exits, we drain
    /// queued output then surface session_dead.
    #[test]
    fn read_after_child_exit_drains_then_returns_session_dead() {
        let (pty, io) = fresh();
        // /bin/sh -c "printf MARK; exit" → child writes once and
        // exits immediately. The reader thread observes EOF and
        // sets `closed`.
        let id = pty
            .create(PtyCreateSpec {
                cols: 80,
                rows: 24,
                command: Some("/bin/sh".to_string()),
                command_args: vec!["-c".to_string(), "printf 'BYE_MARK'; exit".to_string()],
                cwd: None,
                env: HashMap::new(),
            })
            .unwrap();

        // Wait up to 5s for the marker to land + reader to mark
        // closed. We don't poll close itself — the read handler's
        // session_dead surfaces only after buffer drain.
        let mut got_mark = false;
        let mut got_dead = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !(got_mark && got_dead) {
            let resp = read_handler(
                &pty,
                &io,
                json!({"session_id": id.as_str(), "timeout": 0.5}),
            )
            .unwrap();
            if let Some(b64) = resp["output"].as_str() {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default();
                if String::from_utf8_lossy(&raw).contains("BYE_MARK") {
                    got_mark = true;
                }
            }
            if resp["code"] == "session_dead" {
                got_dead = true;
                break;
            }
        }
        assert!(got_mark, "child's printf output must be drained first");
        assert!(got_dead, "session_dead must surface after drain + EOF");
        pty.close(&id);
    }

    #[test]
    fn resize_actually_propagates_to_pty() {
        // Hard to assert ioctl effect from a unit test without
        // racing TIOCGWINSZ — but we CAN assert the call returns
        // ack=true on a live session, exercising the resize path.
        let (pty, _) = fresh();
        let id = spawn_sh(&pty);
        let resp = resize_handler(
            &pty,
            json!({"session_id": id.as_str(), "cols": 132, "rows": 50}),
        )
        .unwrap();
        assert_eq!(resp["ack"], true);
        pty.close(&id);
    }

    #[test]
    fn resize_rejects_zero_dimensions() {
        let (pty, _) = fresh();
        let id = spawn_sh(&pty);
        let err = resize_handler(
            &pty,
            json!({"session_id": id.as_str(), "cols": 0, "rows": 24}),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("> 0"));
        pty.close(&id);
    }

    #[test]
    fn resize_rejects_unknown_fields_before_session_lookup() {
        let (pty, _) = fresh();
        let id = spawn_sh(&pty);
        let err = resize_handler(
            &pty,
            json!({"session_id": id.as_str(), "cols": 80, "rows": 24, "legacy_mode": true}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.resize: unsupported argument field(s): legacy_mode"),
            "unexpected error: {message}"
        );
        pty.close(&id);
    }

    #[test]
    fn resize_rejects_wrong_typed_dimensions_before_session_lookup() {
        let (pty, _) = fresh();
        let err = resize_handler(
            &pty,
            json!({"session_id": "nope", "cols": "wide", "rows": 24}),
        )
        .unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("terminal.resize: `cols` must be a number"),
            "unexpected error: {message}"
        );
    }
}
