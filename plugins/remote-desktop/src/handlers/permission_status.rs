// EasyNet CLI — remote desktop permission-status handler
// ======================================================

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_PERMISSION_STATUS;
use crate::daemon::plugins::remote_desktop::permissions::{
    ensure_permission_probe_access, screen_capture_permission_status,
};

/// Handle `remote_desktop.permission_status`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    ensure_permission_probe_access(ABILITY_PERMISSION_STATUS, &env, &args)?;
    Ok(screen_capture_permission_status())
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::REASON_INVALID_ARGUMENT;

    #[test]
    fn permission_probe_rejects_subject_scoped_invocation() {
        let err = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/resource/display.1",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_INVALID_ARGUMENT));
    }

    #[test]
    fn permission_probe_accepts_authenticated_user_self_subject() {
        let response = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/user/tester",
            ),
            json!({}),
        )
        .unwrap();
        assert!(response.get("granted").is_some());
    }

    #[test]
    fn permission_probe_rejects_non_caller_user_subject() {
        let err = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/user/dev",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_INVALID_ARGUMENT));
        assert!(err.to_string().contains("subject must match"));
    }

    #[test]
    fn permission_probe_rejects_device_subject_before_defaulting() {
        let err = handle(
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/tester",
                "easynet:///r/acme/device/dev-1",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_INVALID_ARGUMENT));
        assert!(err.to_string().contains("caller-owned User subject"));
    }

    #[test]
    fn permission_probe_accepts_local_system_loopback_subject() {
        let response = handle(
            EnvelopeContext::for_test(
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            ),
            json!({}),
        )
        .unwrap();
        assert!(response.get("granted").is_some());
    }
}
