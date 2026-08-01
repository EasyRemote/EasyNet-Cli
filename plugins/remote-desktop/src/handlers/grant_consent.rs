// EasyNet CLI — remote desktop consent grant handler
// ==================================================
//
// File: plugins/remote-desktop/src/handlers/grant_consent.rs
// Description: Mint an auditable local-user consent invocation for one remote
//              desktop resource. The terminal receipt of this invocation is
//              consumed as causal_context by remote_desktop.create_session.

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_GRANT_CONSENT;
use crate::daemon::plugins::remote_desktop::resource::resolve_screen_resource_from_envelope;

/// Handle `remote_desktop.grant_consent`.
///
/// This handler deliberately does not create a session and does not replace
/// Axon receipt verification. Its single responsibility is to bind an explicit
/// local user action to the selected resource so the runtime can issue a
/// canonical terminal receipt. `create_session` then fail-closes unless that
/// receipt is present in its causal context.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_screen_resource_from_envelope(ABILITY_GRANT_CONSENT, &env, &args)?;
    Ok(json!({
        "consent": "granted",
        "policy": "local_user_consent",
        "approval_actor_ura": env.caller(),
        "subject_ura": entry.resource_ura,
        "subject_type": entry.kind.as_str(),
        "subject_display_name": entry.display_name,
    }))
}
