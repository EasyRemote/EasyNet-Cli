// EasyNet CLI — PtyService (C-M3b)
// =================================
//
// File: src/runtime/execution/pty/mod.rs
//
// Sub-service that owns every live PTY-hosted child process the
// daemon spawns through `terminal.create`. v1 surface is
// minimal: create one, close one, look one up. The `_attach` ability
// (C-M3c) borrows a session through `get` to wire the client's
// InvokeBidi frames against the PTY master fd.
//
// Why a sub-service rather than per-handler statics
// -------------------------------------------------
// PTY sessions outlive a single Invoke call. The create handler
// returns a session_id and exits; the child keeps running until
// `_close` (or until it exits on its own); the attach handler
// borrows the same session_id later. That requires process-wide
// state with a single owner — the same shape PermissionService
// owns the broker + pending queue. Pinning the ownership at the
// service boundary lets the create / close / attach handlers all
// hold an `Arc<PtyService>` without entangling each other.
//
// Concurrency model
// -----------------
// Every map operation goes through one `std::sync::Mutex` on the
// session table. Per-session work (read/write/resize) goes through
// a per-session `tokio::sync::Mutex` that lives inside the
// `PtySession` row, so a slow attach loop on session A does NOT
// block a create / close on session B.
//
// portable-pty wraps the unix openpty + child fork+exec sequence
// behind one `PtySystem` trait; v1 uses the native_pty_system().
// On macOS / Linux that lands on openpty(3) + execvpe(3); the
// trait gives us a path forward to ConPTY on Windows without
// touching our handlers.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Opaque identifier for one PTY session. v1 wraps a UUID v4 string
/// for printability + cheap equality. The dispatch path treats it
/// as opaque — handlers never parse it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PtySessionId(String);

impl PtySessionId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PtySessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Spec the create handler hands the service. Field shapes match the
/// terminal.create wire schema (see `pty_lifecycle_ability.rs`).
#[derive(Debug, Clone)]
pub struct PtyCreateSpec {
    /// Initial column count. Defaults to 80 when unset.
    pub cols: u16,
    /// Initial row count. Defaults to 24 when unset.
    pub rows: u16,
    /// Command to run inside the PTY. Defaults to the user's
    /// `$SHELL` (or `/bin/sh` if unset). The child's argv0 = the
    /// command itself; v1 has no argv override knob — extending
    /// args is what `command_args` is for.
    pub command: Option<String>,
    /// Extra argv tail. Empty by default.
    pub command_args: Vec<String>,
    /// Working directory. Defaults to the daemon's cwd.
    pub cwd: Option<String>,
    /// Extra environment variables to inject (merged with the
    /// daemon's env, overriding any collisions).
    pub env: HashMap<String, String>,
}

/// Read-only snapshot of a live PTY session. This is the daemon-side
/// truth exposed by `terminal.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySessionSnapshot {
    pub id: PtySessionId,
    pub created_unix_ms: u64,
    pub command: Option<String>,
    pub command_args: Vec<String>,
    pub cwd: Option<String>,
}

impl Default for PtyCreateSpec {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            command: None,
            command_args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
        }
    }
}

/// One live PTY session row. The master is the only handle we keep
/// around: it's both the read side (output the child wrote) and the
/// write side (input we feed the child) and the resize handle. The
/// child handle lives separately because its `wait()` is blocking;
/// the close handler spawns a thread to reap it.
///
/// `MasterPty` is `Send` but not `Sync`; wrapping in a tokio Mutex
/// keeps per-session attach loops single-writer without forcing
/// every borrow path to be async (close can use `try_lock`).
pub struct PtySession {
    pub id: PtySessionId,
    pub created_unix_ms: u64,
    pub command: Option<String>,
    pub command_args: Vec<String>,
    pub cwd: Option<String>,
    pub master: tokio::sync::Mutex<Box<dyn MasterPty + Send>>,
    /// Boxed Child trait object — portable-pty owns process
    /// ownership. We take it via Mutex<Option<...>> because
    /// `kill()` consumes self in some builds; close path takes the
    /// Option, leaves None, then drops the box after kill+wait.
    pub child: std::sync::Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl PtySession {
    /// Resize the PTY. Plumbed through `terminal.resize`;
    /// attach can wire SIGWINCH handling without touching this
    /// struct.
    pub async fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        let m = self.master.lock().await;
        m.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow::anyhow!("resize: {e}"))
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("id", &self.id)
            // Skip master + child — they have no useful Debug.
            .finish_non_exhaustive()
    }
}

/// Outcome of a close call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyCloseOutcome {
    /// True when a row matched; false when the id was unknown
    /// (idempotent close — the surface is "remove if present").
    pub ack: bool,
    /// Exit status reported by the child, when waitable. None when
    /// the row was unknown OR when the child was killed before wait
    /// could grab a status. u32 preserves portable-pty's own type;
    /// POSIX exit codes are 0-255 in practice.
    pub exit_status: Option<u32>,
}

/// Process-wide PTY session registry. Cloneable handle (the inner
/// state lives behind `Arc<Mutex<...>>`).
#[derive(Clone)]
pub struct PtyService {
    inner: Arc<Mutex<PtyServiceInner>>,
}

struct PtyServiceInner {
    sessions: HashMap<PtySessionId, Arc<PtySession>>,
}

impl Default for PtyService {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PtyServiceInner {
                sessions: HashMap::new(),
            })),
        }
    }

    /// Create a new PTY-hosted child per the spec. Returns the new
    /// session id; the row is stored in the service and reachable
    /// via `get`.
    ///
    /// Errors:
    ///   * `openpty` failure (rare; usually ENFILE or out-of-pty
    ///     resources)
    ///   * spawn failure (command not found, permission denied)
    pub fn create(&self, spec: PtyCreateSpec) -> anyhow::Result<PtySessionId> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                cols: spec.cols,
                rows: spec.rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| anyhow::anyhow!("openpty: {e}"))?;

        let cmd_str = spec
            .command
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(&cmd_str);
        for arg in &spec.command_args {
            cmd.arg(arg);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("spawn `{cmd_str}`: {e}"))?;
        // Slave fd held by the child; drop our handle so the only
        // process owning it is the child.
        drop(pair.slave);

        let id = PtySessionId::new(uuid::Uuid::new_v4().to_string());
        let created_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let session = Arc::new(PtySession {
            id: id.clone(),
            created_unix_ms,
            command: Some(cmd_str),
            command_args: spec.command_args.clone(),
            cwd: spec.cwd.clone(),
            master: tokio::sync::Mutex::new(pair.master),
            child: std::sync::Mutex::new(Some(child)),
        });

        let mut g = self.inner.lock().expect("pty service lock");
        g.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Look up a live session. Returns the same `Arc` the service
    /// holds, so callers (e.g. the future attach handler) can keep
    /// it alive across `await` points without taking the service
    /// lock for the whole loop.
    pub fn get(&self, id: &PtySessionId) -> Option<Arc<PtySession>> {
        let g = self.inner.lock().expect("pty service lock");
        g.sessions.get(id).cloned()
    }

    /// Return a stable snapshot of every live PTY session. The caller
    /// receives cloned metadata only; PTY handles never escape.
    pub fn list(&self) -> Vec<PtySessionSnapshot> {
        let g = self.inner.lock().expect("pty service lock");
        let mut sessions = g
            .sessions
            .values()
            .map(|session| PtySessionSnapshot {
                id: session.id.clone(),
                created_unix_ms: session.created_unix_ms,
                command: session.command.clone(),
                command_args: session.command_args.clone(),
                cwd: session.cwd.clone(),
            })
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.created_unix_ms);
        sessions
    }

    /// Close a session. Idempotent: returns `ack: false` when the id
    /// was unknown (an honest signal — the surface contract is
    /// "remove if present"; raising would force every caller to
    /// special-case "already gone").
    ///
    /// Sequence:
    ///   1. Remove the session row (no further attach can claim it).
    ///   2. Take the child out of its slot, kill it. portable-pty's
    ///      Child::kill() sends SIGHUP on unix (verified against
    ///      portable-pty 0.8 src/unix.rs) — most shells handle that
    ///      as "session disconnected" and clean up gracefully. We
    ///      don't escalate to SIGKILL because the close path is
    ///      synchronous from the wire's perspective; a stuck child
    ///      can be reaped by the OS later.
    ///   3. Try-wait once for the exit status; if it's not yet
    ///      reaped return `exit_status: None`. The OS reaper still
    ///      claims it.
    pub fn close(&self, id: &PtySessionId) -> PtyCloseOutcome {
        let session = {
            let mut g = self.inner.lock().expect("pty service lock");
            g.sessions.remove(id)
        };
        let Some(session) = session else {
            return PtyCloseOutcome {
                ack: false,
                exit_status: None,
            };
        };
        let mut child_slot = session.child.lock().expect("pty child slot");
        let Some(mut child) = child_slot.take() else {
            // Already taken (close racing with itself). Treat as
            // ack=true since the row was present when we removed it.
            return PtyCloseOutcome {
                ack: true,
                exit_status: None,
            };
        };
        // SIGHUP on unix; ignored if the child already exited.
        let _ = child.kill();
        // exit_code() returns u32; preserve as u32 (POSIX exit codes
        // are 0-255). The previous `as i32` cast was a footgun for
        // the rare cases when extended status fields encode signal
        // info in high bits.
        let exit_status = child.try_wait().ok().flatten().map(|s| s.exit_code());
        PtyCloseOutcome {
            ack: true,
            exit_status,
        }
    }

    /// Test/operator helper: how many sessions are currently alive.
    pub fn live_count(&self) -> usize {
        let g = self.inner.lock().expect("pty service lock");
        g.sessions.len()
    }
}

impl std::fmt::Debug for PtyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyService")
            .field("live_count", &self.live_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform test harness pin: `true` /bin/true is in $PATH on
    /// macOS + every Linux distro CI ever runs on; using it instead of
    /// $SHELL keeps the test deterministic + fast (true exits with 0
    /// immediately, so close path doesn't race the spawn).
    fn true_spec() -> PtyCreateSpec {
        // /usr/bin/true exists on macOS; on some Linux distros true
        // is at /bin/true. Pick whichever exists.
        let command = if std::path::Path::new("/usr/bin/true").exists() {
            "/usr/bin/true"
        } else {
            "/bin/true"
        };
        PtyCreateSpec {
            command: Some(command.to_string()),
            ..PtyCreateSpec::default()
        }
    }

    #[test]
    fn new_service_starts_empty() {
        assert_eq!(PtyService::new().live_count(), 0);
    }

    #[test]
    fn create_then_close_round_trips_and_decrements_count() {
        let svc = PtyService::new();
        let id = svc.create(true_spec()).expect("create true");
        assert_eq!(svc.live_count(), 1);
        let out = svc.close(&id);
        assert!(out.ack, "close of live session must ack=true");
        assert_eq!(svc.live_count(), 0);
    }

    #[test]
    fn close_unknown_id_returns_ack_false_without_panic() {
        let svc = PtyService::new();
        let out = svc.close(&PtySessionId::new("not-a-session"));
        assert!(!out.ack, "close of unknown id must ack=false (idempotent)");
        assert_eq!(out.exit_status, None);
    }

    #[test]
    fn close_is_idempotent_second_call_is_ack_false() {
        let svc = PtyService::new();
        let id = svc.create(true_spec()).expect("create true");
        let first = svc.close(&id);
        assert!(first.ack);
        let second = svc.close(&id);
        assert!(!second.ack, "second close of same id must ack=false");
    }

    #[test]
    fn get_returns_session_until_close_drops_it() {
        let svc = PtyService::new();
        let id = svc.create(true_spec()).expect("create true");
        assert!(svc.get(&id).is_some(), "get of live session must succeed");
        svc.close(&id);
        assert!(svc.get(&id).is_none(), "get after close must miss");
    }

    #[test]
    fn create_reports_spawn_error_clearly() {
        let svc = PtyService::new();
        let spec = PtyCreateSpec {
            command: Some("/this/path/should/not/exist/ever".to_string()),
            ..PtyCreateSpec::default()
        };
        let err = svc.create(spec).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("spawn"),
            "spawn error must mention the spawn step; got {msg:?}"
        );
    }

    #[test]
    fn create_three_sessions_each_gets_distinct_id() {
        let svc = PtyService::new();
        let a = svc.create(true_spec()).unwrap();
        let b = svc.create(true_spec()).unwrap();
        let c = svc.create(true_spec()).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        assert_eq!(svc.live_count(), 3);
        // Cleanup so the test doesn't leak processes.
        svc.close(&a);
        svc.close(&b);
        svc.close(&c);
        assert_eq!(svc.live_count(), 0);
    }

    #[tokio::test]
    async fn resize_succeeds_on_a_live_session() {
        let svc = PtyService::new();
        let id = svc.create(true_spec()).expect("create true");
        let s = svc.get(&id).expect("get live session");
        s.resize(120, 40).await.expect("resize must succeed");
        svc.close(&id);
    }
}
