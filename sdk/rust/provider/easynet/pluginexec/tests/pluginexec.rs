use std::io::Cursor;

use easynet_provider_pluginexec::{serve_exec_plugin_io, SidecarInvocation};
use serde_json::{json, Value};

fn request_frame() -> String {
    json!({
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller": "easynet:///r/hub/user/alice",
            "callee": "easynet:///r/hub/device/provider",
            "ability": "demo.echo",
            "subject": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "causal_context": {"root": true},
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
    assert_eq!(invocation.caller, "easynet:///r/hub/user/alice");
    assert_eq!(invocation.callee, "easynet:///r/hub/device/provider");
    assert_eq!(invocation.ability, "demo.echo");
    assert_eq!(invocation.subject, "easynet:///r/hub/resource/demo");
    assert_eq!(invocation.invocation_nonce, vec![1, 2, 3, 4]);
    assert_eq!(invocation.causal_context, json!({"root": true}));
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
