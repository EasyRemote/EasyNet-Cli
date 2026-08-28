// EasyNet CLI - remote desktop permission helpers
// =================================================
//
// File: plugins/remote-desktop/src/permissions.rs
// Description: Host-local screen capture and input permission probes.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use easynet_remoteapp_native_protocol::screen_capture_permission::{
    Operation as ScreenCapturePermissionOperation, Request as ScreenCapturePermissionRequest,
    Response as ScreenCapturePermissionResponse,
};
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::package::REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, input_injection_backend, input_injection_unavailable_reason,
    request_input_injection_permission,
};
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::native_host_process::execute_one_shot_native_host;

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

pub(in crate::daemon::plugins::remote_desktop) fn host_local_permission_subject_contract() -> Value
{
    json!({
        "kind": "host_local_permission_probe",
        "subject_contract_ura": REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA,
        "allowed_subjects": [
            "caller_user_self",
            "descriptor_bound_invoke_resource",
            "local_system_loopback",
        ],
        "target_resource_subjects_allowed": false,
        "target_resource_rejection_reason": "host_local_permission_probe",
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn screen_capture_permission_status() -> Value {
    let capture = PlatformScreenCapturePermission::status();
    json!({
        "permission": "screen_capture",
        "subject_contract": host_local_permission_subject_contract(),
        "platform": std::env::consts::OS,
        "granted": capture.granted,
        "requestable": capture.requestable,
        "capture_backend": platform_screen_capture_backend(),
        "unavailable_reason": if capture.granted {
            Value::Null
        } else {
            json!(platform_screen_capture_unavailable_reason(capture.granted))
        },
        "permission_probe_error": capture.error,
        "input_permission": InputPermissionProbe::current().to_value(None, None),
        "process_path": capture.process_path,
        "restart_recommended_after_grant": cfg!(target_os = "macos"),
        "settings_hint": platform_screen_capture_settings_hint(),
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn request_screen_capture_permission() -> Value {
    let capture = PlatformScreenCapturePermission::request();
    let input_before = InputPermissionProbe::current();
    let input_requested = !input_before.granted && input_before.requestable;
    let input_after = if input_before.granted {
        input_before
    } else if input_requested {
        InputPermissionProbe::after_request(request_input_injection_permission())
    } else {
        input_before
    };
    json!({
        "permission": "screen_capture",
        "subject_contract": host_local_permission_subject_contract(),
        "platform": std::env::consts::OS,
        "granted": capture.granted,
        "previously_granted": capture.previously_granted,
        "requested": capture.requested,
        "requestable": capture.requestable,
        "capture_backend": platform_screen_capture_backend(),
        "unavailable_reason": if capture.granted {
            Value::Null
        } else {
            json!(platform_screen_capture_unavailable_reason(capture.granted))
        },
        "permission_probe_error": capture.error,
        "input_permission": input_after.to_value(
            Some(input_before.granted),
            Some(input_requested),
        ),
        "process_path": capture.process_path,
        "restart_recommended_after_grant": cfg!(target_os = "macos")
            && !capture.previously_granted
            && capture.granted,
        "settings_hint": platform_screen_capture_settings_hint(),
    })
}

struct PlatformScreenCapturePermission {
    granted: bool,
    requestable: bool,
    previously_granted: bool,
    requested: bool,
    process_path: Option<String>,
    error: Option<String>,
}

impl PlatformScreenCapturePermission {
    fn status() -> Self {
        platform_screen_capture_permission(ScreenCapturePermissionIntent::Status)
    }

    fn request() -> Self {
        platform_screen_capture_permission(ScreenCapturePermissionIntent::Request)
    }
}

#[derive(Clone, Copy)]
enum ScreenCapturePermissionIntent {
    Status,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputPermissionProbe {
    permission: &'static str,
    backend: &'static str,
    granted: bool,
    requestable: bool,
    unavailable_reason: Option<&'static str>,
}

impl InputPermissionProbe {
    fn current() -> Self {
        let granted = input_injection_available();
        Self {
            permission: platform_input_permission_name(),
            backend: input_injection_backend(),
            granted,
            requestable: platform_input_permission_requestable(),
            unavailable_reason: (!granted)
                .then(input_injection_unavailable_reason)
                .flatten(),
        }
    }

    fn after_request(granted: bool) -> Self {
        let current = Self::current();
        Self {
            granted,
            unavailable_reason: (!granted).then_some(current.unavailable_reason).flatten(),
            ..current
        }
    }

    fn to_value(self, previously_granted: Option<bool>, requested: Option<bool>) -> Value {
        let mut value = json!({
            "permission": self.permission,
            "backend": self.backend,
            "granted": self.granted,
            "requestable": self.requestable,
            "unavailable_reason": self.unavailable_reason,
            "required_for": ["pointer", "keyboard", "wheel"],
        });
        let Some(object) = value.as_object_mut() else {
            return value;
        };
        if let Some(previously_granted) = previously_granted {
            object.insert(
                "previously_granted".to_string(),
                Value::Bool(previously_granted),
            );
        }
        if let Some(requested) = requested {
            object.insert("requested".to_string(), Value::Bool(requested));
        }
        value
    }
}

#[cfg(target_os = "macos")]
fn platform_input_permission_name() -> &'static str {
    "accessibility"
}

#[cfg(target_os = "windows")]
fn platform_input_permission_name() -> &'static str {
    "windows_user32_sendinput"
}

#[cfg(target_os = "linux")]
fn platform_input_permission_name() -> &'static str {
    "linux_x11_xtest"
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_input_permission_name() -> &'static str {
    "input_injection"
}

fn platform_input_permission_requestable() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(target_os = "macos")]
static NEXT_MEDIA_HOST_PERMISSION_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "macos")]
fn platform_screen_capture_permission(
    intent: ScreenCapturePermissionIntent,
) -> PlatformScreenCapturePermission {
    let generation = match NEXT_MEDIA_HOST_PERMISSION_GENERATION.fetch_update(
        Ordering::AcqRel,
        Ordering::Acquire,
        |value| value.checked_add(1),
    ) {
        Ok(previous) => previous.saturating_add(1),
        Err(_) => return failed_screen_capture_permission("media-host generation exhausted"),
    };
    let operation = match intent {
        ScreenCapturePermissionIntent::Status => ScreenCapturePermissionOperation::Status,
        ScreenCapturePermissionIntent::Request => ScreenCapturePermissionOperation::Request,
    };
    let request = ScreenCapturePermissionRequest::new(generation, generation, operation);
    let deadline = match intent {
        ScreenCapturePermissionIntent::Status => Duration::from_millis(2_500),
        ScreenCapturePermissionIntent::Request => Duration::from_secs(30),
    };
    let response: ScreenCapturePermissionResponse = match execute_one_shot_native_host(
        generation,
        crate::daemon::plugins::remote_desktop::MEDIA_HOST_EXECUTABLE,
        "screen-capture-permission",
        &[],
        &request,
        deadline,
    ) {
        Ok(response) => response,
        Err(error) => return failed_screen_capture_permission(&error),
    };
    if !response.matches_request(&request) {
        return failed_screen_capture_permission(
            "media-host returned an invalid screen-capture permission response",
        );
    }
    PlatformScreenCapturePermission {
        granted: response.granted,
        requestable: response.requestable,
        previously_granted: response.previously_granted,
        requested: response.requested,
        process_path: response.executable_path,
        error: None,
    }
}

#[cfg(target_os = "macos")]
fn failed_screen_capture_permission(detail: &str) -> PlatformScreenCapturePermission {
    let mut detail = detail.replace('\0', "");
    if detail.len() > 2_048 {
        let mut boundary = 2_048;
        while !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        detail.truncate(boundary);
    }
    PlatformScreenCapturePermission {
        granted: false,
        requestable: true,
        previously_granted: false,
        requested: false,
        process_path:
            crate::daemon::plugins::remote_desktop::native_host_process::sibling_executable(
                crate::daemon::plugins::remote_desktop::MEDIA_HOST_EXECUTABLE,
            )
            .ok()
            .map(|path| path.display().to_string()),
        error: Some(detail),
    }
}

#[cfg(target_os = "windows")]
fn platform_screen_capture_permission_granted() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_screen_capture_permission_granted() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn platform_screen_capture_permission_granted() -> bool {
    matches!(
        current_linux_screen_capture_environment(),
        LinuxScreenCaptureEnvironment::X11Ready
    )
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission(
    _: ScreenCapturePermissionIntent,
) -> PlatformScreenCapturePermission {
    let granted = platform_screen_capture_permission_granted();
    PlatformScreenCapturePermission {
        granted,
        requestable: false,
        previously_granted: granted,
        requested: false,
        process_path: std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string()),
        error: None,
    }
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_settings_hint() -> &'static str {
    "System Settings > Privacy & Security > Screen & System Audio Recording"
}

#[cfg(target_os = "windows")]
fn platform_screen_capture_settings_hint() -> &'static str {
    "Windows screen capture has no prompt; the interactive user session and selected target are checked when capture starts."
}

#[cfg(target_os = "linux")]
fn platform_screen_capture_settings_hint() -> &'static str {
    match current_linux_screen_capture_environment() {
        LinuxScreenCaptureEnvironment::X11Ready => {
            "X11 screen capture has no prompt; selected-target availability is checked when capture starts."
        }
        LinuxScreenCaptureEnvironment::WaylandPortalRequired => {
            "Wayland RemoteDesktop/ScreenCast portal capture is not implemented; use a supported X11 session."
        }
        LinuxScreenCaptureEnvironment::NoDisplay => {
            "No X11 DISPLAY is available for the EasyNet daemon."
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_screen_capture_settings_hint() -> &'static str {
    "Screen capture is unsupported on this platform."
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_backend() -> &'static str {
    "macos_screencapturekit"
}

#[cfg(target_os = "windows")]
fn platform_screen_capture_backend() -> &'static str {
    "windows_xcap"
}

#[cfg(target_os = "linux")]
fn platform_screen_capture_backend() -> &'static str {
    "linux_x11_xcap"
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_screen_capture_backend() -> &'static str {
    "unsupported"
}

fn platform_screen_capture_unavailable_reason(granted: bool) -> Option<&'static str> {
    if granted {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        return Some(match current_linux_screen_capture_environment() {
            LinuxScreenCaptureEnvironment::X11Ready => "linux_screen_capture_unavailable",
            LinuxScreenCaptureEnvironment::WaylandPortalRequired => {
                "linux_wayland_portal_screen_capture_not_implemented"
            }
            LinuxScreenCaptureEnvironment::NoDisplay => "linux_x11_display_unavailable",
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        Some("screen_capture_permission_denied")
    }
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxScreenCaptureEnvironment {
    X11Ready,
    WaylandPortalRequired,
    NoDisplay,
}

#[cfg(any(target_os = "linux", test))]
fn linux_screen_capture_environment(
    wayland_display_present: bool,
    x11_display_present: bool,
) -> LinuxScreenCaptureEnvironment {
    if wayland_display_present {
        LinuxScreenCaptureEnvironment::WaylandPortalRequired
    } else if x11_display_present {
        LinuxScreenCaptureEnvironment::X11Ready
    } else {
        LinuxScreenCaptureEnvironment::NoDisplay
    }
}

#[cfg(target_os = "linux")]
fn current_linux_screen_capture_environment() -> LinuxScreenCaptureEnvironment {
    linux_screen_capture_environment(
        environment_variable_present("WAYLAND_DISPLAY"),
        environment_variable_present("DISPLAY"),
    )
}

#[cfg(target_os = "linux")]
fn environment_variable_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn host_local_permission_subject_contract_projects_explicit_policy() {
        let contract = host_local_permission_subject_contract();
        assert_eq!(
            contract["subject_contract_ura"],
            json!(REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA)
        );
        assert_eq!(
            contract["allowed_subjects"],
            json!([
                "caller_user_self",
                "descriptor_bound_invoke_resource",
                "local_system_loopback"
            ])
        );
        assert_eq!(contract["target_resource_subjects_allowed"], json!(false));
        assert_eq!(
            contract["target_resource_rejection_reason"],
            json!("host_local_permission_probe")
        );
    }

    #[test]
    fn screen_capture_permission_status_reports_host_local_subject_contract() {
        let response = screen_capture_permission_status();
        assert_eq!(
            response["subject_contract"]["subject_contract_ura"],
            json!(REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA)
        );
        assert_eq!(
            response["subject_contract"]["target_resource_subjects_allowed"],
            json!(false)
        );
        assert!(response["capture_backend"].is_string());
        assert!(response["input_permission"]["backend"].is_string());
        assert!(response["input_permission"]["requestable"].is_boolean());
    }

    #[test]
    fn linux_screen_capture_environment_fails_closed_for_wayland() {
        assert_eq!(
            linux_screen_capture_environment(true, true),
            LinuxScreenCaptureEnvironment::WaylandPortalRequired
        );
        assert_eq!(
            linux_screen_capture_environment(true, false),
            LinuxScreenCaptureEnvironment::WaylandPortalRequired
        );
    }

    #[test]
    fn linux_screen_capture_environment_requires_an_x11_display() {
        assert_eq!(
            linux_screen_capture_environment(false, true),
            LinuxScreenCaptureEnvironment::X11Ready
        );
        assert_eq!(
            linux_screen_capture_environment(false, false),
            LinuxScreenCaptureEnvironment::NoDisplay
        );
    }
}
