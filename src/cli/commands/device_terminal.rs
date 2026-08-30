// EasyNet CLI — invocation-backed device terminal
// =================================================
//
// File: src/cli/commands/device_terminal.rs
// Description: Interactive PTY client over canonical terminal.* Invocation.
//
// Protocol Responsibility
// -----------------------
// Own only the product interaction loop. PTY lifecycle remains daemon-owned,
// authority signing remains key-service-owned, and admission remains the sole
// verifier of session authority.
//
// Implementation Approach
// -----------------------
// A TerminalSession is an explicit lifecycle object: open -> active -> closed.
// Every follow-up invocation carries one renewable canonical session-authority
// lease bound to the exact session, owner, callee, subject, and ability set.
//
// Usage Contract
// --------------
// Call `run(target)` with `local`, the current device identity, or a canonical
// remote Device URA. The function restores terminal mode and attempts
// idempotent remote close on every exit path.
//
// Architectural Position
// ----------------------
// CLI facade only. It does not access PTY state or backend APIs directly.

use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "axon-pb")]
use std::sync::Arc;

use anyhow::Context;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
#[cfg(feature = "axon-pb")]
use futures::StreamExt;
use serde_json::{json, Value};

use crate::cli::daemon_client::remote_system_ability::RemoteDeviceSessionAbility;
#[cfg(feature = "axon-pb")]
use crate::cli::daemon_client::remote_system_ability::{
    invoke_remote_device_session_ability, invoke_remote_device_system_ability_as_caller,
    open_remote_terminal_attach, RemoteTargetSystemAbility,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::admission::authority_metadata::CanonicalSessionAuthorityIssuer;
use crate::daemon::invocation::admission::authority_metadata::{
    IssuedAuthorityMetadata, SessionAuthorityRequest,
};
use crate::daemon::invocation::routing::target::LocalAbilityTarget;
use crate::support::platform::local_invoke::{
    LocalDaemonSystemAbilityIssuer, LocalRuntimeAuthorityIssuer,
};

const AUTHORITY_LEASE_MILLIS: i64 = 5 * 60 * 1_000;
const AUTHORITY_RENEWAL_MARGIN_MILLIS: i64 = 30 * 1_000;

fn authority_renewal_deadline(expires_at_ms: i64) -> i64 {
    expires_at_ms.saturating_sub(AUTHORITY_RENEWAL_MARGIN_MILLIS)
}
const TERMINAL_INVOKE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(feature = "axon-pb")]
const TERMINAL_BIDI_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const TERMINAL_INPUT_BATCH_BYTES: usize = 16 * 1024;
/// Coalesce human key events without adding perceptible interactive latency.
const TERMINAL_INPUT_BATCH_DELAY: Duration = Duration::from_millis(2);
const TERMINAL_OUTPUT_FLUSH_BYTES: usize = 16 * 1024;
const TERMINAL_OUTPUT_FLUSH_DELAY: Duration = Duration::from_millis(2);
const TERMINAL_SESSION_ALLOWED_ACTIONS: [&str; 2] = ["stream", "manage"];

pub(crate) fn run(raw_target: &str) -> anyhow::Result<()> {
    let route = TerminalInvocationRoute::resolve(raw_target)?;
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut session = TerminalSession::open(route, cols.max(1), rows.max(1))?;
    match session.run_interactive() {
        Ok(TerminalInteractiveOutcome::Exited) => session.close(),
        Ok(TerminalInteractiveOutcome::Detached) => {
            eprintln!(
                "\r\nDetached terminal session {} at epoch {}.",
                session.session_id, session.epoch
            );
            Ok(())
        }
        Err(error) => Err(error.context(format!(
            "terminal session {} remains available for reattach",
            session.session_id
        ))),
    }
}

pub(crate) fn run_existing(raw_target: &str, session_id: &str) -> anyhow::Result<()> {
    let route = TerminalInvocationRoute::resolve(raw_target)?;
    let epoch = route.session_epoch(session_id)?;
    let mut session = TerminalSession::resume(route, session_id, epoch)?;
    match session.run_interactive()? {
        TerminalInteractiveOutcome::Exited => session.close(),
        TerminalInteractiveOutcome::Detached => {
            eprintln!(
                "\r\nDetached terminal session {} at epoch {}.",
                session.session_id, session.epoch
            );
            Ok(())
        }
    }
}

pub(crate) fn list(raw_target: &str) -> anyhow::Result<()> {
    let route = TerminalInvocationRoute::resolve(raw_target)?;
    let response = route.list()?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

pub(crate) fn close_existing(raw_target: &str, session_id: &str) -> anyhow::Result<()> {
    let route = TerminalInvocationRoute::resolve(raw_target)?;
    let epoch = route.session_epoch(session_id)?;
    TerminalSession::resume(route, session_id, epoch)?.close()
}

enum TerminalInvocationRoute {
    Local {
        target_ura: String,
    },
    #[cfg(feature = "axon-pb")]
    Remote {
        target_ura: String,
        caller_ura: String,
        signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
    },
}

impl TerminalInvocationRoute {
    fn resolve(raw_target: &str) -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()
            .context("device terminal requires paired runtime credentials")?;
        let identity = crate::support::platform::remote_device::PairedInvocationIdentity::load(
            "device terminal",
        )?;
        let target = raw_target.trim();
        let local_device_ura = identity.local_device_ura();
        if target.is_empty()
            || target.eq_ignore_ascii_case("local")
            || target == credentials.node_id
            || target == local_device_ura
        {
            // Prove the authority signer is available before terminal.create
            // allocates a live PTY row.
            crate::daemon::identity::local_invocation::system_verifying_key()
                .context("prepare local runtime authority signer")?;
            return Ok(Self::Local {
                target_ura: crate::daemon::identity::local_invocation::local_daemon_ura()?,
            });
        }

        #[cfg(feature = "axon-pb")]
        {
            let target_ura =
                crate::support::platform::remote_device::resolve_target_device_ura(target)?;
            let caller_ura = identity.caller_user_ura().to_string();
            let signer =
                crate::daemon::invocation::routing::remote_invoke::load_remote_invocation_caller_signer(
                    &caller_ura,
                )
                .context("prepare remote terminal caller signer")?;
            Ok(Self::Remote {
                target_ura,
                caller_ura,
                signer,
            })
        }

        #[cfg(not(feature = "axon-pb"))]
        {
            let _ = identity;
            Err(
                crate::support::platform::local_invoke::federation_capability_unsupported_error(
                    "opening a remote device terminal",
                ),
            )
        }
    }

    fn target_ura(&self) -> &str {
        match self {
            Self::Local { target_ura } => target_ura,
            #[cfg(feature = "axon-pb")]
            Self::Remote { target_ura, .. } => target_ura,
        }
    }

    fn issuer_ura(&self) -> &str {
        match self {
            Self::Local { .. } => crate::core::ura::LOCAL_SYSTEM_AGENT_URA,
            #[cfg(feature = "axon-pb")]
            Self::Remote { caller_ura, .. } => caller_ura,
        }
    }

    fn session_callee_ura(&self) -> anyhow::Result<String> {
        Ok(LocalAbilityTarget::for_device_sponsored_system_ability(
            "terminal.create",
            self.target_ura(),
        )?
        .callee_ura()
        .to_string())
    }

    fn create(&self, cols: u16, rows: u16) -> anyhow::Result<Value> {
        let args = json!({"cols": cols, "rows": rows});
        match self {
            Self::Local { target_ura } => {
                let target = LocalAbilityTarget::for_device_sponsored_system_ability(
                    "terminal.create",
                    target_ura,
                )?;
                LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(
                    &target,
                    args,
                    target_ura,
                    TERMINAL_INVOKE_TIMEOUT,
                )
            }
            #[cfg(feature = "axon-pb")]
            Self::Remote {
                target_ura,
                caller_ura,
                ..
            } => invoke_remote_device_system_ability_as_caller(
                target_ura,
                RemoteTargetSystemAbility::TerminalCreate,
                args,
                caller_ura,
            ),
        }
    }

    fn list(&self) -> anyhow::Result<Value> {
        match self {
            Self::Local { target_ura } => {
                let target = LocalAbilityTarget::for_device_sponsored_system_ability(
                    "terminal.list",
                    target_ura,
                )?;
                LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(
                    &target,
                    json!({}),
                    target_ura,
                    TERMINAL_INVOKE_TIMEOUT,
                )
            }
            #[cfg(feature = "axon-pb")]
            Self::Remote {
                target_ura,
                caller_ura,
                ..
            } => invoke_remote_device_system_ability_as_caller(
                target_ura,
                RemoteTargetSystemAbility::TerminalList,
                json!({}),
                caller_ura,
            ),
        }
    }

    fn session_epoch(&self, session_id: &str) -> anyhow::Result<u64> {
        let response = self.list()?;
        response
            .get("sessions")
            .and_then(Value::as_array)
            .and_then(|sessions| {
                sessions.iter().find(|session| {
                    session.get("session_id").and_then(Value::as_str) == Some(session_id)
                })
            })
            .and_then(|session| session.get("epoch"))
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("SESSION_NOT_FOUND: terminal session `{session_id}`"))
    }

    fn issue_authority(
        &self,
        request: SessionAuthorityRequest,
    ) -> anyhow::Result<IssuedAuthorityMetadata> {
        match self {
            Self::Local { .. } => LocalRuntimeAuthorityIssuer::issue_session_authority(request),
            #[cfg(feature = "axon-pb")]
            Self::Remote { signer, .. } => {
                let prepared =
                    CanonicalSessionAuthorityIssuer::prepare(request, signer.owner_ura())?;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("build runtime for remote session-authority signing")?;
                let signature = runtime
                    .block_on(signer.sign_canonical(prepared.canonical_payload()))
                    .context("sign remote terminal session authority")?;
                prepared
                    .seal(signature.to_bytes().to_vec())
                    .map_err(anyhow::Error::new)
            }
        }
    }

    fn invoke_followup(
        &self,
        ability: RemoteDeviceSessionAbility,
        subject_ura: &str,
        args: Value,
        authority: IssuedAuthorityMetadata,
    ) -> anyhow::Result<Value> {
        match self {
            Self::Local { target_ura } => {
                let target = LocalAbilityTarget::for_device_sponsored_system_ability(
                    ability.as_str(),
                    target_ura,
                )?;
                LocalDaemonSystemAbilityIssuer::invoke_target_root_with_authority_timeout(
                    &target,
                    args,
                    subject_ura,
                    authority,
                    TERMINAL_INVOKE_TIMEOUT,
                )
            }
            #[cfg(feature = "axon-pb")]
            Self::Remote {
                target_ura,
                caller_ura,
                signer,
            } => invoke_remote_device_session_ability(
                target_ura,
                caller_ura,
                subject_ura,
                ability,
                args,
                authority,
                Arc::clone(signer),
            ),
        }
    }

    #[cfg(feature = "axon-pb")]
    async fn attach(
        &self,
        session_id: &str,
        attachment_id: &str,
        expected_epoch: u64,
        subject_ura: &str,
        authority: IssuedAuthorityMetadata,
    ) -> anyhow::Result<crate::support::platform::bidi_session::DaemonBidiSession> {
        match self {
            Self::Local { target_ura } => {
                let target = LocalAbilityTarget::for_device_sponsored_system_ability(
                    "terminal.attach",
                    target_ura,
                )?;
                LocalDaemonSystemAbilityIssuer::open_target_bidi_with_authority(
                    &target,
                    json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "expected_epoch": expected_epoch,
                    }),
                    subject_ura,
                    authority,
                    TERMINAL_BIDI_TIMEOUT,
                )
                .await
            }
            Self::Remote {
                target_ura,
                caller_ura,
                signer,
            } => {
                open_remote_terminal_attach(
                    target_ura,
                    caller_ura,
                    subject_ura,
                    session_id,
                    attachment_id,
                    expected_epoch,
                    authority,
                    Arc::clone(signer),
                    TERMINAL_BIDI_TIMEOUT,
                )
                .await
            }
        }
    }
}

struct TerminalAuthorityLease {
    metadata: IssuedAuthorityMetadata,
    expires_at_ms: i64,
}

struct TerminalSession {
    route: TerminalInvocationRoute,
    session_id: String,
    session_owner_user_id: String,
    subject_ura: String,
    authority: TerminalAuthorityLease,
    epoch: u64,
    closed: bool,
}

impl TerminalSession {
    fn open(route: TerminalInvocationRoute, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()
            .context("load terminal session owner credentials")?;
        let session_owner_user_id = credentials.user_id()?.to_string();
        let response = route.create(cols, rows).context("invoke terminal.create")?;
        let session_id = response
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("terminal.create returned no session_id"))?
            .to_string();
        let epoch = response.get("epoch").and_then(Value::as_u64).unwrap_or(0);
        let subject_ura = crate::core::ura::resource_dot_ura(
            credentials.realm_str(),
            &format!("user.{session_owner_user_id}"),
            &format!("session/{session_id}"),
        );
        let authority =
            Self::issue_authority_lease(&route, &session_id, &session_owner_user_id, &subject_ura)
                .context("issue terminal session authority")?;
        Ok(Self {
            route,
            session_id,
            session_owner_user_id,
            subject_ura,
            authority,
            epoch,
            closed: false,
        })
    }

    fn issue_authority_lease(
        route: &TerminalInvocationRoute,
        session_id: &str,
        session_owner_user_id: &str,
        subject_ura: &str,
    ) -> anyhow::Result<TerminalAuthorityLease> {
        let now_ms = unix_epoch_millis()?;
        let expires_at_ms = now_ms
            .checked_add(AUTHORITY_LEASE_MILLIS)
            .ok_or_else(|| anyhow::anyhow!("terminal authority expiry overflow"))?;
        let session_callee_ura = route.session_callee_ura()?;
        let request = SessionAuthorityRequest {
            issuer_ura: route.issuer_ura().to_string(),
            session_id: session_id.to_string(),
            session_owner_user_id: session_owner_user_id.to_string(),
            creator_principal_id: route.issuer_ura().to_string(),
            callee_ura: session_callee_ura.clone(),
            subject_ura: subject_ura.to_string(),
            audience: session_callee_ura,
            scopes: vec!["terminal.*".to_string()],
            // Descriptor admission remains authoritative: input/read/resize
            // are `stream`, while terminal close is `manage`.
            allowed_actions: TERMINAL_SESSION_ALLOWED_ACTIONS
                .into_iter()
                .map(str::to_string)
                .collect(),
            allowed_followup_abilities: vec![
                "terminal.attach".to_string(),
                "terminal.close".to_string(),
            ],
            issued_at_ms: now_ms.saturating_sub(1_000),
            expires_at_ms,
        };
        Ok(TerminalAuthorityLease {
            metadata: route.issue_authority(request)?,
            expires_at_ms,
        })
    }

    fn resume(
        route: TerminalInvocationRoute,
        session_id: &str,
        epoch: u64,
    ) -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()
            .context("load terminal session owner credentials")?;
        let session_owner_user_id = credentials.user_id()?.to_string();
        let subject_ura = crate::core::ura::resource_dot_ura(
            credentials.realm_str(),
            &format!("user.{session_owner_user_id}"),
            &format!("session/{session_id}"),
        );
        let authority =
            Self::issue_authority_lease(&route, session_id, &session_owner_user_id, &subject_ura)?;
        Ok(Self {
            route,
            session_id: session_id.to_string(),
            session_owner_user_id,
            subject_ura,
            authority,
            epoch,
            closed: false,
        })
    }

    fn ensure_authority(&mut self) -> anyhow::Result<()> {
        let renewal_at = authority_renewal_deadline(self.authority.expires_at_ms);
        if unix_epoch_millis()? < renewal_at {
            return Ok(());
        }
        self.authority = Self::issue_authority_lease(
            &self.route,
            &self.session_id,
            &self.session_owner_user_id,
            &self.subject_ura,
        )?;
        Ok(())
    }

    fn invoke(
        &mut self,
        ability: RemoteDeviceSessionAbility,
        args: Value,
    ) -> anyhow::Result<Value> {
        self.ensure_authority()?;
        self.route.invoke_followup(
            ability,
            &self.subject_ura,
            args,
            self.authority.metadata.clone(),
        )
    }

    fn run_interactive(&mut self) -> anyhow::Result<TerminalInteractiveOutcome> {
        #[cfg(not(feature = "axon-pb"))]
        {
            return Err(
                crate::support::platform::local_invoke::local_invocation_capability_unsupported_error(
                    "opening an interactive terminal InvokeBidi session",
                ),
            );
        }

        #[cfg(feature = "axon-pb")]
        {
            self.ensure_authority()?;
            let authority = self.authority.metadata.clone();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build runtime for terminal InvokeBidi session")?;
            runtime.block_on(self.run_interactive_bidi(authority))
        }
    }

    #[cfg(feature = "axon-pb")]
    async fn run_interactive_bidi(
        &mut self,
        authority: IssuedAuthorityMetadata,
    ) -> anyhow::Result<TerminalInteractiveOutcome> {
        let attachment_id = uuid::Uuid::new_v4().to_string();
        let session = self
            .route
            .attach(
                &self.session_id,
                &attachment_id,
                self.epoch,
                &self.subject_ura,
                authority,
            )
            .await
            .context("invoke terminal.attach")?;
        let _raw_mode = RawTerminalMode::enter()?;
        let (mut upstream, mut downstream) = session.split();
        let mut stdout = io::stdout().lock();
        let mut input_events = TerminalEventPump::spawn();
        let mut input_closed = false;
        let mut pending_input = Vec::with_capacity(TERMINAL_INPUT_BATCH_BYTES);
        let mut input_flush_deadline: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        let mut pending_output_bytes = 0_usize;
        let mut output_flush_deadline: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        loop {
            tokio::select! {
                frame = downstream.recv() => {
                    let Some(frame) = frame? else {
                        if pending_output_bytes > 0 {
                            stdout.flush()?;
                        }
                        anyhow::bail!("terminal.attach transport ended without exit or detached frame");
                    };
                    if let Some(binary) = frame.binary {
                        stdout.write_all(&binary.data)?;
                        let was_clean = pending_output_bytes == 0;
                        pending_output_bytes = pending_output_bytes.saturating_add(binary.data.len());
                        if pending_output_bytes >= TERMINAL_OUTPUT_FLUSH_BYTES {
                            stdout.flush()?;
                            pending_output_bytes = 0;
                            output_flush_deadline = None;
                        } else if was_clean {
                            output_flush_deadline = Some(Box::pin(tokio::time::sleep(TERMINAL_OUTPUT_FLUSH_DELAY)));
                        }
                        continue;
                    }
                    if pending_output_bytes > 0 {
                        stdout.flush()?;
                        pending_output_bytes = 0;
                        output_flush_deadline = None;
                    }
                    match frame.payload.get("type").and_then(Value::as_str) {
                        Some("attached") => {
                            self.epoch = frame.payload.get("epoch").and_then(Value::as_u64)
                                .ok_or_else(|| anyhow::anyhow!("terminal.attach attached frame omitted epoch"))?;
                        }
                        Some("exit") => return Ok(TerminalInteractiveOutcome::Exited),
                        Some("detached") => {
                            self.epoch = frame.payload.get("epoch").and_then(Value::as_u64)
                                .ok_or_else(|| anyhow::anyhow!("terminal.attach detached frame omitted epoch"))?;
                            return Ok(TerminalInteractiveOutcome::Detached);
                        }
                        Some("output_gap") => {
                            let dropped_bytes = frame.payload.get("dropped_bytes").and_then(Value::as_u64)
                                .unwrap_or_default();
                            anyhow::bail!(
                                "OUTPUT_GAP: terminal session dropped {dropped_bytes} buffered output bytes"
                            );
                        }
                        Some("error") => {
                            let message = frame.payload.get("message").and_then(Value::as_str)
                                .unwrap_or("terminal.attach rejected the client frame");
                            anyhow::bail!("{message}");
                        }
                        Some("receipt") if frame.terminal => {
                            let failure = frame.payload.pointer("/failure/message")
                                .and_then(Value::as_str)
                                .unwrap_or("terminal.attach ended without an exit frame");
                            anyhow::bail!("{failure}");
                        }
                        Some("receipt" | "control") | None => {}
                        Some(other) => anyhow::bail!("terminal.attach returned unsupported frame type `{other}`"),
                    }
                }
                event = input_events.recv(), if !input_closed => {
                    match event.transpose().context("read local terminal event")? {
                        Some(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                            if is_terminal_detach_key(key) {
                                flush_terminal_input(&mut upstream, &mut pending_input).await?;
                                input_flush_deadline = None;
                                upstream.send_pty_control_json(&json!({"type": "detach"})).await?;
                            } else if let Some(bytes) = key_event_bytes(key) {
                                let was_empty = pending_input.is_empty();
                                buffer_terminal_input(&mut upstream, &mut pending_input, &bytes).await?;
                                if pending_input.is_empty() {
                                    input_flush_deadline = None;
                                } else if was_empty {
                                    input_flush_deadline = Some(Box::pin(tokio::time::sleep(TERMINAL_INPUT_BATCH_DELAY)));
                                }
                            }
                        }
                        Some(Event::Paste(text)) => {
                            let was_empty = pending_input.is_empty();
                            buffer_terminal_input(&mut upstream, &mut pending_input, text.as_bytes()).await?;
                            if pending_input.is_empty() {
                                input_flush_deadline = None;
                            } else if was_empty {
                                input_flush_deadline = Some(Box::pin(tokio::time::sleep(TERMINAL_INPUT_BATCH_DELAY)));
                            }
                        }
                        Some(Event::Resize(cols, rows)) => {
                            flush_terminal_input(&mut upstream, &mut pending_input).await?;
                            input_flush_deadline = None;
                            upstream.send_pty_resize(u32::from(cols.max(1)), u32::from(rows.max(1))).await?;
                        }
                        Some(_) => {}
                        None => {
                            flush_terminal_input(&mut upstream, &mut pending_input).await?;
                            input_flush_deadline = None;
                            upstream.send_pty_control_json(&json!({"type": "close_input"})).await?;
                            upstream.send_eof().await?;
                            input_closed = true;
                        }
                    }
                }
                _ = async {
                    if let Some(deadline) = input_flush_deadline.as_mut() {
                        deadline.as_mut().await;
                    }
                }, if input_flush_deadline.is_some() => {
                    flush_terminal_input(&mut upstream, &mut pending_input).await?;
                    input_flush_deadline = None;
                }
                _ = async {
                    if let Some(deadline) = output_flush_deadline.as_mut() {
                        deadline.as_mut().await;
                    }
                }, if output_flush_deadline.is_some() => {
                    stdout.flush()?;
                    pending_output_bytes = 0;
                    output_flush_deadline = None;
                }
            }
        }
    }

    fn close(&mut self) -> anyhow::Result<()> {
        if self.closed {
            return Ok(());
        }
        let response = self.invoke(
            RemoteDeviceSessionAbility::Close,
            json!({"session_id": self.session_id}),
        )?;
        self.closed = true;
        if response.get("ack").and_then(Value::as_bool) == Some(false) {
            // Close is explicitly idempotent. An already-dead session is a
            // completed terminal state, not a second failure path.
            return Ok(());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalInteractiveOutcome {
    Exited,
    Detached,
}

fn is_terminal_detach_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char(']') && key.modifiers == KeyModifiers::CONTROL
}

#[cfg(feature = "axon-pb")]
async fn buffer_terminal_input(
    upstream: &mut crate::support::platform::bidi_session::DaemonBidiSender,
    pending: &mut Vec<u8>,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let available = TERMINAL_INPUT_BATCH_BYTES.saturating_sub(pending.len());
        if available == 0 {
            flush_terminal_input(upstream, pending).await?;
            continue;
        }
        let take = available.min(remaining.len());
        pending.extend_from_slice(&remaining[..take]);
        remaining = &remaining[take..];
        if pending.len() == TERMINAL_INPUT_BATCH_BYTES {
            flush_terminal_input(upstream, pending).await?;
        }
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
async fn flush_terminal_input(
    upstream: &mut crate::support::platform::bidi_session::DaemonBidiSender,
    pending: &mut Vec<u8>,
) -> anyhow::Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let bytes = std::mem::take(pending);
    upstream.send_binary(bytes).await
}

struct RawTerminalMode;

impl RawTerminalMode {
    fn enter() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode().context("enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for RawTerminalMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(feature = "axon-pb")]
struct TerminalEventPump {
    receiver: tokio::sync::mpsc::Receiver<std::io::Result<Event>>,
    task: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "axon-pb")]
impl TerminalEventPump {
    fn spawn() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        let task = tokio::spawn(async move {
            let mut events = EventStream::new();
            while let Some(event) = events.next().await {
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Self { receiver, task }
    }

    async fn recv(&mut self) -> Option<std::io::Result<Event>> {
        self.receiver.recv().await
    }
}

#[cfg(feature = "axon-pb")]
impl Drop for TerminalEventPump {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn key_event_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = match key.code {
        KeyCode::Char(character) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let upper = character.to_ascii_uppercase();
            if upper.is_ascii() {
                vec![(upper as u8) & 0x1f]
            } else {
                return None;
            }
        }
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

fn unix_epoch_millis() -> anyhow::Result<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_millis();
    i64::try_from(millis).context("system clock exceeds i64 milliseconds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_projection_preserves_terminal_control_sequences() {
        assert_eq!(
            key_event_bytes(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
        assert_eq!(
            key_event_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_event_bytes(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            Some(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn authority_lease_is_short_lived_and_renewed_before_expiry() {
        let issued_at_ms = unix_epoch_millis().expect("clock");
        let expires_at_ms = issued_at_ms + AUTHORITY_LEASE_MILLIS;
        let renewal_at_ms = authority_renewal_deadline(expires_at_ms);
        assert!(renewal_at_ms > issued_at_ms);
        assert!(renewal_at_ms < expires_at_ms);
    }

    #[test]
    fn authority_actions_cover_followup_descriptor_contracts() {
        assert_eq!(TERMINAL_SESSION_ALLOWED_ACTIONS, ["stream", "manage"]);
        assert!(
            !TERMINAL_SESSION_ALLOWED_ACTIONS.contains(&"invoke"),
            "terminal authority must use descriptor actions, not a generic invocation verb"
        );
    }

    #[test]
    fn terminal_session_authority_targets_system_agent_not_device_host() {
        let route = TerminalInvocationRoute::Local {
            target_ura: crate::core::ura::device_ura("hub", "node-a"),
        };

        assert_eq!(
            route.session_callee_ura().expect("terminal callee"),
            crate::core::ura::device_agent_ura("hub", "node-a", "terminal")
        );
    }
}
