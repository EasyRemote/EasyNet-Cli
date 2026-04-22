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

// ── McpError → AxonError (reverse classification) ───────────────────────────
//
// The SDK's `ReconnectingBridge::with_bridge` takes a closure that returns
// `Result<T, AxonError>` and reconnects iff the SDK's own transport classifier
// (`reconnect::is_transport_error`) flags the error as transport-level. Our
// MCP handlers return `Result<_, McpError>`; to run them under a reconnecting
// bridge we round-trip through `AxonError`, then remap back via the existing
// `From<AxonError> for McpError` impl above.
//
// Mapping contract — picked so that a round-trip
//   McpError → AxonError (via `to_axon_error`) → McpError (via `From<AxonError>`)
// preserves the original variant *and* makes the SDK's classifier agree with
// our transport intent:
//
//   | McpError variant     | AxonError carrier                     | transport? |
//   |----------------------|---------------------------------------|------------|
//   | `Unavailable(m)`     | `Bridge("unavailable: {m}")`          | yes        |
//   | `DeadlineExceeded(m)`| `DeadlineExceeded(m)`                 | yes        |
//   | `Validation(m)`      | `Validation(m)`                       | no         |
//   | `NotFound(m)`        | `NotInstalled(m)`                     | no         |
//   | `Internal(m)`        | `SymbolNotFound(m)`                   | no         |
//
// The `Unavailable` prefix is load-bearing: `is_transport_error` scans
// `Bridge` variants against a substring marker list and `"unavailable"` is one
// of those markers (gRPC status code 14's canonical text). Changing the
// prefix — or the SDK's marker list — without updating this contract would
// silently disable reconnect for transient MCP failures.
//
// The `NotFound` → `NotInstalled` and `Internal` → `SymbolNotFound` mappings
// are chosen because those AxonError variants round-trip cleanly through the
// existing `From<AxonError>` impl (see the table above the `match` at
// line ~161): `NotInstalled` maps back to `NotFound`, `SymbolNotFound` maps
// back to `Internal`. Picking other variants would break round-trip identity.
//
// The mapping is exposed as a free function (not a trait impl) deliberately:
// the lossy re-wrapping of a free-form string inside a known-transport marker
// prefix is an implementation choice for the reconnect seam, not a general
// `Into<AxonError>` contract callers should assume elsewhere.

/// Prefix attached to the `AxonError::Bridge` message when carrying an
/// `McpError::Unavailable` across the reconnect boundary. Must remain a
/// substring of [`easynet_axon::reconnect`]'s transport-marker list — it is
/// what tells the SDK's classifier "reconnect this".
///
/// Kept as a constant so the round-trip tests below can assert exact shape
/// without hardcoding the literal twice.
const UNAVAILABLE_TRANSPORT_PREFIX: &str = "unavailable: ";

/// Convert an [`McpError`] into the [`easynet_axon::AxonError`] variant that
/// the SDK's reconnect classifier (`reconnect::is_transport_error`) will
/// classify with the same transport / non-transport intent.
///
/// This is the forward half of the round-trip used by
/// `HubMcpProvider::with_bridge` to run `McpError`-producing handlers under
/// [`easynet_axon::reconnect::ReconnectingBridge`]. See the block comment
/// above for the full mapping table and the invariants each row enforces.
pub(crate) fn to_axon_error(err: McpError) -> easynet_axon::AxonError {
    use easynet_axon::AxonError as A;
    match err {
        // Transport-level — the SDK classifier must route these through a
        // reconnect attempt.
        McpError::Unavailable(m) => A::Bridge(format!("{UNAVAILABLE_TRANSPORT_PREFIX}{m}")),
        McpError::DeadlineExceeded(m) => A::DeadlineExceeded(m),
        // Application-level — must bypass reconnect. Variants picked to
        // round-trip cleanly via `From<AxonError> for McpError` above.
        McpError::Validation(m) => A::Validation(m),
        McpError::NotFound(m) => A::NotInstalled(m),
        McpError::Internal(m) => A::SymbolNotFound(m),
    }
}

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

    // ── McpError → AxonError (reverse mapping, used by reconnect seam) ──────
    //
    // These tests guard the contract documented next to `to_axon_error`. The
    // mapping is load-bearing for reconnect correctness: a wrong row here
    // either (a) silently disables reconnect for transient MCP failures by
    // routing them through a non-transport `AxonError` variant, or (b) burns
    // retry budget on permanent errors by mis-labeling them transport.
    //
    // We assert two properties per variant:
    //   1. The `AxonError` variant produced is the one we documented.
    //   2. Feeding that `AxonError` back through `From<AxonError> for McpError`
    //      yields the original `McpError` variant (round-trip identity).
    //
    // The SDK's `reconnect::is_transport_error` is `pub(crate)` so we cannot
    // assert its agreement directly. Instead, `unavailable_prefix_contains_a_transport_marker`
    // below mirrors a subset of the SDK's marker list and fails loudly if the
    // substring marker our Unavailable prefix relies on is ever dropped.

    /// Extract the variant tag for equality comparison without forcing
    /// `PartialEq` onto the public `McpError` type (its inner strings carry
    /// FFI prose we don't want to lock down).
    fn tag(err: &McpError) -> &'static str {
        err.error_code()
    }

    #[test]
    fn unavailable_round_trips_and_carries_transport_marker() {
        let msg = "peer closed before first byte";
        let axon = super::to_axon_error(McpError::Unavailable(msg.into()));
        match &axon {
            Axon::Bridge(m) => {
                assert!(
                    m.starts_with(super::UNAVAILABLE_TRANSPORT_PREFIX),
                    "transport prefix must lead the carrier message; got {m:?}"
                );
                assert!(
                    m.ends_with(msg),
                    "caller message must be preserved verbatim after the prefix; got {m:?}"
                );
            }
            other => panic!("Unavailable must map to Bridge, got {other:?}"),
        }
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "unavailable");
        assert!(
            back.message().contains(msg),
            "round-trip must not lose caller's prose"
        );
    }

    #[test]
    fn deadline_exceeded_round_trips_unmodified() {
        let msg = "invoke_ability exceeded 30000ms";
        let axon = super::to_axon_error(McpError::DeadlineExceeded(msg.into()));
        assert!(
            matches!(axon, Axon::DeadlineExceeded(ref m) if m == msg),
            "DeadlineExceeded must round-trip with an unmodified message"
        );
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "deadline_exceeded");
    }

    #[test]
    fn validation_round_trips_unmodified() {
        let axon = super::to_axon_error(McpError::Validation("bad arg".into()));
        assert!(matches!(axon, Axon::Validation(_)));
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "validation_error");
    }

    #[test]
    fn not_found_round_trips_via_not_installed() {
        // NotFound is carried as NotInstalled because that AxonError variant
        // maps back cleanly to NotFound via the forward impl. Swapping it
        // for e.g. `Invocation` would break round-trip identity and silently
        // re-categorise agent-visible errors.
        let axon = super::to_axon_error(McpError::NotFound("node-x".into()));
        assert!(
            matches!(axon, Axon::NotInstalled(_)),
            "NotFound must be carried as NotInstalled for round-trip identity"
        );
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "not_found");
    }

    #[test]
    fn internal_round_trips_via_symbol_not_found() {
        // Internal is carried as SymbolNotFound for the same reason NotFound
        // is carried as NotInstalled — it's the AxonError variant whose
        // forward mapping lands back on Internal.
        let axon = super::to_axon_error(McpError::Internal("serde drift".into()));
        assert!(
            matches!(axon, Axon::SymbolNotFound(_)),
            "Internal must be carried as SymbolNotFound for round-trip identity"
        );
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "internal_error");
    }

    /// Every variant must round-trip to its own error_code. This is an
    /// exhaustive version of the per-variant tests above so a newly added
    /// McpError variant will be caught here (missing arm → panic in
    /// `to_axon_error`) before it can silently escape the round-trip
    /// guarantee.
    #[test]
    fn all_variants_round_trip_to_same_error_code() {
        let samples = [
            McpError::Validation("v".into()),
            McpError::NotFound("n".into()),
            McpError::Unavailable("u".into()),
            McpError::DeadlineExceeded("d".into()),
            McpError::Internal("i".into()),
        ];
        for original in samples {
            let code_before = original.error_code();
            let axon = super::to_axon_error(original);
            let back: McpError = axon.into();
            assert_eq!(
                back.error_code(),
                code_before,
                "round-trip must preserve error_code; broke on {code_before}"
            );
        }
    }

    /// Regression pin on the `UNAVAILABLE_TRANSPORT_PREFIX` contract.
    ///
    /// The prefix is load-bearing because
    /// `easynet_axon::reconnect::is_transport_error` scans `AxonError::Bridge`
    /// messages against an internal marker list — if the lowercase substring
    /// "unavailable" is removed from that list (or our prefix is changed to
    /// one that doesn't match), reconnect silently stops happening for
    /// transient MCP failures.
    ///
    /// We cannot import the SDK's `MARKERS` (it's crate-private), so we pin
    /// the property here: the prefix must contain at least one known
    /// transport-marker substring in its canonical lowercase form.
    #[test]
    fn unavailable_prefix_contains_a_transport_marker() {
        // Mirror of a subset of the SDK's marker list. If the SDK drops
        // "unavailable" we'd need to pick a different marker here (and update
        // the prefix); this test is what tells a future reader that.
        const KNOWN_TRANSPORT_MARKERS: &[&str] =
            &["unavailable", "connection refused", "deadline exceeded"];
        let prefix = super::UNAVAILABLE_TRANSPORT_PREFIX.to_ascii_lowercase();
        assert!(
            KNOWN_TRANSPORT_MARKERS
                .iter()
                .any(|m| prefix.contains(m)),
            "UNAVAILABLE_TRANSPORT_PREFIX = {prefix:?} must contain a known \
             SDK transport marker, or reconnect will silently break"
        );
    }

    /// Edge case: empty messages must not panic and must still round-trip.
    /// The SDK's classifier is substring-based, so an empty Unavailable
    /// message still produces a Bridge carrier whose text is exactly the
    /// prefix — which still contains the marker.
    #[test]
    fn empty_messages_round_trip_without_panicking() {
        for original in [
            McpError::Validation(String::new()),
            McpError::NotFound(String::new()),
            McpError::Unavailable(String::new()),
            McpError::DeadlineExceeded(String::new()),
            McpError::Internal(String::new()),
        ] {
            let code = original.error_code();
            let back: McpError = super::to_axon_error(original).into();
            assert_eq!(back.error_code(), code);
        }
    }

    /// Edge case: an `McpError::Validation` whose free-form message happens
    /// to contain a transport-shaped phrase (e.g. a user copy-pasted a gRPC
    /// error into their tool arg) must NOT be reclassified as transport.
    /// The variant tag is authoritative; prose coincidence is not.
    #[test]
    fn validation_message_shaped_like_transport_stays_validation() {
        let sneaky = McpError::Validation("expected JSON, got: connection refused".into());
        let axon = super::to_axon_error(sneaky);
        assert!(
            matches!(axon, Axon::Validation(_)),
            "prose that looks transport-shaped must not sneak past the variant tag"
        );
        let back: McpError = axon.into();
        assert_eq!(tag(&back), "validation_error");
    }
}
