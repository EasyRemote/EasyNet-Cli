// EasyNet CLI — sidecar host tests
// ================================
//
// File: src/daemon/plugins/sidecar/tests.rs
// Description: Contract tests for sidecar frames, process RPC, stream, and bidi.

use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use serde_json::json;

use super::io::{capture_stderr_diagnostics, collect_stderr};
use super::{
    SidecarCommand, SidecarExecutionModel, SidecarInvocationEnvelope, SidecarRequestFrame,
    SidecarResponseFrame, SidecarRuntimeHost, SidecarRuntimeLimits,
};
use crate::daemon::ability::dispatch::StreamSource;
use crate::daemon::plugins::errors::PluginHostError;

#[test]
fn sidecar_open_frame_carries_daemon_invocation_envelope() {
    let frame = SidecarRequestFrame::Invoke {
        call_id: "call-1".to_string(),
        invocation: SidecarInvocationEnvelope {
            caller_ura: "easynet:///r/acme/user/alice".to_string(),
            callee_ura: "easynet:///r/acme/device/mac".to_string(),
            ability_ura: "device.test.echo".to_string(),
            subject_ura: "easynet:///r/acme/resource/display.primary".to_string(),
            invocation_nonce: vec![7; 16],
            causal_context: json!({"form": "none"}),
            args: json!({"message": "hello"}),
        },
    };

    let encoded = serde_json::to_value(frame).expect("sidecar frame serializes");
    assert_eq!(encoded["type"], json!("invoke"));
    assert_eq!(encoded["call_id"], json!("call-1"));
    assert_eq!(
        encoded["invocation"]["caller_ura"],
        json!("easynet:///r/acme/user/alice")
    );
    assert_eq!(
        encoded["invocation"]["callee_ura"],
        json!("easynet:///r/acme/device/mac")
    );
    assert_eq!(
        encoded["invocation"]["subject_ura"],
        json!("easynet:///r/acme/resource/display.primary")
    );
    assert_eq!(
        encoded["invocation"]["invocation_nonce"],
        json!(vec![7; 16])
    );
    assert_eq!(
        encoded["invocation"]["causal_context"],
        json!({"form": "none"})
    );
    assert!(encoded.get("ability_ura").is_none());
    assert!(encoded.get("args").is_none());
}

#[test]
fn sidecar_request_frame_rejects_unknown_variant_fields() {
    let raw = json!({
        "type": "invoke",
        "call_id": "call-1",
        "invocation": canonical_sidecar_envelope_json(),
        "legacy_route": "rpc"
    });

    let err = serde_json::from_value::<SidecarRequestFrame>(raw)
        .expect_err("sidecar request frames must reject unknown variant fields");
    assert!(
        err.to_string().contains("unknown field `legacy_route`"),
        "strict sidecar request decode should name the rejected field: {err}"
    );
}

#[test]
fn sidecar_invocation_envelope_rejects_unknown_identity_fields() {
    let mut raw = canonical_sidecar_envelope_json();
    raw.as_object_mut()
        .expect("sidecar envelope object")
        .insert(
            "legacy_subject".to_string(),
            json!("easynet:///r/acme/device/mac"),
        );

    let err = serde_json::from_value::<SidecarInvocationEnvelope>(raw)
        .expect_err("sidecar envelope must reject hidden identity aliases");
    assert!(
        err.to_string().contains("unknown field `legacy_subject`"),
        "strict sidecar envelope decode should name the rejected field: {err}"
    );
}

#[test]
fn sidecar_invocation_envelope_rejects_missing_canonical_tuple_objects() {
    for field in ["causal_context", "args"] {
        let mut raw = canonical_sidecar_envelope_json();
        raw.as_object_mut()
            .expect("sidecar envelope object")
            .remove(field);

        let err = serde_json::from_value::<SidecarInvocationEnvelope>(raw)
            .expect_err("sidecar envelope must reject incomplete canonical tuple fields");
        assert!(
            err.to_string()
                .contains(&format!("missing field `{field}`")),
            "strict sidecar envelope decode should name the missing field: {err}"
        );

        let mut raw = canonical_sidecar_envelope_json();
        raw.as_object_mut()
            .expect("sidecar envelope object")
            .insert(field.to_string(), serde_json::Value::Null);

        let err = serde_json::from_value::<SidecarInvocationEnvelope>(raw)
            .expect_err("sidecar envelope must reject null canonical tuple objects");
        assert!(
            err.to_string().contains("must be an object"),
            "strict sidecar envelope decode should reject null {field}: {err}"
        );
    }
}

#[test]
fn sidecar_response_frame_rejects_unknown_variant_fields() {
    let raw = json!({
        "type": "result",
        "call_id": "call-1",
        "value": {"ok": true},
        "receipt": {"legacy": true}
    });

    let err = serde_json::from_value::<SidecarResponseFrame>(raw)
        .expect_err("sidecar response frames must reject hidden receipt projections");
    assert!(
        err.to_string().contains("unknown field `receipt`"),
        "strict sidecar response decode should name the rejected field: {err}"
    );
}

#[test]
fn sidecar_runtime_model_is_explicitly_one_shot_process() {
    let host = SidecarRuntimeHost::new(SidecarCommand::new("/bin/echo"));
    assert_eq!(
        host.execution_model(),
        SidecarExecutionModel::OneShotProcess
    );
}

#[test]
fn sidecar_runtime_invokes_process_with_envelope_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    let captured = dir.path().join("captured.json");
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
read frame
printf '%s\n' "$frame" > '{}'
printf '%s\n' '{{"type":"result","call_id":"call-1","value":{{"ok":true}}}}'
"#,
            captured.display()
        ),
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let host = SidecarRuntimeHost::new(SidecarCommand::new(&script));
    let value = host
        .invoke_rpc(
            "call-1",
            SidecarInvocationEnvelope {
                caller_ura: "easynet:///r/acme/user/alice".to_string(),
                callee_ura: "easynet:///r/acme/device/mac".to_string(),
                ability_ura: "device.test.echo".to_string(),
                subject_ura: "easynet:///r/acme/resource/display.primary".to_string(),
                invocation_nonce: vec![9; 16],
                causal_context: json!({"form": "none"}),
                args: json!({"message": "hello"}),
            },
        )
        .expect("sidecar rpc");

    assert_eq!(value, json!({"ok": true}));
    let captured: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(captured).expect("captured request"))
            .expect("captured request json");
    assert_eq!(
        captured["invocation"]["caller_ura"],
        json!("easynet:///r/acme/user/alice")
    );
    assert_eq!(
        captured["invocation"]["callee_ura"],
        json!("easynet:///r/acme/device/mac")
    );
    assert_eq!(
        captured["invocation"]["subject_ura"],
        json!("easynet:///r/acme/resource/display.primary")
    );
    assert_eq!(
        captured["invocation"]["invocation_nonce"],
        json!(vec![9; 16])
    );
    assert_eq!(
        captured["invocation"]["causal_context"],
        json!({"form": "none"})
    );
    assert_eq!(captured["invocation"]["args"], json!({"message": "hello"}));
}

#[test]
fn sidecar_runtime_process_failure_reports_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
read frame
printf '%s\n' 'operator-visible failure' >&2
exit 42
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let err = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .invoke_rpc("call-1", test_invocation())
        .expect_err("sidecar non-zero exit must be typed host error");

    assert!(format!("{err}").contains("operator-visible failure"));
}

#[test]
fn sidecar_stderr_capture_preserves_binary_diagnostics() {
    let stderr = capture_stderr_diagnostics(io::Cursor::new([b'o', 0xff, b'k']));

    assert!(
        stderr.contains("o") && stderr.contains('\u{fffd}') && stderr.contains("k"),
        "binary stderr must be preserved lossily, got {stderr:?}"
    );
}

#[test]
fn sidecar_stderr_capture_reports_reader_failure() {
    struct FailingReader {
        emitted: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.emitted {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "synthetic stderr failure",
                ));
            }
            self.emitted = true;
            let bytes = b"partial diagnostic";
            buf[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
    }

    let stderr = capture_stderr_diagnostics(FailingReader { emitted: false });

    assert!(stderr.contains("partial diagnostic"));
    assert!(stderr.contains("sidecar stderr capture failed"));
    assert!(stderr.contains("synthetic stderr failure"));
}

#[test]
fn sidecar_stderr_collection_reports_reader_panic() {
    let handle = std::thread::spawn(|| -> String {
        panic!("synthetic stderr reader panic");
    });

    let stderr = collect_stderr(Some(handle));

    assert_eq!(stderr, "sidecar stderr reader panicked");
}

#[test]
fn sidecar_runtime_times_out_and_kills_hung_rpc() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
printf '%s\n' 'sidecar entered hung rpc' >&2
read frame
sleep 30
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let started = Instant::now();
    let err = SidecarRuntimeHost::with_limits(
        SidecarCommand::new(&script),
        SidecarRuntimeLimits::new(Duration::from_millis(100), Duration::from_millis(100)),
    )
    .invoke_rpc("call-1", test_invocation())
    .expect_err("hung sidecar rpc must time out");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timeout must be bounded"
    );
    match err {
        PluginHostError::SidecarProcessTimedOut {
            timeout_ms, stderr, ..
        } => {
            assert_eq!(timeout_ms, 100);
            assert!(
                stderr.is_empty() || stderr.contains("sidecar entered hung rpc"),
                "timeout keeps any stderr already captured"
            );
        }
        other => panic!("expected typed sidecar timeout, got {other:?}"),
    }
}

#[test]
fn sidecar_stream_rejects_frames_after_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
read frame
printf '%s\n' '{"type":"stream_item","call_id":"call-1","value":{"step":1}}'
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"closed"}'
printf '%s\n' '{"type":"stream_item","call_id":"call-1","value":{"step":2}}'
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let err = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .invoke_stream_snapshot("call-1", test_invocation())
        .expect_err("stream output after terminal is invalid");

    assert!(format!("{err}").contains("emitted item after terminal frame"));
}

#[test]
fn sidecar_stream_rejects_duplicate_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
read frame
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"closed"}'
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"closed-again"}'
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let err = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .invoke_stream_snapshot("call-1", test_invocation())
        .expect_err("duplicate stream terminal is invalid");

    assert!(format!("{err}").contains("multiple terminal frames"));
}

#[tokio::test]
async fn sidecar_stream_open_returns_live_receiver_before_process_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
read frame
printf '%s\n' '{"type":"stream_item","call_id":"call-1","value":{"step":1}}'
sleep 1
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"done"}'
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let started = Instant::now();
    let source = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .open_stream("call-1", test_invocation())
        .expect("sidecar live stream");
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "open_stream must return before the sidecar reaches terminal"
    );

    let StreamSource::Live(mut rx) = source else {
        panic!("sidecar stream must be live, not a finite snapshot");
    };
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("live item before timeout")
        .expect("live stream item");
    assert_eq!(first, json!({"step": 1}));
    let closed = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await;
    assert!(
        matches!(
            closed,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed))
        ),
        "terminal should close the live sidecar stream"
    );
}

#[tokio::test]
async fn sidecar_bidi_pump_forwards_input_output_and_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    let captured = dir.path().join("input.json");
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
read open_frame
printf '%s\n' '{{"type":"bidi_output","call_id":"call-1","frame":{{"server":"opened"}}}}'
read input_frame
printf '%s\n' "$input_frame" > '{}'
printf '%s\n' '{{"type":"bidi_output","call_id":"call-1","frame":{{"server":"saw-input"}}}}'
printf '%s\n' '{{"type":"terminal","call_id":"call-1","reason":"done"}}'
"#,
            captured.display()
        ),
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let mut source = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .open_bidi("call-1", test_invocation())
        .expect("sidecar bidi open");
    source
        .to_client
        .send(json!({"client": "input"}))
        .await
        .expect("send bidi input");
    drop(source.to_client);

    let first = source
        .from_client
        .recv()
        .await
        .expect("opened output")
        .into_json_value()
        .expect("json output");
    let second = source
        .from_client
        .recv()
        .await
        .expect("saw input output")
        .into_json_value()
        .expect("json output");
    let closed = source
        .from_client
        .recv()
        .await
        .expect("closed output")
        .into_json_value()
        .expect("json output");

    assert_eq!(first, json!({"server": "opened"}));
    assert_eq!(second, json!({"server": "saw-input"}));
    assert_eq!(closed["type"], json!("closed"));
    assert_eq!(closed["reason"], json!("done"));
    assert!(
        source.from_client.recv().await.is_none(),
        "bidi source closes after one terminal"
    );

    let captured: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(captured).expect("captured input"))
            .expect("captured input json");
    assert_eq!(captured["type"], json!("bidi_input"));
    assert_eq!(captured["call_id"], json!("call-1"));
    assert_eq!(captured["frame"], json!({"client": "input"}));
}

#[tokio::test]
async fn sidecar_bidi_suppresses_duplicate_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("sidecar.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
read open_frame
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"first"}'
printf '%s\n' '{"type":"terminal","call_id":"call-1","reason":"second"}'
"#,
    )
    .expect("write sidecar");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod sidecar");

    let mut source = SidecarRuntimeHost::new(SidecarCommand::new(&script))
        .open_bidi("call-1", test_invocation())
        .expect("sidecar bidi open");
    drop(source.to_client);

    let closed = source
        .from_client
        .recv()
        .await
        .expect("closed output")
        .into_json_value()
        .expect("json output");

    assert_eq!(closed["type"], json!("closed"));
    assert_eq!(closed["reason"], json!("first"));
    assert!(
        source.from_client.recv().await.is_none(),
        "second terminal frame must not be emitted"
    );
}

fn test_invocation() -> SidecarInvocationEnvelope {
    SidecarInvocationEnvelope {
        caller_ura: "easynet:///r/acme/user/alice".to_string(),
        callee_ura: "easynet:///r/acme/device/mac".to_string(),
        ability_ura: "device.test.stream".to_string(),
        subject_ura: "easynet:///r/acme/resource/display.primary".to_string(),
        invocation_nonce: vec![1; 16],
        causal_context: json!({"form": "none"}),
        args: json!({"watch": true}),
    }
}

fn canonical_sidecar_envelope_json() -> serde_json::Value {
    json!({
        "caller_ura": "easynet:///r/acme/user/alice",
        "callee_ura": "easynet:///r/acme/device/mac",
        "ability_ura": "device.test.echo",
        "subject_ura": "easynet:///r/acme/resource/display.primary",
        "invocation_nonce": vec![7; 16],
        "causal_context": {"form": "none"},
        "args": {}
    })
}
