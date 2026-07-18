use super::*;

use crate::daemon::invocation::dispatch::unary_dispatcher::require_complete_signed_remote_request;
use axon_sdk::pb::axon::v1::{causal_context, Empty};

const CALLER: &str = "easynet:///r/test-realm/device/caller";
const CALLEE: &str = "easynet:///r/test-realm/device/target";

fn complete_request() -> InvokeRequest {
    let descriptor_ref = test_descriptor_ref(CALLEE, "echo");
    InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                ura: CALLER.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: CALLEE.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: CALLEE.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            invocation_nonce: vec![7; 16],
            causal_context: Some(axon_sdk::pb::axon::v1::CausalContext {
                form: Some(causal_context::Form::None(Empty {})),
            }),
            caller_signature: Some(CallerSignature {
                algorithm: "ed25519".to_string(),
                signature: vec![3; 64],
                key_id_hint: CALLER.to_string(),
            }),
            ..Envelope::default()
        }),
        target: Some(
            wire_invocation_target(&descriptor_ref, "echo").expect("typed descriptor target"),
        ),
        arguments: br#"{"value":1}"#.to_vec(),
        ..InvokeRequest::default()
    }
}

#[test]
fn complete_descriptor_bound_request_is_relayable() {
    require_complete_signed_remote_request(&complete_request()).expect("complete request");
}

#[test]
fn metadata_only_descriptor_ref_is_ignored() {
    let mut request = complete_request();
    let descriptor_ref = test_descriptor_ref(CALLEE, "echo");
    request.target = None;
    request
        .metadata
        .insert("x-product-proof".to_string(), descriptor_ref);

    let status = require_complete_signed_remote_request(&request).unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("typed_target"));
}

#[test]
fn route_only_typed_target_is_not_relayable() {
    let mut request = complete_request();
    request.target = Some(wire_invocation_target("echo", "echo").unwrap());

    let status = require_complete_signed_remote_request(&request).unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("complete descriptor ref"));
}

#[test]
fn missing_caller_signature_fails_closed() {
    let mut request = complete_request();
    request.envelope.as_mut().unwrap().caller_signature = None;
    let status = require_complete_signed_remote_request(&request).unwrap_err();
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert!(status.message().contains("explicit caller signature"));
}

#[test]
fn descriptor_owner_mismatch_fails_closed() {
    let mut request = complete_request();
    request
        .envelope
        .as_mut()
        .unwrap()
        .callee
        .as_mut()
        .unwrap()
        .ura = "easynet:///r/other-realm/device/other".to_string();
    let status = require_complete_signed_remote_request(&request).unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("complete descriptor ref"));
}
