// EasyNet CLI - remote desktop permission helpers
// =================================================
//
// File: src/plugins/builtin/remote_desktop/permissions.rs
// Description: Host-local screen capture and input permission probes.

use serde_json::{json, Value};

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    is_resource_ura_subject, reject_subject_in_args,
};
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::constants::REASON_INVALID_ARGUMENT;
use crate::plugins::remote_desktop::input::{
    input_injection_available, request_input_injection_permission,
};

pub(in crate::plugins::builtin::remote_desktop) fn ensure_permission_probe_access(
    ability: &str,
    env: &EnvelopeContext,
    args: &Value,
) -> anyhow::Result<()> {
    reject_subject_in_args(ability, args)?;
    let resource_scoped = is_resource_ura_subject(env.subject());
    if resource_scoped {
        anyhow::bail!(
            "{ability}: screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(())
}

pub(in crate::plugins::builtin::remote_desktop) fn screen_capture_permission_status() -> Value {
    let granted = platform_screen_capture_permission_granted();
    let input_granted = input_injection_available();
    json!({
        "permission": "screen_capture",
        "platform": std::env::consts::OS,
        "granted": granted,
        "requestable": platform_screen_capture_permission_requestable(),
        "input_permission": {
            "permission": "accessibility",
            "granted": input_granted,
            "requestable": cfg!(target_os = "macos"),
            "required_for": ["pointer", "keyboard", "wheel"],
        },
        "process_path": std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string()),
        "restart_recommended_after_grant": cfg!(target_os = "macos"),
        "settings_hint": platform_screen_capture_settings_hint(),
    })
}

pub(in crate::plugins::builtin::remote_desktop) fn request_screen_capture_permission() -> Value {
    let before = platform_screen_capture_permission_granted();
    let input_before = input_injection_available();
    let after = if before {
        true
    } else {
        platform_request_screen_capture_permission()
    };
    let input_after = if input_before {
        true
    } else {
        request_input_injection_permission()
    };
    json!({
        "permission": "screen_capture",
        "platform": std::env::consts::OS,
        "granted": after,
        "previously_granted": before,
        "requested": !before,
        "requestable": platform_screen_capture_permission_requestable(),
        "input_permission": {
            "permission": "accessibility",
            "granted": input_after,
            "previously_granted": input_before,
            "requested": !input_before,
            "requestable": cfg!(target_os = "macos"),
            "required_for": ["pointer", "keyboard", "wheel"],
        },
        "process_path": std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string()),
        "restart_recommended_after_grant": cfg!(target_os = "macos") && !before && after,
        "settings_hint": platform_screen_capture_settings_hint(),
    })
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_permission_granted() -> bool {
    crate::plugins::remote_desktop::screencapturekit_capture::screen_capture_permission_granted()
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission_granted() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn platform_request_screen_capture_permission() -> bool {
    crate::plugins::remote_desktop::screencapturekit_capture::request_screen_capture_permission()
}

#[cfg(not(target_os = "macos"))]
fn platform_request_screen_capture_permission() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_permission_requestable() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission_requestable() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_settings_hint() -> &'static str {
    "System Settings > Privacy & Security > Screen & System Audio Recording"
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_settings_hint() -> &'static str {
    "No OS-level screen-capture TCC prompt is required on this platform."
}
