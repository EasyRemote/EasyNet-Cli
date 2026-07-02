// EasyNet CLI — remote desktop request-permission handler
// =======================================================

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::constants::ABILITY_REQUEST_PERMISSION;
use crate::plugins::remote_desktop::permissions::{
    ensure_permission_probe_access, request_screen_capture_permission,
};

/// Handle `remote_desktop.request_permission`.
pub(in crate::plugins::builtin::remote_desktop) fn handle(
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    ensure_permission_probe_access(ABILITY_REQUEST_PERMISSION, &env, &args)?;
    Ok(request_screen_capture_permission())
}
