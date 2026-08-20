//! Browser plugin runtime and bounded session repository.
//! ======================================================
//!
//! File: plugins/browser/src/runtime.rs
//! Description: Reserve resources, open browser sessions, enforce access, and
//!              reap idle sessions.
//!
//! Protocol Responsibility:
//! - Project complete Axon envelope identity onto plugin session authority.
//!
//! Implementation Approach:
//! - One repository lock covers explicit active/closing row sets, opening
//!   reservations, and named profile leases. Chrome/CDP work stays outside it.
//! - Capacity is reserved before process launch and rolled back on every error.
//!
//! Usage Contract:
//! - Session operations use the invocation subject as the session URA.
//! - Named profiles are single-writer within one daemon process.
//!
//! Architectural Position:
//! - Browser plugin application service.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use url::Url;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::support::async_bridge::{
    run_blocking, spawn_current_thread_tokio, SyncBridgeRuntimePolicy,
};

use super::chrome::{open_target, ChromeOpenOptions};
use super::constants::*;
use super::errors::{BrowserError, BrowserResult};
use super::session::{now_ms, BrowserSession, BrowserSessionState, CloseOutcome};

struct SessionEntry {
    session: Arc<BrowserSession>,
    profile: Option<String>,
}

#[derive(Default)]
struct BrowserRepository {
    active_sessions: HashMap<String, SessionEntry>,
    closing_sessions: HashMap<String, SessionEntry>,
    opening: usize,
    leased_profiles: HashSet<String>,
}

pub struct BrowserRuntime {
    repository: Mutex<BrowserRepository>,
    max_sessions: usize,
    max_frame_queue: usize,
}

impl BrowserRuntime {
    pub fn new(max_sessions: usize, max_frame_queue: usize) -> Arc<Self> {
        let runtime = Arc::new(Self {
            repository: Mutex::new(BrowserRepository::default()),
            max_sessions: max_sessions.max(1),
            max_frame_queue: max_frame_queue.max(1),
        });
        Self::start_idle_reaper(&runtime);
        runtime
    }

    pub fn max_frame_queue(&self) -> usize {
        self.max_frame_queue
    }

    pub fn open_session(
        self: &Arc<Self>,
        env: &EnvelopeContext,
        args: Value,
    ) -> BrowserResult<Value> {
        let request = OpenSessionRequest::parse(args)?;
        self.reserve_open(request.profile.as_deref())?;
        let opened = run_blocking(
            open_target(request.chrome_options()),
            SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        );
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                self.rollback_open(request.profile.as_deref());
                return Err(error);
            }
        };

        let realm = crate::core::ura::realm_from_ura(env.callee())
            .or_else(|| crate::core::ura::realm_from_ura(env.caller()))
            .unwrap_or_else(|| "local".to_string());
        let session_ura = crate::core::ura::resource_dot_ura(
            &realm,
            "plugin.easynet.browser",
            &format!("session/{}", uuid::Uuid::new_v4().simple()),
        );
        let session = match BrowserSession::from_opened(
            session_ura.clone(),
            env.caller().to_string(),
            request.url,
            request.idle_timeout_seconds,
            opened,
        ) {
            Ok(session) => session,
            Err(error) => {
                self.rollback_open(request.profile.as_deref());
                return Err(error);
            }
        };
        self.commit_open(Arc::clone(&session), request.profile);
        Ok(session.status())
    }

    pub fn require_session(
        &self,
        ability: &'static str,
        env: &EnvelopeContext,
    ) -> BrowserResult<Arc<BrowserSession>> {
        let session = self
            .repository
            .lock()
            .expect("browser repository poisoned")
            .active_sessions
            .get(env.subject())
            .map(|entry| Arc::clone(&entry.session))
            .ok_or_else(|| BrowserError::SessionNotFound {
                ability,
                session_ura: env.subject().to_string(),
            })?;
        session.require_access(ability, env.caller(), env.subject())?;
        Ok(session)
    }

    pub fn show_session(&self, env: &EnvelopeContext, args: Value) -> BrowserResult<Value> {
        require_empty_args(ABILITY_SHOW_SESSION, &args)?;
        Ok(self.require_session(ABILITY_SHOW_SESSION, env)?.status())
    }

    pub fn close_session(&self, env: &EnvelopeContext, args: Value) -> BrowserResult<Value> {
        require_empty_args(ABILITY_CLOSE_SESSION, &args)?;
        let entry = {
            let repository = self.repository.lock().expect("browser repository poisoned");
            repository
                .active_sessions
                .get(env.subject())
                .or_else(|| repository.closing_sessions.get(env.subject()))
                .map(|entry| Arc::clone(&entry.session))
        };
        let Some(session) = entry else {
            return Ok(json!({
                "session_ura": env.subject(),
                "state": "closed",
                "already_closed": true,
                "warnings": [],
            }));
        };
        session.require_identity(ABILITY_CLOSE_SESSION, env.caller(), env.subject())?;
        self.promote_to_closing(&session);
        let outcome = run_blocking(
            session.close("explicit_close"),
            SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        );
        self.finalize_close(&session, &outcome);
        Ok(close_outcome_value(session.session_ura(), outcome))
    }

    pub async fn close_session_from_runtime(
        &self,
        session: Arc<BrowserSession>,
        reason: &str,
    ) -> CloseOutcome {
        self.promote_to_closing(&session);
        let outcome = session.close(reason).await;
        self.finalize_close(&session, &outcome);
        outcome
    }

    fn reserve_open(&self, profile: Option<&str>) -> BrowserResult<()> {
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        let reserved = repository
            .active_sessions
            .len()
            .saturating_add(repository.closing_sessions.len())
            .saturating_add(repository.opening);
        if reserved >= self.max_sessions {
            return Err(BrowserError::SessionStoreFull {
                ability: ABILITY_OPEN_SESSION,
            });
        }
        if let Some(profile) = profile {
            if !repository.leased_profiles.insert(profile.to_string()) {
                return Err(BrowserError::InvalidArgument {
                    ability: ABILITY_OPEN_SESSION,
                    detail: format!("named profile {profile:?} is already in use"),
                });
            }
        }
        repository.opening += 1;
        Ok(())
    }

    fn rollback_open(&self, profile: Option<&str>) {
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        repository.opening = repository.opening.saturating_sub(1);
        if let Some(profile) = profile {
            repository.leased_profiles.remove(profile);
        }
    }

    fn commit_open(&self, session: Arc<BrowserSession>, profile: Option<String>) {
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        repository.opening = repository.opening.saturating_sub(1);
        repository.active_sessions.insert(
            session.session_ura().to_string(),
            SessionEntry { session, profile },
        );
    }

    fn finalize_close(&self, session: &Arc<BrowserSession>, outcome: &CloseOutcome) {
        if outcome.state != BrowserSessionState::Closed {
            return;
        }
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        let remove_closing = repository
            .closing_sessions
            .get(session.session_ura())
            .is_some_and(|entry| Arc::ptr_eq(&entry.session, session));
        if remove_closing {
            let entry = repository
                .closing_sessions
                .remove(session.session_ura())
                .expect("checked closing browser session");
            if let Some(profile) = entry.profile {
                repository.leased_profiles.remove(&profile);
            }
        }
    }

    fn promote_to_closing(&self, session: &Arc<BrowserSession>) {
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        if let Some(entry) = repository.closing_sessions.get(session.session_ura()) {
            debug_assert!(Arc::ptr_eq(&entry.session, session));
            return;
        }
        let is_same_session = repository
            .active_sessions
            .get(session.session_ura())
            .is_some_and(|entry| Arc::ptr_eq(&entry.session, session));
        if !is_same_session {
            return;
        }
        let entry = repository
            .active_sessions
            .remove(session.session_ura())
            .expect("checked active browser session");
        repository
            .closing_sessions
            .insert(session.session_ura().to_string(), entry);
    }

    fn start_idle_reaper(runtime: &Arc<Self>) {
        let runtime = Arc::downgrade(runtime);
        let _ = std::thread::Builder::new()
            .name("easynet-browser-idle-reaper".to_string())
            .spawn(move || idle_reaper_loop(runtime));
    }

    fn take_expired_sessions(&self) -> Vec<Arc<BrowserSession>> {
        let now = now_ms();
        let mut repository = self.repository.lock().expect("browser repository poisoned");
        let expired = repository
            .active_sessions
            .iter()
            .filter(|(_, entry)| entry.session.is_idle_expired(now))
            .map(|(session_ura, _)| session_ura.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|session_ura| {
                let entry = repository.active_sessions.remove(&session_ura)?;
                let session = Arc::clone(&entry.session);
                repository.closing_sessions.insert(session_ura, entry);
                Some(session)
            })
            .collect()
    }

    #[cfg(test)]
    fn reservation_counts(&self) -> (usize, usize, usize, usize) {
        let repository = self.repository.lock().expect("browser repository poisoned");
        (
            repository.active_sessions.len(),
            repository.closing_sessions.len(),
            repository.opening,
            repository.leased_profiles.len(),
        )
    }
}

fn idle_reaper_loop(runtime: Weak<BrowserRuntime>) {
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let Some(runtime) = runtime.upgrade() else {
            return;
        };
        for session in runtime.take_expired_sessions() {
            let runtime_for_close = Arc::clone(&runtime);
            let session_for_close = Arc::clone(&session);
            let _ = spawn_current_thread_tokio(
                format!("easynet-browser-close-{}", uuid::Uuid::new_v4().simple()),
                async move {
                    let outcome = session_for_close.close("idle_timeout").await;
                    runtime_for_close.finalize_close(&session_for_close, &outcome);
                },
                |_| {},
            );
        }
    }
}

fn close_outcome_value(session_ura: &str, outcome: CloseOutcome) -> Value {
    json!({
        "session_ura": session_ura,
        "state": outcome.state.as_str(),
        "already_closed": outcome.already_closed,
        "warnings": outcome.warnings,
    })
}

fn require_empty_args(ability: &'static str, args: &Value) -> BrowserResult<()> {
    let object = args
        .as_object()
        .ok_or_else(|| BrowserError::InvalidArgument {
            ability,
            detail: "args must be a JSON object".to_string(),
        })?;
    if object.is_empty() {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument {
            ability,
            detail: format!(
                "unsupported argument field(s): {}",
                sorted_keys(object).join(", ")
            ),
        })
    }
}

#[derive(Clone, Debug)]
struct OpenSessionRequest {
    url: String,
    headless: bool,
    cdp_endpoint: Option<String>,
    executable_path: Option<String>,
    profile: Option<String>,
    viewport_width: u32,
    viewport_height: u32,
    idle_timeout_seconds: u64,
}

impl OpenSessionRequest {
    fn parse(args: Value) -> BrowserResult<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| BrowserError::InvalidArgument {
                ability: ABILITY_OPEN_SESSION,
                detail: "args must be a JSON object".to_string(),
            })?;
        reject_unknown_fields(
            object,
            &[
                "url",
                "headless",
                "cdp_endpoint",
                "executable_path",
                "profile",
                "viewport_width",
                "viewport_height",
                "idle_timeout_seconds",
            ],
        )?;
        let url = required_string(object, "url", MAX_URL_BYTES)?;
        let parsed = Url::parse(&url).map_err(|error| BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("invalid url: {error}"),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(BrowserError::InvalidArgument {
                ability: ABILITY_OPEN_SESSION,
                detail: "url must be an absolute http:// or https:// URL".to_string(),
            });
        }
        Ok(Self {
            url,
            // Remote sessions render into the screencast/attach pipeline; popping a
            // visible window on the target machine's desktop is the exception
            // (browser.show_session exists for that), not the default.
            headless: optional_bool(object, "headless", true)?,
            cdp_endpoint: optional_string(object, "cdp_endpoint")?,
            executable_path: optional_string(object, "executable_path")?,
            profile: optional_string(object, "profile")?,
            viewport_width: optional_u32(
                object,
                "viewport_width",
                DEFAULT_VIEWPORT_WIDTH,
                320,
                3840,
            )?,
            viewport_height: optional_u32(
                object,
                "viewport_height",
                DEFAULT_VIEWPORT_HEIGHT,
                240,
                2400,
            )?,
            idle_timeout_seconds: optional_u64(
                object,
                "idle_timeout_seconds",
                DEFAULT_IDLE_TIMEOUT_SECONDS,
                MIN_IDLE_TIMEOUT_SECONDS,
                MAX_IDLE_TIMEOUT_SECONDS,
            )?,
        })
    }

    fn chrome_options(&self) -> ChromeOpenOptions {
        ChromeOpenOptions {
            url: self.url.clone(),
            headless: self.headless,
            cdp_endpoint: self.cdp_endpoint.clone(),
            executable_path: self.executable_path.clone(),
            profile: self.profile.clone(),
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
        }
    }
}

fn reject_unknown_fields(object: &Map<String, Value>, allowed: &[&str]) -> BrowserResult<()> {
    let unknown = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("unsupported argument field(s): {}", unknown.join(", ")),
        })
    }
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> BrowserResult<String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` is required and must be a non-empty string"),
        })?;
    if value.len() > max_bytes {
        return Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` exceeds {max_bytes} bytes"),
        });
    }
    Ok(value.to_string())
}

fn optional_string(object: &Map<String, Value>, key: &str) -> BrowserResult<Option<String>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` must be a non-empty string"),
        })?;
    if value.len() > MAX_BROWSER_OPTION_BYTES {
        return Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` exceeds {MAX_BROWSER_OPTION_BYTES} bytes"),
        });
    }
    Ok(Some(value.to_string()))
}

fn optional_bool(object: &Map<String, Value>, key: &str, default: bool) -> BrowserResult<bool> {
    object
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| BrowserError::InvalidArgument {
                    ability: ABILITY_OPEN_SESSION,
                    detail: format!("`{key}` must be a boolean"),
                })
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn optional_u32(
    object: &Map<String, Value>,
    key: &str,
    default: u32,
    min: u32,
    max: u32,
) -> BrowserResult<u32> {
    let value = optional_u64(object, key, default as u64, min as u64, max as u64)?;
    Ok(value as u32)
}

fn optional_u64(
    object: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> BrowserResult<u64> {
    let Some(value) = object.get(key) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` must be an integer"),
        })?;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("`{key}` must be between {min} and {max}"),
        })
    }
}

fn sorted_keys(object: &Map<String, Value>) -> Vec<&str> {
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_request_defaults_to_headless_remote_browser() {
        // Remote sessions render into the screencast/attach pipeline; a
        // visible window on the target desktop is the explicit exception
        // (browser.show_session), never the default.
        let request = OpenSessionRequest::parse(json!({"url": "https://example.com"})).unwrap();
        assert!(request.headless);
        assert_eq!(request.viewport_width, DEFAULT_VIEWPORT_WIDTH);
        assert_eq!(request.idle_timeout_seconds, DEFAULT_IDLE_TIMEOUT_SECONDS);
    }

    #[test]
    fn open_request_rejects_non_http_and_unknown_fields() {
        assert!(OpenSessionRequest::parse(json!({"url": "file:///etc/passwd"})).is_err());
        assert!(OpenSessionRequest::parse(json!({
            "url": "https://example.com",
            "session_ura": "forged"
        }))
        .is_err());
    }

    #[test]
    fn capacity_and_profile_reservations_roll_back_together() {
        let runtime = BrowserRuntime::new(1, 2);
        runtime.reserve_open(Some("work")).expect("reserve");
        assert_eq!(runtime.reservation_counts(), (0, 0, 1, 1));
        assert!(runtime.reserve_open(Some("other")).is_err());
        runtime.rollback_open(Some("work"));
        assert_eq!(runtime.reservation_counts(), (0, 0, 0, 0));
    }
}
