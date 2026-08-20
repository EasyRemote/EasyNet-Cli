// EasyNet CLI — remote desktop session access checks
// ==================================================
//
// File: plugins/remote-desktop/src/session_access.rs
// Description: Token and resource-subject checks for remote desktop sessions.

use serde_json::Value;

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    reject_subject_in_args, require_resource_ura_subject,
};
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::errors::{RemoteDesktopError, RemoteDesktopResult};
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::session_consent::causal_context_contains_receipt;

/// Verify that a remote desktop session control-plane ability targets exactly
/// one session and carries the session bearer token.
///
/// Every session operation remains bound to the resource EntityRef captured at
/// creation. Adapters must preserve that subject instead of substituting the
/// caller identity.
pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_control_identity(
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
    ensure_session_subject_consistent(ability, env.subject(), session)?;
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
pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_resource_identity(
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
    let expected = session.creator_caller_ura();
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
    let expected = session.consent().approval_receipt();
    if !causal_context_contains_receipt(ability, Some(env.causal_context()), expected)? {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ensure_session_control_identity, ensure_session_resource_identity};
    use crate::daemon::plugins::remote_desktop::constants::ABILITY_SHOW_SESSION;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::test_support::{env_for, test_session_init};

    #[test]
    fn session_control_subject_contract_is_original_resource_ura_not_session_ura() {
        let resource_ura = "easynet:///r/acme/resource/device.01DEV/streams/display.contract";
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-subject-contract",
            resource_ura,
            vec!["webrtc".to_string()],
        ));
        let args = json!({
            "session_id": "rd-subject-contract",
            "session_token": "token",
        });

        ensure_session_control_identity(
            ABILITY_SHOW_SESSION,
            &env_for(resource_ura),
            &args,
            &session,
        )
        .expect("resource subject is the remote desktop session control contract");
        ensure_session_resource_identity(
            ABILITY_SHOW_SESSION,
            &env_for(resource_ura),
            &args,
            &session,
        )
        .expect("resource data-plane access uses the same selected resource subject");

        let session_ura = "easynet:///r/acme/resource/remote-desktop-session/rd-subject-contract";
        let err = ensure_session_control_identity(
            ABILITY_SHOW_SESSION,
            &env_for(session_ura),
            &args,
            &session,
        )
        .expect_err("session URA must not replace the selected resource subject");
        assert!(err.to_string().contains("does not match session subject"));
    }

    #[test]
    fn session_control_rejects_subject_in_args_even_when_token_matches() {
        let resource_ura = "easynet:///r/acme/resource/device.01DEV/streams/display.args-subject";
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-args-subject",
            resource_ura,
            vec!["webrtc".to_string()],
        ));
        let args = json!({
            "session_id": "rd-args-subject",
            "session_token": "token",
            "subject": resource_ura,
        });

        let err = ensure_session_control_identity(
            ABILITY_SHOW_SESSION,
            &env_for(resource_ura),
            &args,
            &session,
        )
        .expect_err("Invocation.subject must not be duplicated in ability args");

        assert!(err.to_string().contains("subject_in_args"));
    }
}
