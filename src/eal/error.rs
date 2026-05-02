// EasyNet CLI — EAL Error Type
// ============================
//
// File: src/eal/error.rs
// Description: Structured error returned by the EAL `StepDispatcher`
//              trait and the dispatch helpers it depends on. Mirrors
//              `mcp::error::McpError` in shape and intent — see that
//              module's docstring for the full rationale on
//              error-code-driven dispatch.
//
// Why a separate type rather than reusing `McpError`?
//   The two error sets cover different boundaries:
//     - `McpError` is the wire-level contract surfaced to MCP clients
//       (Claude Code / Codex). Renaming a variant is a breaking change
//       for every consuming agent.
//     - `EalError` is an *internal* error type used by the EAL
//       interpreter to categorise dispatch failures. It is converted
//       to a display string (via `Display`) when stored in
//       `StepExecResult::Error.message` and on disk in `StepTrace.error`.
//   Coupling the two would mean any internal refactor risks bleeding
//   into the MCP contract. Keeping them separate (with deliberately
//   identical four-variant taxonomy) lets each evolve at its own pace.
//
// Convention:
//   - `Validation`       — caller passed a malformed / out-of-spec
//     payload (e.g. dispatcher cannot handle this `IrTarget`
//     variant). Retries will not help; the bug is in the EAL source
//     or the planner.
//   - `NotFound`         — named resource (registry agent, deployed
//     ability) does not exist. Retries with the same name will not
//     help.
//   - `Unavailable`      — known-good resource cannot serve right now
//     (bridge transport failure, device offline, peer closed the
//     stream). Retries after recovery may succeed.
//   - `DeadlineExceeded` — the step did not complete within its time
//     budget. Separate from `Unavailable` so the interpreter's retry
//     machinery, and any audit-log consumer, can distinguish peer
//     overload from peer unreachability. A timeout warrants a longer
//     backoff than a transport error.
//   - `Internal`         — unexpected condition; probably a bug to
//     report.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Structured error returned by `StepDispatcher` and helpers.
///
/// Constructed by dispatcher implementations; rendered to a display
/// string at the boundary into `StepExecResult::Error.message` (which
/// owns the on-disk trace shape — keeping the conversion here means the
/// trace shape is unchanged by this typing migration).
#[derive(Debug, Clone)]
pub enum EalError {
    /// EAL caller asked for something the dispatcher cannot satisfy by
    /// design — e.g. a Device-only dispatcher receiving an `IrTarget::Agent`.
    Validation(String),
    /// Named resource (registry agent, deployed ability) does not exist.
    NotFound(String),
    /// Resource exists but cannot serve the request right now (bridge
    /// transport failure, device offline, peer closed the stream).
    /// Retry after recovery may succeed.
    Unavailable(String),
    /// The step did not complete within its time budget. Distinct from
    /// `Unavailable`: timeouts imply peer overload more often than
    /// unreachability, so the retry policy should lengthen its backoff
    /// rather than apply the same exponential curve.
    DeadlineExceeded(String),
    /// Unexpected condition; a bug to investigate. Retries are not
    /// informative.
    Internal(String),
}

impl EalError {
    /// Stable machine-readable code. Mirrors `McpError::error_code` so
    /// downstream consumers that branch on the MCP envelope see the
    /// same vocabulary regardless of which error layer produced it.
    pub fn error_code(&self) -> &'static str {
        match self {
            EalError::Validation(_) => "validation_error",
            EalError::NotFound(_) => "not_found",
            EalError::Unavailable(_) => "unavailable",
            EalError::DeadlineExceeded(_) => "deadline_exceeded",
            EalError::Internal(_) => "internal_error",
        }
    }

    /// Human-readable message. For display only — callers must branch
    /// on `error_code`, not message wording.
    pub fn message(&self) -> &str {
        match self {
            EalError::Validation(m)
            | EalError::NotFound(m)
            | EalError::Unavailable(m)
            | EalError::DeadlineExceeded(m)
            | EalError::Internal(m) => m,
        }
    }

    /// Classify a typed [`AxonError`] from the bridge SDK into the
    /// matching EAL category.
    ///
    /// # Why typed classification (not string sniffing)
    ///
    /// An earlier version categorised bridge errors by `to_string()`
    /// then `contains("timeout")` — a last resort from the era when
    /// the SDK surfaced `Result<_, String>`. The SDK now returns
    /// [`AxonError`], a discriminated union whose variants already
    /// carry the semantic we need. String sniffing on top of that is
    /// doubly wrong:
    ///
    ///   1. It re-derives information the SDK encoded by fiat (the
    ///      variant itself). If the SDK renames its `Display` output,
    ///      classification flips silently — a spec change the compiler
    ///      cannot see.
    ///   2. It forces the SDK's prose into a de-facto contract,
    ///      coupling two crates through free-form English.
    ///
    /// The mapping below is defined **once**, by variant, with no
    /// regex on message content. When a new `AxonError` variant is
    /// added, the match must be updated — which is the point.
    ///
    /// # Mapping rationale
    ///
    /// - [`AxonError::Validation`] → [`EalError::Validation`]: both
    ///   mean "caller-supplied input is wrong; retry won't help".
    /// - [`AxonError::NotInstalled`] / [`AxonError::NotActivated`] →
    ///   [`EalError::NotFound`]: the named ability does not exist (or
    ///   is not yet reachable as one). Both are identifier-mismatch
    ///   failures from the caller's point of view — retrying with the
    ///   same `install_id` changes nothing.
    /// - [`AxonError::PolicyDenied`] → [`EalError::Validation`]:
    ///   policy rejection is a contract violation (tenant, quota,
    ///   mode). Retrying the same call produces the same rejection;
    ///   categorising it as `Unavailable` would mislead the retry
    ///   loop into hammering a forbidden operation.
    /// - [`AxonError::DeadlineExceeded`] →
    ///   [`EalError::DeadlineExceeded`]: the SDK tells us the call
    ///   exceeded its budget. Retry with a longer backoff; a tighter
    ///   retry can compound peer overload.
    /// - [`AxonError::Bridge`] / [`AxonError::Stream`] /
    ///   [`AxonError::Invocation`] / [`AxonError::Mcp`] →
    ///   [`EalError::Unavailable`]: transport / peer-side failures
    ///   that may succeed on retry after recovery. `Invocation` is a
    ///   *remote* execution failure (the SDK already split local
    ///   validation out as `Validation`), so it is a peer-availability
    ///   signal, not a caller-bug signal.
    /// - [`AxonError::Io`] → [`EalError::Unavailable`]: file / socket
    ///   I/O errors usually reflect transient resource pressure. The
    ///   exception — filesystem corruption — is rare enough that the
    ///   caller's retry will simply surface it again.
    /// - [`AxonError::SymbolNotFound`] / [`AxonError::Json`] /
    ///   [`AxonError::PartialSuccess`] → [`EalError::Internal`]:
    ///   `SymbolNotFound` means the native library is wrong shape
    ///   (build-time bug), `Json` means the SDK emitted garbage, and
    ///   `PartialSuccess` at a single-step boundary is a contract
    ///   violation (steps are scalar — partial is not a valid outcome
    ///   here).
    #[must_use]
    pub fn from_axon_error(err: easynet_axon::AxonError) -> Self {
        use easynet_axon::AxonError as A;
        // Render once; the typed match below owns the categorisation,
        // but each variant still needs its prose for the operator-
        // facing message. `to_string()` uses `thiserror`'s `#[error]`
        // template, which is the SDK's chosen human wording.
        let msg = err.to_string();
        match err {
            A::Validation(_) | A::PolicyDenied(_) => EalError::Validation(msg),
            A::NotInstalled(_) | A::NotActivated(_) => EalError::NotFound(msg),
            A::DeadlineExceeded(_) => EalError::DeadlineExceeded(msg),
            A::Bridge(_) | A::Stream(_) | A::Invocation(_) | A::Mcp(_) | A::Io(_) => {
                EalError::Unavailable(msg)
            }
            A::SymbolNotFound(_) | A::Json(_) | A::PartialSuccess { .. } => EalError::Internal(msg),
        }
    }
}

impl From<easynet_axon::AxonError> for EalError {
    /// Typed conversion from the bridge SDK's error — see
    /// [`EalError::from_axon_error`] for the mapping rationale.
    ///
    /// Unlike the deliberately absent `From<String>` impl (which would
    /// silently default every error to `Unavailable`), this
    /// conversion is non-lossy: each [`AxonError`] variant has a
    /// specific, reviewed target category. A bare `?` against a
    /// `Result<_, AxonError>` therefore classifies correctly.
    fn from(err: easynet_axon::AxonError) -> Self {
        EalError::from_axon_error(err)
    }
}

impl std::fmt::Display for EalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format includes the code so the on-disk `StepTrace.error`
        // string carries the categorisation forward — operators
        // grepping a trace can see "unavailable: bridge connect..."
        // without needing the typed error available.
        write!(f, "{}: {}", self.error_code(), self.message())
    }
}

impl std::error::Error for EalError {}

// Note: we deliberately do NOT implement `From<String> for EalError`.
//
// Early drafts had `impl From<String> for EalError { Unavailable(s) }`
// so dispatchers could write `?` against `Result<_, String>`. That
// shortcut made every un-annotated `?` silently categorise as
// `Unavailable` — which is wrong for roughly half the real errors
// (malformed input → Validation, missing registry entry → NotFound,
// serde failure → Internal). A default-by-From is a *deception* at
// pre-release scale: it turns "I didn't think about the category"
// into "I claimed it was retryable." The fix is to force every
// call site to pick a category with an explicit constructor.
//
// If you want the old ergonomics back, add a helper like
// `EalError::unavailable_from_string(s)` at the call site — verbose
// by design.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_and_unique() {
        assert_eq!(
            EalError::Validation(String::new()).error_code(),
            "validation_error"
        );
        assert_eq!(EalError::NotFound(String::new()).error_code(), "not_found");
        assert_eq!(
            EalError::Unavailable(String::new()).error_code(),
            "unavailable"
        );
        assert_eq!(
            EalError::DeadlineExceeded(String::new()).error_code(),
            "deadline_exceeded"
        );
        assert_eq!(
            EalError::Internal(String::new()).error_code(),
            "internal_error"
        );

        let codes = [
            EalError::Validation(String::new()).error_code(),
            EalError::NotFound(String::new()).error_code(),
            EalError::Unavailable(String::new()).error_code(),
            EalError::DeadlineExceeded(String::new()).error_code(),
            EalError::Internal(String::new()).error_code(),
        ];
        let uniq: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(uniq.len(), codes.len(), "error_code() must be 1-to-1");
    }

    #[test]
    fn display_includes_code_and_message() {
        let err = EalError::NotFound("agent 'claude' missing".into());
        let s = format!("{err}");
        assert!(s.starts_with("not_found:"));
        assert!(s.contains("agent 'claude' missing"));
    }

    // ── AxonError → EalError mapping ────────────────────────────────────────
    //
    // Each case below pins the contract documented on
    // `from_axon_error` so that changing the SDK's variant set or
    // renaming a display template cannot silently re-route a category.
    // If the SDK grows a new variant, the `match` in
    // `from_axon_error` won't compile — that is the tripwire.

    use easynet_axon::AxonError as Axon;

    #[test]
    fn axon_validation_and_policy_map_to_ealerror_validation() {
        for axon in [
            Axon::Validation("bad field".into()),
            Axon::PolicyDenied("tenant quota".into()),
        ] {
            let original = format!("{axon}");
            let mapped = EalError::from_axon_error(axon);
            assert_eq!(mapped.error_code(), "validation_error");
            assert_eq!(
                mapped.message(),
                original,
                "message must preserve the SDK's Display output verbatim"
            );
        }
    }

    #[test]
    fn axon_lifecycle_state_maps_to_not_found() {
        for axon in [
            Axon::NotInstalled("install-123".into()),
            Axon::NotActivated("install-456".into()),
        ] {
            let mapped = EalError::from_axon_error(axon);
            assert_eq!(mapped.error_code(), "not_found");
        }
    }

    #[test]
    fn axon_deadline_maps_to_deadline_exceeded() {
        let mapped =
            EalError::from_axon_error(Axon::DeadlineExceeded("stream chunk timeout".into()));
        assert_eq!(mapped.error_code(), "deadline_exceeded");
        // Prose is preserved so operator-facing traces carry the cause.
        assert!(mapped.message().contains("stream chunk timeout"));
    }

    #[test]
    fn axon_transport_and_execution_map_to_unavailable() {
        for axon in [
            Axon::Bridge("connect refused".into()),
            Axon::Stream("peer closed".into()),
            Axon::Invocation("remote panic".into()),
            Axon::Mcp("protocol drift".into()),
            Axon::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ] {
            let code = EalError::from_axon_error(axon).error_code();
            assert_eq!(code, "unavailable", "got {code}");
        }
    }

    #[test]
    fn axon_internal_defects_map_to_internal() {
        // `SymbolNotFound` means the native library is the wrong
        // shape — a build/link bug, not a runtime-retryable failure.
        let sym = EalError::from_axon_error(Axon::SymbolNotFound("ax_invoke".into()));
        assert_eq!(sym.error_code(), "internal_error");

        // A `PartialSuccess` at a *single-step* boundary is a contract
        // violation — steps are scalar, so partial is not a valid
        // outcome at this layer.
        let partial = EalError::from_axon_error(Axon::PartialSuccess {
            succeeded: 2,
            failed: 1,
            details: serde_json::json!({}),
        });
        assert_eq!(partial.error_code(), "internal_error");

        // Forge a JSON error by failing to parse an invalid document;
        // `AxonError` implements `From<serde_json::Error>`.
        let json_err: easynet_axon::AxonError = serde_json::from_str::<serde_json::Value>("{")
            .unwrap_err()
            .into();
        assert_eq!(
            EalError::from_axon_error(json_err).error_code(),
            "internal_error"
        );
    }

    /// `?` against `Result<_, AxonError>` must route through the typed
    /// mapping, not default to `Unavailable`. Regression guard for the
    /// `From<AxonError>` impl next to `from_axon_error`.
    #[test]
    fn question_mark_uses_typed_axon_conversion() {
        fn lift() -> Result<(), EalError> {
            // NotInstalled is the most distinctive case: it must route
            // to `not_found`, not `unavailable`.
            Err::<(), _>(Axon::NotInstalled("install-x".into()))?;
            Ok(())
        }
        let err = lift().unwrap_err();
        assert_eq!(
            err.error_code(),
            "not_found",
            "`?` lost the typed categorisation — check `From<AxonError>`"
        );
    }

    /// Regression doc: `From<String> for EalError` MUST NOT exist.
    ///
    /// Stable Rust cannot assert "trait not implemented" at compile
    /// time without `#![feature(negative_impls)]`, so this test
    /// documents the invariant in prose and pins the positive
    /// property — that each variant retains its own `error_code` —
    /// so a naïve "let me just re-add From" refactor has to face a
    /// reviewer's grep of this file. The tripwire is the comment
    /// next to the removed `From<String>` impl above.
    #[test]
    fn from_string_is_intentionally_not_implemented() {
        let unavailable = EalError::Unavailable("x".into());
        let validation = EalError::Validation("x".into());
        assert_ne!(unavailable.error_code(), validation.error_code());
    }
}
