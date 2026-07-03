#![cfg(feature = "axon-pb")]

// PR-N6 C2: byte-deterministic fixture round-trip for SessionDispatch.
//
// Pins the JSON encoding of every SessionDispatch variant — including
// the C2-added `Request` and `RequestResult` shapes — against the
// fixture file checked in at
// `EasyNet-Federation-MVP/tests/schema_compat/baselines/rust/transport/
// session_dispatch.json`.
//
// Why a separate fixture rather than the `capture_rust_transport`
// flow: Request/RequestResult flow only on the device → hub
// direction over `session.open`, which the existing capture
// harness (synthetic client driving a hub) does not exercise. C3+C4
// land the dispatch handlers; until then the wire shape is checked
// statically here.
//
// Acceptance per LB-50 §三 row C2: this file's fixture covers the
// new variants byte-deterministically, and the assertion is "the
// fixture round-trips through serde without losing or reordering
// any field".

use std::path::PathBuf;

use easynet_cli::daemon::invocation::bidi::invoke_remote_initiator::SessionDispatch;
use serde_json::Value;

fn fixture_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .expect("CLI repo has a parent")
        .join("EasyNet-Federation-MVP")
        .join("tests/schema_compat/baselines/rust/transport/session_dispatch.json")
}

fn load_fixture() -> Value {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {} ({err})", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("parse fixture {} ({err})", path.display()))
}

#[test]
fn fixture_present_with_required_variants() {
    let fixture = load_fixture();
    let variants = fixture
        .get("variants")
        .and_then(Value::as_object)
        .expect("fixture has `variants` object");

    // C2 spec §"Wire shape" lock — these names must exist verbatim.
    for required in &[
        "dispatch",
        "result_ok",
        "result_err",
        "request",
        "request_result_ok",
        "request_result_target_offline",
        "request_result_permission_denied",
        "request_result_upstream_failure",
        "request_result_upstream_timeout",
    ] {
        assert!(
            variants.contains_key(*required),
            "fixture missing required variant `{required}`",
        );
    }
}

#[test]
fn every_fixture_variant_round_trips_through_serde() {
    let fixture = load_fixture();
    let variants = fixture
        .get("variants")
        .and_then(Value::as_object)
        .expect("fixture has `variants` object");

    for (name, frame_json) in variants {
        let recovered: SessionDispatch = serde_json::from_value(frame_json.clone())
            .unwrap_or_else(|err| panic!("decode fixture variant `{name}`: {err}"));

        let re_encoded = serde_json::to_value(&recovered)
            .unwrap_or_else(|err| panic!("re-encode variant `{name}`: {err}"));

        assert_eq!(
            &re_encoded, frame_json,
            "variant `{name}` did not round-trip byte-equal through serde; \
             a wire-shape change has shifted the canonical encoding",
        );
    }
}
