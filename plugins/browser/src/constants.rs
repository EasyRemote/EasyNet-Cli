//! Stable browser plugin names and runtime bounds.
//! ===============================================
//!
//! File: plugins/browser/src/constants.rs
//! Description: Canonical browser ability names, capacity limits, and reasons.
//!
//! Protocol Responsibility:
//! - Define fixed application bounds below Axon's transport-level admission.
//!
//! Implementation Approach:
//! - Keep every queue, batch, timeout, capture, and lifecycle limit explicit.
//!
//! Usage Contract:
//! - Runtime code must use these constants instead of introducing local limits.
//!
//! Architectural Position:
//! - Browser plugin shared contract vocabulary.

pub const ABILITY_OPEN_SESSION: &str = "browser.open_session";
pub const ABILITY_SHOW_SESSION: &str = "browser.show_session";
pub const ABILITY_SEND_INPUT: &str = "browser.send_input";
pub const ABILITY_CAPTURE_VIEWPORT: &str = "browser.capture_viewport";
pub const ABILITY_ATTACH_SESSION: &str = "browser.attach_session";
pub const ABILITY_CLOSE_SESSION: &str = "browser.close_session";
pub const ABILITY_CAPTURE_PAGE: &str = "browser.capture_page";
#[cfg(test)]
pub const PUBLIC_ABILITIES: [&str; 7] = [
    ABILITY_OPEN_SESSION,
    ABILITY_SHOW_SESSION,
    ABILITY_SEND_INPUT,
    ABILITY_CAPTURE_VIEWPORT,
    ABILITY_ATTACH_SESSION,
    ABILITY_CLOSE_SESSION,
    ABILITY_CAPTURE_PAGE,
];

/// Upper bound for one structural page snapshot. MHTML inlines every
/// resource, so pathological pages can balloon; the cap keeps a single
/// rpc payload bounded without truncating silently.
pub const MAX_PAGE_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

pub const DEFAULT_VIEWPORT_WIDTH: u32 = 1280;
pub const DEFAULT_VIEWPORT_HEIGHT: u32 = 800;
// Two minutes, not thirty: every session owns a real Chrome process tree on
// the host. Abandoned sessions (page reloads, retries, crashed viewers)
// accumulate fast during interactive use, and dozens of idle Chromes starve
// the machine long before a 30-minute reaper would fire. Callers that need
// long-lived sessions pass idle_timeout_seconds explicitly.
pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u64 = 120;
pub const MIN_IDLE_TIMEOUT_SECONDS: u64 = 60;
pub const MAX_IDLE_TIMEOUT_SECONDS: u64 = 7200;
pub const CDP_PENDING_BOUND: usize = 256;
pub const CDP_EVENT_BOUND: usize = 512;
/// Maximum attachment operations admitted beyond the bounded Axon channel.
/// Raw CDP commands may complete out of order by correlation id; high-level
/// input uses a serial lane so pointer and keyboard order remains deterministic.
pub const ATTACH_OPERATION_BOUND: usize = 32;
pub const ATTACH_BATCH_COMMAND_BOUND: usize = ATTACH_OPERATION_BOUND;
pub const CDP_COMMAND_TIMEOUT_SECONDS: u64 = 15;
pub const CHROME_DISCOVERY_TIMEOUT_SECONDS: u64 = 10;
pub const MIN_CAPTURE_FRAMES: u64 = 1;
pub const MAX_CAPTURE_FRAMES: u64 = 300;
pub const MAX_CDP_METHOD_BYTES: usize = 256;
pub const MAX_CORRELATION_ID_BYTES: usize = 256;
pub const MAX_URL_BYTES: usize = 65_536;
pub const MAX_SELECTOR_BYTES: usize = 4_096;
pub const MAX_INPUT_TEXT_BYTES: usize = 65_536;
pub const MAX_BROWSER_OPTION_BYTES: usize = 65_536;
pub const MAX_KEY_BYTES: usize = 256;

pub const REASON_INVALID_ARGUMENT: &str = "browser_invalid_argument";
pub const REASON_SUBJECT_MISMATCH: &str = "browser_subject_mismatch";
pub const REASON_CALLER_MISMATCH: &str = "browser_caller_mismatch";
pub const REASON_SESSION_NOT_FOUND: &str = "browser_session_not_found";
pub const REASON_SESSION_TERMINAL: &str = "browser_session_terminal";
pub const REASON_SESSION_STORE_FULL: &str = "browser_session_store_full";
pub const REASON_ATTACHMENT_ACTIVE: &str = "browser_attachment_already_active";
pub const REASON_CAPTURE_ACTIVE: &str = "browser_capture_already_active";
pub const REASON_CDP_POLICY: &str = "browser_cdp_policy_denied";
pub const REASON_CDP_UNAVAILABLE: &str = "browser_cdp_unavailable";
