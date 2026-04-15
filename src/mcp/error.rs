// EasyNet CLI — MCP Error Type
// ============================
//
// File: src/mcp/error.rs
// Description: Structured error enum returned by MCP tool handlers, so
//              calling agents (Claude Code, Codex, or any programmatic
//              MCP client) can branch on a stable `error_code` rather
//              than regex-matching free-form English messages.
//
// Why this exists:
//   Before this module, every handler returned `Result<Value, String>`.
//   Tool failures reached the agent as `{"ok": false, "error": "<msg>"}`
//   and nothing else. An agent that wanted to implement "retry on
//   transient failures, give up on contract violations" had to parse
//   the message string — brittle against any wording tweak, and a
//   leaky abstraction because downstream callers now depend on
//   the *exact English phrasing* of every error.
//
//   `McpError` names the kinds of failure instead. Each variant maps
//   to one stable machine-readable code (see `error_code`) that the
//   calling agent can match on. Human-readable text lives in the `msg`
//   field for display only — callers who branch on it are on their
//   own when the wording changes.
//
// Category design:
//   - `Validation`        — caller's input is wrong (missing/malformed
//                           field, type mismatch, out-of-range value).
//                           A well-behaved agent fixes the input and
//                           retries.
//   - `NotFound`          — named resource (device, ability, agent,
//                           install) does not exist. Retrying without
//                           changing the identifier is pointless.
//   - `Unavailable`       — the resource is known but cannot serve the
//                           request right now (bridge unreachable,
//                           device offline, connection lost). Retrying
//                           after transport recovery can succeed.
//   - `DeadlineExceeded`  — the request did not complete within its
//                           timeout, but the peer may still have been
//                           reachable and working. Distinct from
//                           `Unavailable` so agents can apply a longer
//                           backoff (timeouts often indicate peer
//                           overload, and immediate retry compounds
//                           the load). If the operation is not
//                           idempotent, agents should confirm state
//                           before retrying.
//   - `Internal`          — the CLI or SDK hit an unexpected condition;
//                           probably a bug to report. Retrying is
//                           uninformative.
//
// These five cover every place the handlers used to return an
// `Err(String)` and correspond 1-to-1 with the decisions an agent
// needs to make about retry / longer-backoff / fix-and-retry /
// give-up / report.
//
// Stability:
//   The `error_code` strings are part of the public MCP contract. They
//   must not be renamed lightly — an agent that branched on `"not_found"`
//   would silently stop handling the case if we renamed it. Adding
//   *new* variants is additive and does not break existing consumers,
//   who will see the new code as an unknown and fall through to their
//   generic error branch.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

/// Structured error returned by every MCP tool handler.
#[derive(Debug, Clone)]
pub enum McpError {
    /// Caller-supplied input failed validation. Fix the input, retry.
    Validation(String),
    /// Named resource does not exist. Retrying with the same identifier
    /// will not help.
    NotFound(String),
    /// Resource exists but cannot serve the request right now (bridge
    /// down, device offline, peer closed the stream). Retry after
    /// transport recovery may succeed.
    Unavailable(String),
    /// The operation did not complete within its time budget. Distinct
    /// from `Unavailable` so agents can apply a longer backoff — a
    /// timeout often indicates peer overload, and an immediate retry
    /// compounds the load.
    DeadlineExceeded(String),
    /// Unexpected condition — probably a bug. Retry is not informative.
    Internal(String),
}

impl McpError {
    /// Stable machine-readable code. **Part of the public MCP contract
    /// — do not rename existing variants' codes.**
    pub fn error_code(&self) -> &'static str {
        match self {
            McpError::Validation(_) => "validation_error",
            McpError::NotFound(_) => "not_found",
            McpError::Unavailable(_) => "unavailable",
            McpError::DeadlineExceeded(_) => "deadline_exceeded",
            McpError::Internal(_) => "internal_error",
        }
    }

    /// Human-readable message. For display only — callers that branch
    /// on this string are on their own when the wording changes.
    pub fn message(&self) -> &str {
        match self {
            McpError::Validation(m)
            | McpError::NotFound(m)
            | McpError::Unavailable(m)
            | McpError::DeadlineExceeded(m)
            | McpError::Internal(m) => m,
        }
    }

    /// Render into the `{"ok": false, "error_code": ..., "error": ...}`
    /// envelope the provider surfaces to the MCP client.
    pub fn to_payload(&self) -> Value {
        json!({
            "ok": false,
            "error_code": self.error_code(),
            "error": self.message(),
        })
    }
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error_code(), self.message())
    }
}

impl std::error::Error for McpError {}

impl From<easynet_axon::AxonError> for McpError {
    /// Typed conversion from the bridge SDK's error.
    ///
    /// The mapping is the mirror of [`crate::eal::error::EalError::from_axon_error`]
    /// — same taxonomy, same variant-by-variant intent — so a failure
    /// observed by an MCP client and the same failure observed in an
    /// EAL mission trace categorise identically. The two error types
    /// stay separate (they cross different boundaries: MCP is the
    /// wire contract to agents, EAL is an internal category for the
    /// interpreter) but a divergence in mapping would be confusing
    /// for operators cross-referencing a trace against a client log.
    ///
    /// | [`AxonError`] variant                             | [`McpError`]         |
    /// |---------------------------------------------------|----------------------|
    /// | `Validation`, `PolicyDenied`                      | `Validation`         |
    /// | `NotInstalled`, `NotActivated`                    | `NotFound`           |
    /// | `DeadlineExceeded`                                | `DeadlineExceeded`   |
    /// | `Bridge`, `Stream`, `Invocation`, `Mcp`, `Io`     | `Unavailable`        |
    /// | `SymbolNotFound`, `Json`, `PartialSuccess`        | `Internal`           |
    ///
    /// `?` against a `Result<_, AxonError>` routes through this
    /// conversion automatically. A bare `map_err(McpError::from)`
    /// achieves the same at an explicit point in a chain.
    ///
    /// [`AxonError`]: easynet_axon::AxonError
    fn from(err: easynet_axon::AxonError) -> Self {
        use easynet_axon::AxonError as A;
        let msg = err.to_string();
        match err {
            A::Validation(_) | A::PolicyDenied(_) => McpError::Validation(msg),
            A::NotInstalled(_) | A::NotActivated(_) => McpError::NotFound(msg),
            A::DeadlineExceeded(_) => McpError::DeadlineExceeded(msg),
            A::Bridge(_)
            | A::Stream(_)
            | A::Invocation(_)
            | A::Mcp(_)
            | A::Io(_) => McpError::Unavailable(msg),
            A::SymbolNotFound(_) | A::Json(_) | A::PartialSuccess { .. } => {
                McpError::Internal(msg)
            }
        }
    }
}

// Note: we deliberately do NOT implement `From<String> for McpError`.
//
// An earlier draft had `impl From<String> { Unavailable(s) }` so
// handlers could write `let v = br.do_thing()?;` against a
// `Result<_, String>`. That shortcut was a *deception*: every
// unannotated `?` silently classified the error as retryable, even
// when the underlying problem was a schema violation (Validation) or
// a missing resource (NotFound). The on-the-wire contract
// (`error_code`) is only useful if it accurately describes the
// failure, and blanket `Unavailable` undermined that.
//
// Handlers that need to lift an SDK error into `McpError` use the
// `unavailable(e)` helper in `handlers.rs`, which names the category
// at the site — leaving room for `Validation(...)` / `NotFound(...)`
// where that is the truth.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_and_unique() {
        // Pin the on-the-wire codes. Renaming any of these is a breaking
        // change for every agent that branched on them.
        assert_eq!(
            McpError::Validation(String::new()).error_code(),
            "validation_error"
        );
        assert_eq!(McpError::NotFound(String::new()).error_code(), "not_found");
        assert_eq!(
            McpError::Unavailable(String::new()).error_code(),
            "unavailable"
        );
        assert_eq!(
            McpError::DeadlineExceeded(String::new()).error_code(),
            "deadline_exceeded"
        );
        assert_eq!(
            McpError::Internal(String::new()).error_code(),
            "internal_error"
        );

        // Uniqueness: no two variants may share a code.
        let codes = [
            McpError::Validation(String::new()).error_code(),
            McpError::NotFound(String::new()).error_code(),
            McpError::Unavailable(String::new()).error_code(),
            McpError::DeadlineExceeded(String::new()).error_code(),
            McpError::Internal(String::new()).error_code(),
        ];
        let uniq: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(uniq.len(), codes.len(), "error_code() must be 1-to-1");
    }

    #[test]
    fn payload_envelope_shape_is_stable() {
        let err = McpError::NotFound("device 'node-x' not registered".into());
        let payload = err.to_payload();
        assert_eq!(payload["ok"], json!(false));
        assert_eq!(payload["error_code"], json!("not_found"));
        assert_eq!(payload["error"], json!("device 'node-x' not registered"));
        // No extra fields that agents might start depending on by accident.
        assert_eq!(payload.as_object().unwrap().len(), 3);
    }

    /// Regression: `From<String> for McpError` MUST NOT exist. See the
    /// rationale comment next to the `impl std::error::Error` block
    /// above. If a future refactor re-adds the impl, `?` against a
    /// `Result<_, String>` will silently categorise every error as
    /// `Unavailable`, re-introducing the deception this test pins.
    ///
    /// The absence of the impl is enforced by the compiler (any call
    /// site that tries `s.into()` on a String to McpError won't build);
    /// this test documents the invariant so a reviewer grepping for
    /// `From<String>` can see the intent.
    #[test]
    fn from_string_is_intentionally_not_implemented() {
        let unavailable = McpError::Unavailable("x".into());
        let validation = McpError::Validation("x".into());
        assert_ne!(unavailable.error_code(), validation.error_code());
    }

    #[test]
    fn display_includes_code_and_message() {
        let err = McpError::Validation("field `x` missing".into());
        let s = format!("{err}");
        assert!(s.contains("validation_error"));
        assert!(s.contains("field `x` missing"));
    }

    // ── AxonError → McpError mapping ────────────────────────────────────────
    //
    // These tests pin the contract documented on `impl From<AxonError>`.
    // They must stay in lockstep with `EalError::from_axon_error`'s
    // tests — any divergence means an operator sees the same bridge
    // failure categorised differently depending on which seam they're
    // looking at, which defeats the whole point of a shared taxonomy.

    use easynet_axon::AxonError as Axon;

    #[test]
    fn axon_validation_and_policy_map_to_mcp_validation() {
        for axon in [
            Axon::Validation("bad field".into()),
            Axon::PolicyDenied("tenant quota".into()),
        ] {
            let mapped: McpError = axon.into();
            assert_eq!(mapped.error_code(), "validation_error");
        }
    }

    #[test]
    fn axon_lifecycle_state_maps_to_not_found() {
        for axon in [
            Axon::NotInstalled("install-123".into()),
            Axon::NotActivated("install-456".into()),
        ] {
            let mapped: McpError = axon.into();
            assert_eq!(mapped.error_code(), "not_found");
        }
    }

    #[test]
    fn axon_deadline_maps_to_deadline_exceeded() {
        let mapped: McpError = Axon::DeadlineExceeded("stream timeout".into()).into();
        assert_eq!(mapped.error_code(), "deadline_exceeded");
    }

    #[test]
    fn axon_transport_maps_to_unavailable() {
        for axon in [
            Axon::Bridge("connect refused".into()),
            Axon::Stream("peer closed".into()),
            Axon::Invocation("remote panic".into()),
            Axon::Mcp("protocol drift".into()),
            Axon::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ] {
            let mapped: McpError = axon.into();
            assert_eq!(mapped.error_code(), "unavailable");
        }
    }

    #[test]
    fn axon_internal_defects_map_to_internal() {
        let mapped: McpError = Axon::SymbolNotFound("ax_invoke".into()).into();
        assert_eq!(mapped.error_code(), "internal_error");
    }

    /// Regression: the `?` operator against `Result<_, AxonError>` in
    /// an `Result<_, McpError>` context must route through the typed
    /// mapping. The distinctive case is `NotInstalled`, whose wrong
    /// mapping (e.g. `Unavailable`) would tell agents to retry a
    /// permanent identifier mismatch.
    #[test]
    fn question_mark_uses_typed_axon_conversion() {
        fn lift() -> Result<(), McpError> {
            Err::<(), _>(Axon::NotInstalled("install-x".into()))?;
            Ok(())
        }
        assert_eq!(lift().unwrap_err().error_code(), "not_found");
    }
}
