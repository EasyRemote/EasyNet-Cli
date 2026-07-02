// EasyNet CLI — remote desktop session access checks
// ==================================================
//
// File: src/plugins/builtin/remote_desktop/session_access.rs
// Description: Token and resource-subject checks for remote desktop sessions.

use serde_json::Value;

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    reject_subject_in_args, require_resource_ura_subject,
};
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::plugins::remote_desktop::errors::{RemoteDesktopError, RemoteDesktopResult};
use crate::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::plugins::remote_desktop::session_consent::causal_context_contains_receipt;

/// Verify that a remote desktop session control-plane ability targets exactly
/// one session and carries the session bearer token.
///
/// Signaling and lifecycle calls are session-scoped, not media-resource
/// operations. Their envelope subject may be the caller/user subject supplied
/// by the Axon sidecar, so they must not require it to equal the captured
/// resource URA. The resource binding remains inside the session row created
/// by `create_session`.
pub(in crate::plugins::builtin::remote_desktop) fn ensure_session_control_identity(
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &RemoteDesktopSession,
) -> RemoteDesktopResult<()> {
    reject_subject_in_args(ability, args).map_err(|source| {
        RemoteDesktopError::InvalidArgument {
            ability,
            detail: source.to_string(),
        }
    })?;
    let token = require_session_token(ability, args)?;
    if !session.matches_session_token(token) {
        return Err(RemoteDesktopError::SessionTokenMismatch {
            ability,
            session_id: session.session_id().to_string(),
        });
    }
    ensure_session_caller_consistent(ability, env, session)?;
    ensure_session_consent_receipt_consistent(ability, env, session)?;
    Ok(())
}

/// Verify that a resource data-plane ability targets exactly one session and
/// exactly that session's resource subject.
///
/// This is NOT caller authentication. Admission has already authenticated the
/// envelope caller before plugin dispatch. This module enforces the product
/// invariant that data-plane capture/control operations are bound to the
/// resource URA captured by `create_session`.
pub(in crate::plugins::builtin::remote_desktop) fn ensure_session_resource_identity(
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &RemoteDesktopSession,
) -> RemoteDesktopResult<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    let subject = require_resource_ura_subject(
        ability,
        Some(env.subject()),
        "remote desktop data-plane resource",
    )
    .map_err(|_| RemoteDesktopError::InvalidArgument {
        ability,
        detail: "envelope subject is required for resource data-plane access".to_string(),
    })?;
    ensure_session_subject_consistent(ability, subject, session)?;
    Ok(())
}

fn ensure_session_caller_consistent(
    ability: &'static str,
    env: &EnvelopeContext,
    session: &RemoteDesktopSession,
) -> RemoteDesktopResult<()> {
    let Some(expected) = session.creator_caller_ura() else {
        return Ok(());
    };
    let actual = env.caller();
    if expected != actual {
        return Err(RemoteDesktopError::SessionCallerMismatch {
            ability,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn ensure_session_consent_receipt_consistent(
    ability: &'static str,
    env: &EnvelopeContext,
    session: &RemoteDesktopSession,
) -> RemoteDesktopResult<()> {
    let Some(expected) = session.consent().approval_receipt() else {
        return Ok(());
    };
    if !causal_context_contains_receipt(Some(env.causal_context()), expected) {
        return Err(RemoteDesktopError::ConsentReceiptMismatch {
            ability,
            expected: expected.receipt_ura().to_string(),
        });
    }
    Ok(())
}

fn require_session_token<'a>(
    ability: &'static str,
    args: &'a Value,
) -> RemoteDesktopResult<&'a str> {
    args.get("session_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(RemoteDesktopError::SessionTokenRequired { ability })
}

fn ensure_session_subject_consistent(
    ability: &'static str,
    subject: &str,
    session: &RemoteDesktopSession,
) -> RemoteDesktopResult<()> {
    if session.subject_ura() != subject {
        return Err(RemoteDesktopError::InvalidArgument {
            ability,
            detail: format!(
                "subject {subject:?} does not match session subject {:?}",
                session.subject_ura()
            ),
        });
    }
    Ok(())
}
