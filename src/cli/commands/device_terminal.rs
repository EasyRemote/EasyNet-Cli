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
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::{json, Value};

use crate::cli::daemon_client::remote_system_ability::RemoteDeviceSessionAbility;
#[cfg(feature = "axon-pb")]
use crate::cli::daemon_client::remote_system_ability::{
    invoke_remote_device_session_ability, invoke_remote_device_system_ability_as_caller,
    RemoteDeviceSystemAbility,
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
const TERMINAL_READ_TIMEOUT_SECONDS: f64 = 0.10;
const TERMINAL_INPUT_BATCH_BYTES: usize = 16 * 1024;
const TERMINAL_SESSION_ALLOWED_ACTIONS: [&str; 2] = ["stream", "manage"];

pub(crate) fn run(raw_target: &str) -> anyhow::Result<()> {
    let route = TerminalInvocationRoute::resolve(raw_target)?;
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut session = TerminalSession::open(route, cols.max(1), rows.max(1))?;
    let run_result = session.run_interactive();
    let close_result = session.close();
    match (run_result, close_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(run_error), Ok(())) => Err(run_error),
        (Ok(()), Err(close_error)) => Err(close_error.context("close terminal session")),
        (Err(run_error), Err(close_error)) => Err(run_error.context(format!(
            "terminal session also failed to close: {close_error:#}"
        ))),
    }
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
        let target = raw_target.trim();
        let local_device_ura =
            crate::core::ura::device_ura(credentials.realm_str(), &credentials.node_id);
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
            let caller_ura = local_device_ura;
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
            let _ = local_device_ura;
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

    fn create(&self, cols: u16, rows: u16) -> anyhow::Result<Value> {
        let args = json!({"cols": cols, "rows": rows});
        match self {
            Self::Local { target_ura } => {
                let target = LocalAbilityTarget::new("terminal.create", target_ura)?;
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
                RemoteDeviceSystemAbility::TerminalCreate,
                args,
                caller_ura,
            ),
        }
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
                let target = LocalAbilityTarget::new(ability.as_str(), target_ura)?;
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
        let request = SessionAuthorityRequest {
            issuer_ura: route.issuer_ura().to_string(),
            session_id: session_id.to_string(),
            session_owner_user_id: session_owner_user_id.to_string(),
            creator_principal_id: route.issuer_ura().to_string(),
            callee_ura: route.target_ura().to_string(),
            subject_ura: subject_ura.to_string(),
            audience: route.target_ura().to_string(),
            scopes: vec!["terminal.*".to_string()],
            // Descriptor admission remains authoritative: input/read/resize
            // are `stream`, while terminal close is `manage`.
            allowed_actions: TERMINAL_SESSION_ALLOWED_ACTIONS
                .into_iter()
                .map(str::to_string)
                .collect(),
            allowed_followup_abilities: vec![
                "terminal.input".to_string(),
                "terminal.read".to_string(),
                "terminal.resize".to_string(),
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

    fn run_interactive(&mut self) -> anyhow::Result<()> {
        let _raw_mode = RawTerminalMode::enter()?;
        let mut stdout = io::stdout().lock();
        loop {
            let response = self.invoke(
                RemoteDeviceSessionAbility::Read,
                json!({
                    "session_id": self.session_id,
                    "timeout": TERMINAL_READ_TIMEOUT_SECONDS,
                }),
            )?;
            if let Some(output) = response.get("output").and_then(Value::as_str) {
                if !output.is_empty() {
                    let bytes = BASE64_STANDARD
                        .decode(output)
                        .context("terminal.read returned invalid base64 output")?;
                    stdout.write_all(&bytes)?;
                    stdout.flush()?;
                }
            }
            if response.get("code").and_then(Value::as_str) == Some("session_dead") {
                return Ok(());
            }

            let mut pending_input = Vec::with_capacity(TERMINAL_INPUT_BATCH_BYTES);
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if let Some(bytes) = key_event_bytes(key) {
                            self.buffer_input(&mut pending_input, &bytes)?;
                        }
                    }
                    Event::Paste(text) => {
                        self.buffer_input(&mut pending_input, text.as_bytes())?;
                    }
                    Event::Resize(cols, rows) => {
                        self.flush_input(&mut pending_input)?;
                        self.invoke(
                            RemoteDeviceSessionAbility::Resize,
                            json!({
                                "session_id": self.session_id,
                                "cols": cols.max(1),
                                "rows": rows.max(1),
                            }),
                        )?;
                    }
                    _ => {}
                }
            }
            self.flush_input(&mut pending_input)?;
        }
    }

    fn buffer_input(&mut self, pending: &mut Vec<u8>, bytes: &[u8]) -> anyhow::Result<()> {
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let available = TERMINAL_INPUT_BATCH_BYTES.saturating_sub(pending.len());
            if available == 0 {
                self.flush_input(pending)?;
                continue;
            }
            let take = available.min(remaining.len());
            pending.extend_from_slice(&remaining[..take]);
            remaining = &remaining[take..];
            if pending.len() == TERMINAL_INPUT_BATCH_BYTES {
                self.flush_input(pending)?;
            }
        }
        Ok(())
    }

    fn flush_input(&mut self, pending: &mut Vec<u8>) -> anyhow::Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::take(pending);
        self.invoke(
            RemoteDeviceSessionAbility::Input,
            json!({
                "session_id": self.session_id,
                "data": BASE64_STANDARD.encode(bytes),
            }),
        )?;
        Ok(())
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
}
