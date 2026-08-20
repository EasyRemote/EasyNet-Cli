// EasyNet CLI — remote desktop request-permission handler
// =======================================================

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_REQUEST_PERMISSION;
use crate::daemon::plugins::remote_desktop::permissions::{
    ensure_permission_probe_access, request_screen_capture_permission,
};

/// Handle `remote_desktop.request_permission`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    ensure_permission_probe_access(ABILITY_REQUEST_PERMISSION, &env, &args)?;
    Ok(request_screen_capture_permission())
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::REASON_INVALID_ARGUMENT;

    #[test]
    fn request_permission_rejects_device_stream_resource_subject_before_os_prompt() {
        let err = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/resource/device.mac-1/streams/window.7",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_INVALID_ARGUMENT));
        assert!(err.to_string().contains("MUST NOT be scoped"));
    }

    #[test]
    fn request_permission_rejects_target_subject_in_args_before_os_prompt() {
        let err = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/user/tester",
            ),
            json!({
                "subject": "easynet:///r/acme/resource/device.mac-1/streams/application.com.example"
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_INVALID_ARGUMENT));
    }
}
