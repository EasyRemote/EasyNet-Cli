// EasyNet CLI — error composition helpers
// =======================================
//
// File: src/support/errors.rs
// Description: Dependency-free helpers for composing one operator-facing
//              error out of a primary failure and a best-effort cleanup
//              that may itself fail.
//
// This lives in `support` (the leaf plumbing layer) because both the
// `runtime::agents` rollback paths and the `persistence` store-write
// paths need it, and `support` is the only module both are allowed to
// depend on. It takes `anyhow::Error` values in and out — no upward
// dependency on `persistence`, `registry`, or `runtime`.

/// Fold a best-effort cleanup outcome into a primary error.
///
/// Transactional rollback paths run a primary fallible operation and, on
/// failure, attempt a compensating cleanup that can itself fail. The
/// operator needs both facts in one error: what failed, and whether the
/// system was left in a clean state. When `cleanup` succeeded, the
/// primary error is returned unchanged; when it failed, the cleanup
/// failure is appended so the audit trail records that the compensation
/// did not complete.
pub(crate) fn append_cleanup_error(
    primary: anyhow::Error,
    cleanup: anyhow::Result<()>,
    cleanup_action: &'static str,
) -> anyhow::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup_err) => {
            anyhow::anyhow!("{primary}; additionally failed to {cleanup_action}: {cleanup_err}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_cleanup_returns_primary_unchanged() {
        let primary = anyhow::anyhow!("primary failure");
        let folded = append_cleanup_error(primary, Ok(()), "roll back store");
        assert_eq!(folded.to_string(), "primary failure");
    }

    #[test]
    fn failed_cleanup_is_appended_to_primary() {
        let primary = anyhow::anyhow!("primary failure");
        let cleanup = Err(anyhow::anyhow!("disk full"));
        let folded = append_cleanup_error(primary, cleanup, "roll back store");
        assert_eq!(
            folded.to_string(),
            "primary failure; additionally failed to roll back store: disk full"
        );
    }
}
