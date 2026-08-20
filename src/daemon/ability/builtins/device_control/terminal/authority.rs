//! Shared RFC-014 session-authority checks for terminal follow-ups.

use crate::daemon::ability::dispatch::EnvelopeContext;

/// Verify the binding that is common to every terminal follow-up before the
/// handler accesses the PTY table. Transport admission already verified the
/// signature and policy; this check binds that verified authority to the
/// concrete daemon session and ability being touched.
pub(crate) fn require_session_authority(
    env: &EnvelopeContext,
    session_id: &str,
    ability: &str,
) -> anyhow::Result<()> {
    let authority = env.session_authority().ok_or_else(|| {
        anyhow::anyhow!(
            "{ability}: verified session authority required before accessing session `{session_id}`"
        )
    })?;
    if authority.session_id != session_id {
        anyhow::bail!(
            "{ability}: session authority `{}` does not match session `{session_id}`",
            authority.session_id
        );
    }
    if authority.issuer_ura != env.caller() {
        anyhow::bail!("{ability}: session authority issuer does not match envelope caller");
    }
    if authority.callee_ura != env.callee() {
        anyhow::bail!("{ability}: session authority callee does not match envelope callee");
    }
    // The canonical admission descriptor has already checked
    // `allowed_actions` against the descriptor-bound action. Re-evaluating a
    // product-defined action name here would create a second authority model
    // and can disagree with the signed descriptor (for example unary
    // terminal I/O is `invoke`, not a locally invented `stream` action).
    if !authority
        .allowed_followup_abilities
        .iter()
        .any(|candidate| ability_matches(candidate, ability))
    {
        anyhow::bail!("{ability}: session authority does not allow follow-up ability");
    }
    Ok(())
}

fn ability_matches(pattern: &str, ability: &str) -> bool {
    let pattern = pattern.trim();
    pattern == ability
        || pattern
            .strip_suffix(".*")
            .is_some_and(|prefix| ability.starts_with(prefix) && ability.len() > prefix.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::dispatch::EnvelopeContext;

    #[test]
    fn wildcard_follow_up_scope_matches_terminal_ability() {
        assert!(ability_matches("terminal.*", "terminal.read"));
        assert!(!ability_matches("terminal.*", "remote_desktop.attach"));
    }

    #[test]
    fn missing_authority_is_rejected_before_session_access() {
        let env = EnvelopeContext::for_test_ability(
            "easynet:///r/test/backend/api",
            "terminal.read",
            "easynet:///r/test/device/node",
        );
        let err = require_session_authority(&env, "session-1", "terminal.read")
            .expect_err("missing authority must fail closed");
        assert!(err
            .to_string()
            .contains("verified session authority required"));
    }
}
