use std::io::Cursor;

use easynet_provider_pluginexec::{serve_exec_plugin_io, SidecarInvocation};
use serde_json::{json, Value};

fn request_frame() -> String {
    json!({
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "causal_context": {"form": "none"},
            "args": {"message": "hello"}
        }
    })
    .to_string()
        + "\n"
}

#[test]
fn sidecar_invocation_projects_daemon_frame() {
    let frame: Value = serde_json::from_str(&request_frame()).expect("frame");
    let invocation = SidecarInvocation::from_frame(frame).expect("invocation");

    assert_eq!(invocation.call_id, "call-1");
    assert_eq!(invocation.caller_ura, "easynet:///r/hub/user/alice");
    assert_eq!(invocation.callee_ura, "easynet:///r/hub/device/provider");
    assert_eq!(invocation.ability_ura, "demo.echo");
    assert_eq!(invocation.subject_ura, "easynet:///r/hub/resource/demo");
    assert_eq!(invocation.invocation_nonce, vec![1, 2, 3, 4]);
    assert_eq!(invocation.causal_context, json!({"form": "none"}));
    assert_eq!(invocation.args["message"], json!("hello"));
}

#[test]
fn serve_exec_plugin_writes_result_frame() {
    let mut input = Cursor::new(request_frame());
    let mut output = Vec::new();

    serve_exec_plugin_io(&mut input, &mut output, |invocation| {
        Ok::<_, std::convert::Infallible>(json!({
            "ok": true,
            "message": invocation.args["message"],
            "nonce_len": invocation.invocation_nonce.len()
        }))
    })
    .expect("serve");

    let response: Value = serde_json::from_slice(&output).expect("response");
    assert_eq!(
        response,
        json!({
            "type": "result",
            "call_id": "call-1",
            "value": {"ok": true, "message": "hello", "nonce_len": 4}
        })
    );
}

#[test]
fn serve_exec_plugin_writes_error_frame_for_handler_failure() {
    let mut input = Cursor::new(request_frame());
    let mut output = Vec::new();

    serve_exec_plugin_io(&mut input, &mut output, |_invocation| {
        Err::<Value, _>("boom")
    })
    .expect("serve");

    let response: Value = serde_json::from_slice(&output).expect("response");
    assert_eq!(
        response,
        json!({
            "type": "error",
            "call_id": "call-1",
            "message": "boom"
        })
    );
}

#[test]
fn sidecar_invocation_rejects_non_invoke_frame() {
    let frame = json!({
        "type": "stream_open",
        "call_id": "call-1",
        "invocation": {}
    });

    assert!(SidecarInvocation::from_frame(frame).is_err());
}

#[test]
fn sidecar_invocation_rejects_retired_tuple_aliases() {
    let frame = json!({
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "caller": "easynet:///r/hub/user/bob",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "args": {}
        }
    });

    let error = SidecarInvocation::from_frame(frame).expect_err("retired alias");
    assert!(error.to_string().contains("retired"));
}

#[test]
fn sidecar_invocation_rejects_unknown_invocation_fields() {
    let frame = json!({
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "descriptor_ref": "legacy-provider-leak",
            "args": {}
        }
    });

    let error = SidecarInvocation::from_frame(frame).expect_err("unknown field");
    assert!(error.to_string().contains("canonical invocation frame"));
}

#[test]
fn sidecar_invocation_rejects_unknown_request_fields() {
    let frame = json!({
        "type": "invoke",
        "call_id": "call-1",
        "legacy_mode": "json",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "args": {}
        }
    });

    let error = SidecarInvocation::from_frame(frame).expect_err("unknown field");
    assert!(error.to_string().contains("canonical request frame"));
}

#[test]
fn sidecar_invocation_rejects_missing_canonical_invocation_objects() {
    for field in ["causal_context", "args"] {
        let mut frame: Value = serde_json::from_str(&request_frame()).expect("frame");
        frame["invocation"]
            .as_object_mut()
            .expect("invocation object")
            .remove(field);

        let error = SidecarInvocation::from_frame(frame).expect_err("missing object");
        assert!(
            error.to_string().contains("must be an object"),
            "unexpected {field} error: {error}"
        );

        let mut frame: Value = serde_json::from_str(&request_frame()).expect("frame");
        frame["invocation"]
            .as_object_mut()
            .expect("invocation object")
            .insert(field.to_string(), Value::Null);

        let error = SidecarInvocation::from_frame(frame).expect_err("null object");
        assert!(
            error.to_string().contains("must be an object"),
            "unexpected {field} null error: {error}"
        );
    }
}
