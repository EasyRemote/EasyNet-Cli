//! Browser session aggregate and lifecycle state machine.
//! ======================================================
//!
//! File: plugins/browser/src/session.rs
//! Description: Own one caller-bound page target and every legal lifecycle
//!              transition around it.
//!
//! Protocol Responsibility:
//! - Enforce plugin session authority before CDP work. Axon admission and
//!   receipts remain outside this aggregate.
//!
//! Implementation Approach:
//! - Keep state transitions synchronous and explicit.
//! - Keep CDP/process handles outside the state lock.
//! - Represent attach and capture cardinality as atomic leases.
//!
//! Usage Contract:
//! - Callers must validate caller and resource subject through `require_access`.
//! - No state lock may be held across a CDP await.
//!
//! Architectural Position:
//! - Browser plugin domain aggregate.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};
use tokio::sync::Notify;

#[cfg(test)]
use super::cdp::CdpConnectionState;
use super::cdp::{CdpClient, CdpEvent, CdpFailure};
use super::chrome::{BrowserVersion, ChromeProcessLease, OpenedChromeTarget};
use super::constants::{REASON_ATTACHMENT_ACTIVE, REASON_CAPTURE_ACTIVE};
use super::errors::{BrowserError, BrowserResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserSessionState {
    Starting,
    Active,
    Closing,
    Closed,
    Failed,
}

impl BrowserSessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Failed => "failed",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Failed)
    }
}

#[derive(Debug)]
struct SessionLifecycle {
    state: BrowserSessionState,
    terminal_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseDisposition {
    Start,
    Wait,
    Complete(BrowserSessionState),
}

impl SessionLifecycle {
    fn starting() -> Self {
        Self {
            state: BrowserSessionState::Starting,
            terminal_reason: None,
        }
    }

    fn activate(&mut self) -> BrowserResult<()> {
        self.transition(
            BrowserSessionState::Starting,
            BrowserSessionState::Active,
            None,
        )
    }

    fn fail(&mut self, reason: String) {
        self.state = BrowserSessionState::Failed;
        self.terminal_reason = Some(reason);
    }

    fn begin_close(&mut self, reason: &str) -> CloseDisposition {
        match self.state {
            BrowserSessionState::Starting | BrowserSessionState::Active => {
                self.state = BrowserSessionState::Closing;
                self.terminal_reason = Some(reason.to_string());
                CloseDisposition::Start
            }
            BrowserSessionState::Closing => CloseDisposition::Wait,
            BrowserSessionState::Closed | BrowserSessionState::Failed => {
                CloseDisposition::Complete(self.state)
            }
        }
    }

    fn finish_close(&mut self) {
        if self.state == BrowserSessionState::Closing {
            self.state = BrowserSessionState::Closed;
        }
    }

    fn transition(
        &mut self,
        expected: BrowserSessionState,
        next: BrowserSessionState,
        reason: Option<String>,
    ) -> BrowserResult<()> {
        if self.state != expected {
            return Err(BrowserError::Cdp {
                ability: "browser.session_lifecycle",
                detail: format!(
                    "illegal browser session transition {} -> {} (expected {})",
                    self.state.as_str(),
                    next.as_str(),
                    expected.as_str()
                ),
            });
        }
        self.state = next;
        self.terminal_reason = reason;
        Ok(())
    }
}

pub struct BrowserSession {
    session_ura: String,
    creator_caller: String,
    initial_url: String,
    target_id: String,
    cdp_session_id: String,
    version: BrowserVersion,
    profile_mode: String,
    browser_owned: bool,
    opened_at_ms: u64,
    last_activity_ms: AtomicU64,
    idle_timeout_ms: u64,
    lifecycle: Mutex<SessionLifecycle>,
    client: Arc<CdpClient>,
    process: Mutex<Option<ChromeProcessLease>>,
    close_notify: Notify,
    attachment_active: AtomicBool,
    capture_active: AtomicBool,
}

impl BrowserSession {
    pub fn from_opened(
        session_ura: String,
        creator_caller: String,
        initial_url: String,
        idle_timeout_seconds: u64,
        opened: OpenedChromeTarget,
    ) -> BrowserResult<Arc<Self>> {
        let now = now_ms();
        let session = Arc::new(Self {
            session_ura,
            creator_caller,
            initial_url,
            target_id: opened.target_id,
            cdp_session_id: opened.cdp_session_id,
            version: opened.version,
            profile_mode: opened.profile_mode,
            browser_owned: opened.browser_owned,
            opened_at_ms: now,
            last_activity_ms: AtomicU64::new(now),
            idle_timeout_ms: idle_timeout_seconds.saturating_mul(1000),
            lifecycle: Mutex::new(SessionLifecycle::starting()),
            client: opened.client,
            process: Mutex::new(opened.process),
            close_notify: Notify::new(),
            attachment_active: AtomicBool::new(false),
            capture_active: AtomicBool::new(false),
        });
        let activation = session
            .lifecycle
            .lock()
            .expect("browser lifecycle poisoned")
            .activate();
        if let Err(error) = activation {
            session
                .lifecycle
                .lock()
                .expect("browser lifecycle poisoned")
                .fail(error.to_string());
            return Err(error);
        }
        Ok(session)
    }

    pub fn session_ura(&self) -> &str {
        &self.session_ura
    }

    pub fn cdp_session_id(&self) -> &str {
        &self.cdp_session_id
    }

    pub fn client(&self) -> &Arc<CdpClient> {
        &self.client
    }

    pub fn state(&self) -> BrowserSessionState {
        self.lifecycle
            .lock()
            .expect("browser lifecycle poisoned")
            .state
    }

    pub fn require_access(
        &self,
        ability: &'static str,
        caller: &str,
        subject: &str,
    ) -> BrowserResult<()> {
        self.require_identity(ability, caller, subject)?;
        if self.state().is_terminal() || self.state() != BrowserSessionState::Active {
            return Err(BrowserError::SessionTerminal {
                ability,
                session_ura: self.session_ura.clone(),
            });
        }
        self.touch();
        Ok(())
    }

    pub fn require_identity(
        &self,
        ability: &'static str,
        caller: &str,
        subject: &str,
    ) -> BrowserResult<()> {
        if subject != self.session_ura {
            return Err(BrowserError::SubjectMismatch {
                ability,
                expected: self.session_ura.clone(),
                actual: subject.to_string(),
            });
        }
        if caller != self.creator_caller {
            return Err(BrowserError::CallerMismatch {
                ability,
                expected: self.creator_caller.clone(),
                actual: caller.to_string(),
            });
        }
        Ok(())
    }

    pub fn status(&self) -> Value {
        let lifecycle = self.lifecycle.lock().expect("browser lifecycle poisoned");
        json!({
            "session_ura": self.session_ura,
            "state": lifecycle.state.as_str(),
            "terminal_reason": lifecycle.terminal_reason,
            "initial_url": self.initial_url,
            "target_id": self.target_id,
            "browser": {
                "product": self.version.product,
                "protocol_version": self.version.protocol_version,
                "user_agent": self.version.user_agent,
                "js_version": self.version.js_version,
                "owned": self.browser_owned,
                "profile_mode": self.profile_mode,
            },
            "cdp_state": self.client.state().as_str(),
            "opened_at_ms": self.opened_at_ms,
            "last_activity_ms": self.last_activity_ms.load(Ordering::Relaxed),
            "idle_timeout_ms": self.idle_timeout_ms,
            "attachment_active": self.attachment_active.load(Ordering::Acquire),
            "capture_active": self.capture_active.load(Ordering::Acquire),
        })
    }

    pub async fn command(&self, method: &str, params: Option<Value>) -> BrowserResult<Value> {
        if self.state() != BrowserSessionState::Active {
            return Err(BrowserError::SessionTerminal {
                ability: "browser.cdp",
                session_ura: self.session_ura.clone(),
            });
        }
        self.touch();
        crate::op_event!(component = browser_plugin, kind = cdp_command_begin, method = method);
        let started = std::time::Instant::now();
        let result = self
            .client
            .send_command(method, params, Some(&self.cdp_session_id))
            .await
            .map_err(|error| BrowserError::Cdp {
                ability: "browser.cdp",
                detail: error.to_string(),
            });
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let ok = result.is_ok();
        crate::op_event!(
            component = browser_plugin,
            kind = cdp_command_end,
            method = method,
            elapsed_ms = elapsed_ms,
            ok = ok,
        );
        result
    }

    pub async fn raw_command(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CdpFailure> {
        if self.state() != BrowserSessionState::Active {
            return Err(CdpFailure::Closed);
        }
        self.touch();
        self.client
            .send_command(method, params, Some(&self.cdp_session_id))
            .await
    }

    pub fn event_belongs_to_session(&self, event: &CdpEvent) -> bool {
        event.session_id.as_deref() == Some(self.cdp_session_id.as_str())
    }

    pub fn begin_attachment(self: &Arc<Self>) -> BrowserResult<SessionActivityLease> {
        self.begin_activity(
            SessionActivityKind::Attachment,
            &self.attachment_active,
            REASON_ATTACHMENT_ACTIVE,
        )
    }

    pub fn begin_capture(self: &Arc<Self>) -> BrowserResult<SessionActivityLease> {
        self.begin_activity(
            SessionActivityKind::Capture,
            &self.capture_active,
            REASON_CAPTURE_ACTIVE,
        )
    }

    fn begin_activity(
        self: &Arc<Self>,
        kind: SessionActivityKind,
        flag: &AtomicBool,
        _reason: &'static str,
    ) -> BrowserResult<SessionActivityLease> {
        if self.state() != BrowserSessionState::Active {
            return Err(BrowserError::SessionTerminal {
                ability: kind.ability(),
                session_ura: self.session_ura.clone(),
            });
        }
        if flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(match kind {
                SessionActivityKind::Attachment => BrowserError::AttachmentActive {
                    ability: kind.ability(),
                },
                SessionActivityKind::Capture => BrowserError::CaptureActive {
                    ability: kind.ability(),
                },
            });
        }
        self.touch();
        Ok(SessionActivityLease {
            session: Arc::downgrade(self),
            kind,
        })
    }

    pub fn is_idle_expired(&self, now_ms: u64) -> bool {
        self.state() == BrowserSessionState::Active
            && now_ms.saturating_sub(self.last_activity_ms.load(Ordering::Relaxed))
                >= self.idle_timeout_ms
    }

    pub async fn close(&self, reason: &str) -> CloseOutcome {
        let close_disposition = self
            .lifecycle
            .lock()
            .expect("browser lifecycle poisoned")
            .begin_close(reason);
        match close_disposition {
            CloseDisposition::Start => {}
            CloseDisposition::Wait => {
                return CloseOutcome {
                    state: self.wait_for_close_completion().await,
                    already_closed: true,
                    warnings: Vec::new(),
                };
            }
            CloseDisposition::Complete(state) => {
                return CloseOutcome {
                    state,
                    already_closed: true,
                    warnings: Vec::new(),
                };
            }
        }

        self.attachment_active.store(false, Ordering::Release);
        self.capture_active.store(false, Ordering::Release);
        let mut warnings = Vec::new();
        if let Err(error) = self
            .client
            .send_command(
                "Target.closeTarget",
                Some(json!({"targetId": self.target_id})),
                None,
            )
            .await
        {
            if !matches!(error, CdpFailure::Closed) {
                warnings.push(error.to_string());
            }
        }
        if self.browser_owned {
            let _ = self.client.send_command("Browser.close", None, None).await;
        }
        self.client.shutdown().await;
        let process = self
            .process
            .lock()
            .expect("browser process lease poisoned")
            .take();
        if let Some(process) = process {
            if let Err(error) = tokio::task::spawn_blocking(move || process.terminate()).await {
                warnings.push(format!("Chrome process cleanup task failed: {error}"));
            }
        }
        self.lifecycle
            .lock()
            .expect("browser lifecycle poisoned")
            .finish_close();
        self.close_notify.notify_waiters();
        CloseOutcome {
            state: BrowserSessionState::Closed,
            already_closed: false,
            warnings,
        }
    }

    async fn wait_for_close_completion(&self) -> BrowserSessionState {
        loop {
            let notified = self.close_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let state = self.state();
            if state != BrowserSessionState::Closing {
                return state;
            }
            notified.await;
        }
    }

    fn touch(&self) {
        self.last_activity_ms.store(now_ms(), Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
enum SessionActivityKind {
    Attachment,
    Capture,
}

impl SessionActivityKind {
    fn ability(self) -> &'static str {
        match self {
            Self::Attachment => super::constants::ABILITY_ATTACH_SESSION,
            Self::Capture => super::constants::ABILITY_CAPTURE_VIEWPORT,
        }
    }
}

pub struct SessionActivityLease {
    session: Weak<BrowserSession>,
    kind: SessionActivityKind,
}

impl Drop for SessionActivityLease {
    fn drop(&mut self) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        match self.kind {
            SessionActivityKind::Attachment => {
                session.attachment_active.store(false, Ordering::Release);
            }
            SessionActivityKind::Capture => {
                session.capture_active.store(false, Ordering::Release);
            }
        }
        session.touch();
    }
}

pub struct CloseOutcome {
    pub state: BrowserSessionState,
    pub already_closed: bool,
    pub warnings: Vec<String>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_rejects_skipping_active() {
        let mut lifecycle = SessionLifecycle::starting();
        assert!(lifecycle
            .transition(
                BrowserSessionState::Active,
                BrowserSessionState::Closed,
                None,
            )
            .is_err());
        assert_eq!(lifecycle.state, BrowserSessionState::Starting);
    }

    #[test]
    fn lifecycle_close_is_idempotent() {
        let mut lifecycle = SessionLifecycle::starting();
        lifecycle.activate().expect("active");
        assert_eq!(lifecycle.begin_close("explicit"), CloseDisposition::Start);
        assert_eq!(lifecycle.begin_close("duplicate"), CloseDisposition::Wait);
        lifecycle.finish_close();
        assert_eq!(lifecycle.state, BrowserSessionState::Closed);
        assert!(lifecycle.state.is_terminal());
        assert_eq!(
            lifecycle.begin_close("after-terminal"),
            CloseDisposition::Complete(BrowserSessionState::Closed)
        );
    }

    #[test]
    fn connection_state_has_stable_projection() {
        assert_eq!(CdpConnectionState::Active.as_str(), "active");
    }
}
