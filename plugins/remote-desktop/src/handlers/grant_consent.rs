// EasyNet CLI — remote desktop consent grant handler
// ==================================================
//
// File: plugins/remote-desktop/src/handlers/grant_consent.rs
// Description: Mint an auditable local-user consent invocation for one remote
//              desktop resource. The terminal receipt of this invocation is
//              consumed as causal_context by remote_desktop.create_session.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::consent_registry::CONSENT_INTENT;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_GRANT_CONSENT;
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::resource::resolve_screen_resource_from_envelope;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;

/// Handle `remote_desktop.grant_consent`.
///
/// This handler deliberately does not create a session and does not replace
/// Axon receipt verification. Its single responsibility is to bind an explicit
/// local user action to the selected resource so the runtime can issue a
/// canonical terminal receipt. `create_session` then fail-closes unless that
/// receipt is present in its causal context.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_screen_resource_from_envelope(ABILITY_GRANT_CONSENT, &env, &args)?;
    let intent = require_str(&args, "intent", ABILITY_GRANT_CONSENT)?;
    if intent != CONSENT_INTENT {
        return Err(RemoteDesktopError::InvalidArgument {
            ability: ABILITY_GRANT_CONSENT,
            detail: format!("unsupported consent intent {intent:?}"),
        }
        .into());
    }
    let input_control = optional_bool(&args, "input_control", ABILITY_GRANT_CONSENT)?;
    let issued = plugin.consent_registry().issue_with_grants(
        env.caller(),
        &entry.resource_ura,
        intent,
        input_control,
    )?;
    Ok(json!({
        "consent": "granted",
        "intent": intent,
        "policy": "local_user_consent",
        "grant_scope": {
            "media": true,
            "input_control": input_control,
        },
        "approval_actor_ura": env.caller(),
        "subject_ura": entry.resource_ura,
        "subject_type": entry.kind.as_str(),
        "subject_display_name": entry.display_name,
        "consent_ticket": issued.ticket,
        "consent_expires_at_ms": issued.expires_at_ms,
    }))
}

fn optional_bool(args: &Value, key: &'static str, ability: &'static str) -> anyhow::Result<bool> {
    match args.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(RemoteDesktopError::InvalidArgument {
            ability,
            detail: format!("{key} must be a boolean"),
        }
        .into()),
    }
}
