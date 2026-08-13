// EasyNet CLI - remote desktop permission helpers
// =================================================
//
// File: plugins/remote-desktop/src/permissions.rs
// Description: Host-local screen capture and input permission probes.

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, request_input_injection_permission,
};

pub(in crate::daemon::plugins::remote_desktop) fn ensure_permission_probe_access(
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
) -> anyhow::Result<()> {
    reject_permission_subject_in_args(ability, args)?;
    HostLocalPermissionProbeSubject::try_from_envelope(ability, env).map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostLocalPermissionProbeSubject {
    UserSelf { user_ura: String },
    UserInvokeResource { resource_ura: String },
    LocalSystemLoopback,
}

impl HostLocalPermissionProbeSubject {
    fn try_from_envelope(ability: &'static str, env: &EnvelopeContext) -> anyhow::Result<Self> {
        let caller = env.caller().trim();
        let subject = env.subject().trim();
        if caller == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
            return if subject == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                Ok(Self::LocalSystemLoopback)
            } else {
                Err(host_local_subject_error(
                    ability,
                    "local-system permission probes must use the local-system subject",
                ))
            };
        }
        let parsed_caller = crate::core::ura::parse_ura(caller).map_err(|error| {
            host_local_subject_error(
                ability,
                &format!("caller_ura is not a canonical host-local caller: {error}"),
            )
        })?;
        let parsed_subject = crate::core::ura::parse_ura(subject).map_err(|error| {
            host_local_subject_error(
                ability,
                &format!("subject_ura is not a canonical host-local subject: {error}"),
            )
        })?;
        if parsed_caller.kind != crate::core::ura::URAKind::User {
            return Err(host_local_subject_error(
                ability,
                "host-local permission probes require a User caller or local-system loopback",
            ));
        }
        if parsed_subject.kind == crate::core::ura::URAKind::Resource {
            return if user_descriptor_bound_permission_probe_subject(
                &parsed_caller,
                &parsed_subject,
                ability,
            ) {
                Ok(Self::UserInvokeResource {
                    resource_ura: subject.to_string(),
                })
            } else {
                let detail = if remote_desktop_resource_probe_subject(&parsed_subject) {
                    "screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject"
                } else {
                    "host-local permission probes require a caller-owned descriptor-bound invoke resource subject"
                };
                Err(host_local_subject_error(ability, detail))
            };
        }
        if parsed_subject.kind != crate::core::ura::URAKind::User {
            return Err(host_local_subject_error(
                ability,
                "host-local permission probes require a caller-owned User or descriptor-bound invoke resource subject",
            ));
        }
        if caller != subject {
            return Err(host_local_subject_error(
                ability,
                "host-local permission probe subject must match the authenticated caller",
            ));
        }
        Ok(Self::UserSelf {
            user_ura: subject.to_string(),
        })
    }
}

fn remote_desktop_resource_probe_subject(subject: &crate::core::ura::ParsedURA) -> bool {
    subject
        .resource_owner_id()
        .is_some_and(|owner| owner.starts_with("device."))
        || subject
            .resource_path()
            .is_some_and(|path| path.starts_with("streams/"))
}

fn user_descriptor_bound_permission_probe_subject(
    caller: &crate::core::ura::ParsedURA,
    subject: &crate::core::ura::ParsedURA,
    ability: &'static str,
) -> bool {
    let Some(caller_user_id) = caller.user_id() else {
        return false;
    };
    let Some(owner_user_id) = subject
        .resource_owner_id()
        .and_then(|owner| owner.strip_prefix("user.").or(Some(owner)))
    else {
        return false;
    };
    subject.realm == caller.realm
        && owner_user_id == caller_user_id
        && subject.resource_path() == Some(format!("invoke/{}", ability.trim()).as_str())
}

fn host_local_subject_error(ability: &'static str, detail: &str) -> anyhow::Error {
    anyhow::Error::new(RemoteDesktopError::InvalidArgument {
        ability,
        detail: detail.to_string(),
    })
}

fn reject_permission_subject_in_args(ability: &'static str, args: &Value) -> anyhow::Result<()> {
    if let Value::Object(map) = args {
        if map.contains_key("subject") {
            return Err(host_local_subject_error(
                ability,
                "`subject` MUST come from the invocation envelope, not args",
            ));
        }
    }
    Ok(())
}

pub(in crate::daemon::plugins::remote_desktop) fn screen_capture_permission_status() -> Value {
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

pub(in crate::daemon::plugins::remote_desktop) fn request_screen_capture_permission() -> Value {
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
    crate::daemon::plugins::remote_desktop::screencapturekit_capture::screen_capture_permission_granted()
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission_granted() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn platform_request_screen_capture_permission() -> bool {
    crate::daemon::plugins::remote_desktop::screencapturekit_capture::request_screen_capture_permission()
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
