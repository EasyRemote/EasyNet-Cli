// EasyNet CLI - persistent PTY session supervisor
// ================================================
//
// File: src/daemon/execution/pty/supervisor.rs
// Description: Per-user process that owns PTY handles independently of the
//              EasyNet daemon and exposes a bounded local UDS protocol.
//
// Runtime Boundary
// ----------------
// This process is an OS-handle custodian, not an EasyNet Runtime. It cannot
// route, admit, execute, or sign an Invocation. The daemon remains the sole
// policy/runtime owner and reaches this process only after terminal Ability
// admission has succeeded.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use super::{PtyCloseOutcome, PtyCreateSpec, PtySessionId, PtySessionSnapshot};

// Version the endpoint with the framing contract. A process from an older
// release may still own its socket; a new daemon must never speak a new binary
// protocol to that process or race it for the same journal.
const SOCKET_FILE: &str = "session-supervisor-v2.sock";
const JOURNAL_FILE: &str = "terminal-sessions-v2.json";
const MAX_REQUEST_BYTES: u64 = 2 * 1024 * 1024;
const OUTPUT_BUFFER_CAP_BYTES: usize = 4 * 1024 * 1024;
const READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_SESSIONS: usize = 32;
const CLIENT_CONNECTION_LANES: usize = 16;
const SERVER_WORKERS: usize = 32;
const ACCEPT_QUEUE_BOUND: usize = 64;

#[derive(Debug)]
struct SupervisorRejected(String);

impl std::fmt::Display for SupervisorRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SupervisorRejected {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Ping,
    Reconcile {
        controller_id: String,
    },
    Create {
        spec: PtyCreateSpec,
    },
    List,
    Close {
        session_id: String,
    },
    Exists {
        session_id: String,
    },
    Write {
        session_id: String,
    },
    Read {
        session_id: String,
        timeout_ms: u64,
        max_bytes: usize,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Claim {
        session_id: String,
        attachment_id: String,
        expected_epoch: u64,
    },
    Release {
        session_id: String,
        attachment_id: String,
        attached_epoch: u64,
    },
    Attachment {
        session_id: String,
    },
    ExitStatus {
        session_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Response {
    fn value(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorReadOutcome {
    pub data: Vec<u8>,
    pub closed: bool,
    pub dropped_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SupervisorClient {
    socket_path: PathBuf,
    controller_id: String,
    #[cfg(unix)]
    connections: Arc<SupervisorConnectionPool>,
}

#[cfg(unix)]
#[derive(Debug)]
struct SupervisorConnectionPool {
    lanes: Vec<Mutex<Option<std::os::unix::net::UnixStream>>>,
    next: AtomicUsize,
}

#[cfg(unix)]
impl SupervisorConnectionPool {
    fn new() -> Self {
        Self {
            lanes: (0..CLIENT_CONNECTION_LANES)
                .map(|_| Mutex::new(None))
                .collect(),
            next: AtomicUsize::new(0),
        }
    }
}

impl SupervisorClient {
    pub fn for_current_user() -> Self {
        Self {
            socket_path: socket_path(),
            controller_id: process_controller_id().to_string(),
            #[cfg(unix)]
            connections: Arc::new(SupervisorConnectionPool::new()),
        }
    }

    fn call(&self, request: Request) -> anyhow::Result<serde_json::Value> {
        self.call_with_body(request, &[]).map(|(value, _)| value)
    }

    #[cfg(unix)]
    fn call_with_body(
        &self,
        request: Request,
        body: &[u8],
    ) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
        let lane =
            self.connections.next.fetch_add(1, Ordering::Relaxed) % self.connections.lanes.len();
        let mut connection = self.connections.lanes[lane]
            .lock()
            .expect("supervisor connection lane lock");
        match self.call_once(&mut connection, &request, body) {
            Ok(value) => Ok(value),
            Err(error) if error.is::<SupervisorRejected>() => Err(error),
            Err(first_error) => {
                *connection = None;
                self.start().with_context(|| {
                    format!("start PTY session supervisor after {first_error:#}")
                })?;
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    match self.call_once(&mut connection, &request, body) {
                        Ok(value) => return Ok(value),
                        Err(error) if error.is::<SupervisorRejected>() => return Err(error),
                        Err(error) if std::time::Instant::now() < deadline => {
                            *connection = None;
                            std::thread::sleep(Duration::from_millis(25));
                            let _ = error;
                        }
                        Err(error) => {
                            return Err(error).context("connect to PTY session supervisor")
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    fn call_once(
        &self,
        connection: &mut Option<std::os::unix::net::UnixStream>,
        request: &Request,
        body: &[u8],
    ) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
        use std::os::unix::net::UnixStream;

        if connection.is_none() {
            let stream = UnixStream::connect(&self.socket_path)?;
            stream.set_read_timeout(Some(Duration::from_secs(65)))?;
            stream.set_write_timeout(Some(Duration::from_secs(10)))?;
            *connection = Some(stream);
            let reconcile = Request::Reconcile {
                controller_id: self.controller_id.clone(),
            };
            exchange_frame(
                connection.as_mut().expect("connected supervisor lane"),
                &reconcile,
                &[],
            )?;
        }
        exchange_frame(
            connection.as_mut().expect("connected supervisor lane"),
            request,
            body,
        )
    }

    #[cfg(not(unix))]
    fn call_with_body(
        &self,
        _request: Request,
        _body: &[u8],
    ) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
        anyhow::bail!("PTY session supervisor transport is unsupported on this platform")
    }

    fn start(&self) -> anyhow::Result<()> {
        let current = std::env::current_exe().context("resolve current executable")?;
        let sibling = current.with_file_name(supervisor_executable_name());
        let executable = if sibling.is_file() { sibling } else { current };
        let mut command = std::process::Command::new(executable);
        command
            .env("EASYNET_INTERNAL_SESSION_SUPERVISOR", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn().context("spawn PTY session supervisor")?;
        Ok(())
    }

    pub fn create(&self, spec: PtyCreateSpec) -> anyhow::Result<PtySessionId> {
        let value = self.call(Request::Create { spec })?;
        Ok(PtySessionId::new(required_string(&value, "session_id")?))
    }

    pub fn list(&self) -> anyhow::Result<Vec<PtySessionSnapshot>> {
        serde_json::from_value(self.call(Request::List)?).context("decode session supervisor list")
    }

    pub fn close(&self, id: &PtySessionId) -> anyhow::Result<PtyCloseOutcome> {
        serde_json::from_value(self.call(Request::Close {
            session_id: id.as_str().to_string(),
        })?)
        .context("decode session supervisor close")
    }

    pub fn exists(&self, id: &PtySessionId) -> anyhow::Result<bool> {
        self.call(Request::Exists {
            session_id: id.as_str().to_string(),
        })?
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("session supervisor exists response must be boolean"))
    }

    pub fn write(&self, id: &PtySessionId, data: &[u8]) -> anyhow::Result<bool> {
        let (value, response_body) = self.call_with_body(
            Request::Write {
                session_id: id.as_str().to_string(),
            },
            data,
        )?;
        anyhow::ensure!(
            response_body.is_empty(),
            "supervisor write returned an unexpected body"
        );
        required_bool(&value, "open")
    }

    pub fn read(
        &self,
        id: &PtySessionId,
        timeout: Duration,
        max_bytes: usize,
    ) -> anyhow::Result<SupervisorReadOutcome> {
        let (value, data) = self.call_with_body(
            Request::Read {
                session_id: id.as_str().to_string(),
                timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
                max_bytes,
            },
            &[],
        )?;
        Ok(SupervisorReadOutcome {
            data,
            closed: required_bool(&value, "closed")?,
            dropped_bytes: required_u64(&value, "dropped_bytes")?,
        })
    }

    pub fn resize(&self, id: &PtySessionId, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.call(Request::Resize {
            session_id: id.as_str().to_string(),
            cols,
            rows,
        })?;
        Ok(())
    }

    pub fn claim(
        &self,
        id: &PtySessionId,
        attachment_id: &str,
        expected_epoch: u64,
    ) -> anyhow::Result<u64> {
        Ok(self
            .call(Request::Claim {
                session_id: id.as_str().to_string(),
                attachment_id: attachment_id.to_string(),
                expected_epoch,
            })?
            .get("epoch")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("session supervisor omitted attachment epoch"))?)
    }

    pub fn release(
        &self,
        id: &PtySessionId,
        attachment_id: &str,
        attached_epoch: u64,
    ) -> anyhow::Result<u64> {
        Ok(self
            .call(Request::Release {
                session_id: id.as_str().to_string(),
                attachment_id: attachment_id.to_string(),
                attached_epoch,
            })?
            .get("epoch")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("session supervisor omitted released epoch"))?)
    }

    pub fn attachment(&self, id: &PtySessionId) -> anyhow::Result<(u64, Option<String>)> {
        let value = self.call(Request::Attachment {
            session_id: id.as_str().to_string(),
        })?;
        let active_attachment_id = match value.get("active_attachment_id") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(value)) => Some(value.clone()),
            Some(_) => {
                anyhow::bail!("session supervisor active_attachment_id must be string or null")
            }
        };
        Ok((required_u64(&value, "epoch")?, active_attachment_id))
    }

    pub fn exit_status(&self, id: &PtySessionId) -> anyhow::Result<Option<u32>> {
        let value = self.call(Request::ExitStatus {
            session_id: id.as_str().to_string(),
        })?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::Number(number) => number
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("session supervisor exit status must fit in u32")),
            _ => anyhow::bail!("session supervisor exit status must be integer or null"),
        }
    }
}

#[cfg(unix)]
fn exchange_frame(
    stream: &mut std::os::unix::net::UnixStream,
    request: &Request,
    body: &[u8],
) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
    let header = serde_json::to_vec(request)?;
    write_wire_frame(stream, &header, body)?;
    let (header, body) = read_wire_frame(stream)?;
    let response: Response =
        serde_json::from_slice(&header).context("decode PTY session supervisor response")?;
    if !response.ok {
        return Err(SupervisorRejected(
            response
                .error
                .unwrap_or_else(|| "session supervisor rejected request".to_string()),
        )
        .into());
    }
    Ok((response.value.unwrap_or(serde_json::Value::Null), body))
}

fn write_wire_frame(writer: &mut impl Write, header: &[u8], body: &[u8]) -> anyhow::Result<()> {
    let header_len = u32::try_from(header.len()).context("supervisor header too large")?;
    let body_len = u32::try_from(body.len()).context("supervisor body too large")?;
    anyhow::ensure!(
        u64::from(header_len) + u64::from(body_len) <= MAX_REQUEST_BYTES,
        "supervisor frame exceeds {MAX_REQUEST_BYTES} bytes"
    );
    writer.write_all(&header_len.to_be_bytes())?;
    writer.write_all(&body_len.to_be_bytes())?;
    writer.write_all(header)?;
    writer.write_all(body)?;
    writer.flush()?;
    Ok(())
}

fn read_wire_frame(reader: &mut impl Read) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let mut lengths = [0_u8; 8];
    reader.read_exact(&mut lengths)?;
    let header_len = u32::from_be_bytes(lengths[..4].try_into().expect("four-byte header length"));
    let body_len = u32::from_be_bytes(lengths[4..].try_into().expect("four-byte body length"));
    anyhow::ensure!(
        u64::from(header_len) + u64::from(body_len) <= MAX_REQUEST_BYTES,
        "supervisor frame exceeds {MAX_REQUEST_BYTES} bytes"
    );
    let mut header = vec![0_u8; header_len as usize];
    let mut body = vec![0_u8; body_len as usize];
    reader.read_exact(&mut header)?;
    reader.read_exact(&mut body)?;
    Ok((header, body))
}

fn required_string<'a>(value: &'a serde_json::Value, field: &str) -> anyhow::Result<&'a str> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("session supervisor response omitted {field}"))
}

fn required_bool(value: &serde_json::Value, field: &str) -> anyhow::Result<bool> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("session supervisor response omitted boolean {field}"))
}

fn required_u64(value: &serde_json::Value, field: &str) -> anyhow::Result<u64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("session supervisor response omitted u64 {field}"))
}

fn supervisor_executable_name() -> &'static str {
    if cfg!(windows) {
        "easynet-session-supervisor.exe"
    } else {
        "easynet-session-supervisor"
    }
}

fn process_controller_id() -> &'static str {
    static CONTROLLER_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CONTROLLER_ID
        .get_or_init(|| uuid::Uuid::new_v4().to_string())
        .as_str()
}

pub fn requested_by_environment() -> bool {
    std::env::var_os("EASYNET_INTERNAL_SESSION_SUPERVISOR").is_some()
}

pub fn socket_path() -> PathBuf {
    supervisor_state_root().join(SOCKET_FILE)
}

fn journal_path() -> PathBuf {
    supervisor_state_root().join(JOURNAL_FILE)
}

fn supervisor_state_root() -> PathBuf {
    std::env::var_os("EASYNET_SESSION_SUPERVISOR_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::daemon::persistence::config::state_dir)
}

struct OutputState {
    bytes: VecDeque<u8>,
    closed: bool,
    dropped_bytes: u64,
}

struct AttachmentState {
    epoch: u64,
    active: Option<String>,
}

struct SupervisedSession {
    snapshot: PtySessionSnapshot,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
    output: Arc<(Mutex<OutputState>, Condvar)>,
    attachment: Mutex<AttachmentState>,
    dropped: Arc<AtomicBool>,
}

struct SupervisorState {
    sessions: Mutex<HashMap<PtySessionId, Arc<SupervisedSession>>>,
    session_count: AtomicUsize,
    controller_id: Mutex<Option<String>>,
    journal: PathBuf,
}

impl SupervisorState {
    fn new(journal: PathBuf) -> anyhow::Result<Self> {
        let state = Self {
            sessions: Mutex::new(HashMap::new()),
            session_count: AtomicUsize::new(0),
            controller_id: Mutex::new(None),
            journal,
        };
        // A prior journal with no live supervisor represents unrecoverable OS
        // handles. Reconcile it to the honest empty live set immediately.
        state.persist()?;
        Ok(state)
    }

    fn persist(&self) -> anyhow::Result<()> {
        let sessions = self
            .sessions
            .lock()
            .expect("supervisor sessions lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut rows = sessions
            .iter()
            .map(|session| {
                let attachment = session.attachment.lock().expect("attachment lock");
                serde_json::json!({
                    "session_id": session.snapshot.id.as_str(),
                    "created_unix_ms": session.snapshot.created_unix_ms,
                    "command": session.snapshot.command,
                    "command_args": session.snapshot.command_args,
                    "cwd": session.snapshot.cwd,
                    "epoch": attachment.epoch,
                    "active_attachment_id": attachment.active,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row["created_unix_ms"].as_u64().unwrap_or(0));
        let body = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "supervisor_pid": std::process::id(),
            "sessions": rows,
        }))?;
        crate::daemon::persistence::config::atomic_write_with_permissions(
            &self.journal,
            &body,
            crate::daemon::persistence::config::WritePermissions::OwnerReadWrite,
        )?;
        Ok(())
    }

    fn handle(
        &self,
        request: Request,
        request_body: Vec<u8>,
    ) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
        let request_body_allowed = matches!(&request, Request::Write { .. });
        anyhow::ensure!(
            request_body.is_empty() || request_body_allowed,
            "request body is not permitted for this operation"
        );
        anyhow::ensure!(
            !request_body_allowed || request_body.len() <= 256 * 1024,
            "PTY write body exceeds 262144 bytes"
        );
        let value = match request {
            Request::Ping => Ok(serde_json::json!({"pid": std::process::id()})),
            Request::Reconcile { controller_id } => self.reconcile(controller_id),
            Request::Create { spec } => self.create(spec),
            Request::List => self.list(),
            Request::Close { session_id } => self.close(PtySessionId::new(session_id)),
            Request::Exists { session_id } => Ok(serde_json::Value::Bool(
                self.sessions
                    .lock()
                    .expect("supervisor sessions lock")
                    .contains_key(&PtySessionId::new(session_id)),
            )),
            Request::Write { session_id } => {
                self.write(PtySessionId::new(session_id), &request_body)
            }
            Request::Read {
                session_id,
                timeout_ms,
                max_bytes,
            } => {
                anyhow::ensure!(request_body.is_empty(), "read request body must be empty");
                return self.read(
                    PtySessionId::new(session_id),
                    Duration::from_millis(timeout_ms.min(60_000)),
                    max_bytes.min(256 * 1024),
                );
            }
            Request::Resize {
                session_id,
                cols,
                rows,
            } => self.resize(PtySessionId::new(session_id), cols, rows),
            Request::Claim {
                session_id,
                attachment_id,
                expected_epoch,
            } => self.claim(PtySessionId::new(session_id), attachment_id, expected_epoch),
            Request::Release {
                session_id,
                attachment_id,
                attached_epoch,
            } => self.release(PtySessionId::new(session_id), attachment_id, attached_epoch),
            Request::Attachment { session_id } => self.attachment(PtySessionId::new(session_id)),
            Request::ExitStatus { session_id } => self.exit_status(PtySessionId::new(session_id)),
        }?;
        Ok((value, Vec::new()))
    }

    fn reconcile(&self, controller_id: String) -> anyhow::Result<serde_json::Value> {
        let changed = {
            let mut active_controller = self.controller_id.lock().expect("controller lock");
            if active_controller.as_deref() == Some(controller_id.as_str()) {
                false
            } else {
                *active_controller = Some(controller_id);
                true
            }
        };
        let mut released = 0_u64;
        if changed {
            let sessions = self.sessions.lock().expect("supervisor sessions lock");
            for session in sessions.values() {
                let mut attachment = session.attachment.lock().expect("attachment lock");
                if attachment.active.take().is_some() {
                    attachment.epoch = attachment.epoch.saturating_add(1);
                    released = released.saturating_add(1);
                }
            }
            drop(sessions);
            self.persist()?;
        }
        Ok(serde_json::json!({"released_attachments": released}))
    }

    fn session(&self, id: &PtySessionId) -> anyhow::Result<Arc<SupervisedSession>> {
        self.sessions
            .lock()
            .expect("supervisor sessions lock")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SESSION_NOT_FOUND: `{id}`"))
    }

    fn create(&self, spec: PtyCreateSpec) -> anyhow::Result<serde_json::Value> {
        self.session_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_SESSIONS).then_some(current + 1)
            })
            .map_err(|_| anyhow::anyhow!("SESSION_LIMIT: maximum {MAX_SESSIONS} PTYs reached"))?;
        match self.create_reserved(spec) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.session_count.fetch_sub(1, Ordering::AcqRel);
                Err(error)
            }
        }
    }

    fn create_reserved(&self, spec: PtyCreateSpec) -> anyhow::Result<serde_json::Value> {
        let pair = native_pty_system().openpty(PtySize {
            cols: spec.cols,
            rows: spec.rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let command = spec
            .command
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/sh".to_string());
        let mut builder = CommandBuilder::new(&command);
        for argument in &spec.command_args {
            builder.arg(argument);
        }
        if let Some(cwd) = &spec.cwd {
            builder.cwd(cwd);
        }
        for (name, value) in &spec.env {
            builder.env(name, value);
        }
        let child = pair.slave.spawn_command(builder)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let id = PtySessionId::new(uuid::Uuid::new_v4().to_string());
        let output = Arc::new((
            Mutex::new(OutputState {
                bytes: VecDeque::new(),
                closed: false,
                dropped_bytes: 0,
            }),
            Condvar::new(),
        ));
        let dropped = Arc::new(AtomicBool::new(false));
        spawn_reader(
            id.clone(),
            reader,
            Arc::clone(&output),
            Arc::clone(&dropped),
        );
        let session = Arc::new(SupervisedSession {
            snapshot: PtySessionSnapshot {
                id: id.clone(),
                created_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                command: Some(command),
                command_args: spec.command_args,
                cwd: spec.cwd,
            },
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(Some(child)),
            output,
            attachment: Mutex::new(AttachmentState {
                epoch: 0,
                active: None,
            }),
            dropped,
        });
        self.sessions
            .lock()
            .expect("supervisor sessions lock")
            .insert(id.clone(), session);
        if let Err(error) = self.persist() {
            if let Some(session) = self
                .sessions
                .lock()
                .expect("supervisor sessions lock")
                .remove(&id)
            {
                Self::terminate_session(&session);
            }
            return Err(error).context("persist newly created PTY session");
        }
        Ok(serde_json::json!({"session_id": id.as_str()}))
    }

    fn list(&self) -> anyhow::Result<serde_json::Value> {
        let sessions = self.sessions.lock().expect("supervisor sessions lock");
        let mut snapshots = sessions
            .values()
            .map(|session| session.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.created_unix_ms);
        Ok(serde_json::to_value(snapshots)?)
    }

    fn close(&self, id: PtySessionId) -> anyhow::Result<serde_json::Value> {
        let session = self
            .sessions
            .lock()
            .expect("supervisor sessions lock")
            .remove(&id);
        let Some(session) = session else {
            return Ok(serde_json::to_value(PtyCloseOutcome {
                ack: false,
                exit_status: None,
            })?);
        };
        self.session_count.fetch_sub(1, Ordering::AcqRel);
        let exit_status = Self::terminate_session(&session);
        self.persist()?;
        Ok(serde_json::to_value(PtyCloseOutcome {
            ack: true,
            exit_status,
        })?)
    }

    fn terminate_session(session: &SupervisedSession) -> Option<u32> {
        session.dropped.store(true, Ordering::Release);
        let mut child = session.child.lock().expect("supervisor child lock");
        if let Some(mut child) = child.take() {
            let _ = child.kill();
            child
                .try_wait()
                .ok()
                .flatten()
                .map(|status| status.exit_code())
        } else {
            None
        }
    }

    fn write(&self, id: PtySessionId, bytes: &[u8]) -> anyhow::Result<serde_json::Value> {
        let session = self.session(&id)?;
        let mut writer = session.writer.lock().expect("supervisor writer lock");
        let open = writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .is_ok();
        Ok(serde_json::json!({"open": open}))
    }

    fn read(
        &self,
        id: PtySessionId,
        timeout: Duration,
        max_bytes: usize,
    ) -> anyhow::Result<(serde_json::Value, Vec<u8>)> {
        let session = self.session(&id)?;
        let (lock, ready) = &*session.output;
        let mut output = lock.lock().expect("supervisor output lock");
        let deadline = std::time::Instant::now() + timeout;
        while output.bytes.is_empty() && !output.closed {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            let (next, result) = ready
                .wait_timeout(output, deadline - now)
                .expect("supervisor output wait");
            output = next;
            if result.timed_out() {
                break;
            }
        }
        let take = output.bytes.len().min(max_bytes);
        let data = output.bytes.drain(..take).collect::<Vec<_>>();
        let dropped_bytes = std::mem::take(&mut output.dropped_bytes);
        Ok((
            serde_json::json!({
                "closed": output.closed,
                "dropped_bytes": dropped_bytes,
            }),
            data,
        ))
    }

    fn resize(&self, id: PtySessionId, cols: u16, rows: u16) -> anyhow::Result<serde_json::Value> {
        if cols == 0 || rows == 0 {
            anyhow::bail!("INVALID_TERMINAL_SIZE: cols and rows must be positive");
        }
        self.session(&id)?
            .master
            .lock()
            .expect("supervisor master lock")
            .resize(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        Ok(serde_json::json!({}))
    }

    fn claim(
        &self,
        id: PtySessionId,
        attachment_id: String,
        expected_epoch: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let session = self.session(&id)?;
        let epoch = {
            let mut state = session.attachment.lock().expect("attachment lock");
            if state.epoch != expected_epoch {
                anyhow::bail!(
                    "ATTACHMENT_STALE: session `{id}` epoch is {}, caller expected {expected_epoch}",
                    state.epoch
                );
            }
            if let Some(active) = state.active.as_deref() {
                anyhow::bail!("SESSION_ALREADY_ATTACHED: session `{id}` is attached as `{active}`");
            }
            state.epoch = state.epoch.saturating_add(1);
            state.active = Some(attachment_id);
            state.epoch
        };
        self.persist()?;
        Ok(serde_json::json!({"epoch": epoch}))
    }

    fn release(
        &self,
        id: PtySessionId,
        attachment_id: String,
        attached_epoch: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let session = self.session(&id)?;
        let epoch = {
            let mut state = session.attachment.lock().expect("attachment lock");
            if state.epoch == attached_epoch
                && state.active.as_deref() == Some(attachment_id.as_str())
            {
                state.active = None;
                state.epoch = state.epoch.saturating_add(1);
            }
            state.epoch
        };
        self.persist()?;
        Ok(serde_json::json!({"epoch": epoch}))
    }

    fn attachment(&self, id: PtySessionId) -> anyhow::Result<serde_json::Value> {
        let session = self.session(&id)?;
        let state = session.attachment.lock().expect("attachment lock");
        Ok(serde_json::json!({
            "epoch": state.epoch,
            "active_attachment_id": state.active,
        }))
    }

    fn exit_status(&self, id: PtySessionId) -> anyhow::Result<serde_json::Value> {
        let session = self.session(&id)?;
        let mut child = session.child.lock().expect("supervisor child lock");
        let status = match child.as_mut() {
            Some(child) => child.try_wait()?.map(|status| status.exit_code()),
            None => None,
        };
        Ok(serde_json::to_value(status)?)
    }
}

fn spawn_reader(
    id: PtySessionId,
    mut reader: Box<dyn Read + Send>,
    output: Arc<(Mutex<OutputState>, Condvar)>,
    dropped: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name(format!("easynet-pty-{id}"))
        .spawn(move || {
            let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
            loop {
                if dropped.load(Ordering::Acquire) {
                    return;
                }
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => {
                        let (lock, ready) = &*output;
                        let mut state = lock.lock().expect("supervisor output lock");
                        state.closed = true;
                        ready.notify_all();
                        return;
                    }
                    Ok(read) => {
                        let (lock, ready) = &*output;
                        let mut state = lock.lock().expect("supervisor output lock");
                        state.bytes.extend(&buffer[..read]);
                        if state.bytes.len() > OUTPUT_BUFFER_CAP_BYTES {
                            let excess = state.bytes.len() - OUTPUT_BUFFER_CAP_BYTES;
                            state.bytes.drain(..excess);
                            state.dropped_bytes = state.dropped_bytes.saturating_add(excess as u64);
                        }
                        ready.notify_all();
                    }
                }
            }
        });
}

#[cfg(unix)]
pub fn run() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let socket = socket_path();
    let parent = socket
        .parent()
        .ok_or_else(|| anyhow::anyhow!("session supervisor socket has no parent"))?;
    std::fs::create_dir_all(parent)?;
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            return Ok(());
        }
        std::fs::remove_file(&socket).context("remove stale session supervisor socket")?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let state = Arc::new(SupervisorState::new(journal_path())?);
    let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(ACCEPT_QUEUE_BOUND);
    let accepted_rx = Arc::new(Mutex::new(accepted_rx));
    for worker in 0..SERVER_WORKERS {
        let state = Arc::clone(&state);
        let accepted_rx = Arc::clone(&accepted_rx);
        std::thread::Builder::new()
            .name(format!("easynet-pty-supervisor-{worker}"))
            .spawn(move || loop {
                let stream = accepted_rx
                    .lock()
                    .expect("supervisor accept queue lock")
                    .recv();
                match stream {
                    Ok(stream) => handle_connection(stream, Arc::clone(&state)),
                    Err(_) => break,
                }
            })?;
    }
    for accepted in listener.incoming() {
        match accepted {
            Ok(stream) => accepted_tx
                .send(stream)
                .map_err(|_| anyhow::anyhow!("PTY supervisor worker queue closed"))?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn handle_connection(mut stream: std::os::unix::net::UnixStream, state: Arc<SupervisorState>) {
    if stream
        .set_read_timeout(Some(Duration::from_secs(70)))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(10))))
        .is_err()
    {
        return;
    }
    while let Ok((request_header, request_body)) = read_wire_frame(&mut stream) {
        let handled = serde_json::from_slice::<Request>(&request_header)
            .context("decode supervisor request")
            .and_then(|request| state.handle(request, request_body));
        let (response, response_body) = match handled {
            Ok((value, body)) => (Response::value(value), body),
            Err(error) => (Response::error(error.to_string()), Vec::new()),
        };
        let Ok(response_header) = serde_json::to_vec(&response) else {
            break;
        };
        if write_wire_frame(&mut stream, &response_header, &response_body).is_err() {
            break;
        }
    }
}

#[cfg(not(unix))]
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("PTY session supervisor transport is unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_binary_has_stable_product_name() {
        assert!(supervisor_executable_name().starts_with("easynet-session-supervisor"));
    }

    #[test]
    fn wire_frame_preserves_every_byte_without_text_encoding() {
        let header = br#"{"op":"write","session_id":"test"}"#;
        let body = (0_u8..=u8::MAX).collect::<Vec<_>>();
        let mut wire = std::io::Cursor::new(Vec::new());
        write_wire_frame(&mut wire, header, &body).expect("encode wire frame");
        assert_eq!(wire.get_ref().len(), 8 + header.len() + body.len());

        wire.set_position(0);
        let (decoded_header, decoded_body) = read_wire_frame(&mut wire).expect("decode wire frame");
        assert_eq!(decoded_header, header);
        assert_eq!(decoded_body, body);
    }

    #[test]
    fn aggregate_output_memory_has_a_hard_product_bound() {
        assert_eq!(MAX_SESSIONS * OUTPUT_BUFFER_CAP_BYTES, 128 * 1024 * 1024);
    }
}
