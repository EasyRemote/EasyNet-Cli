use super::*;

use crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY;
use crate::daemon::invocation::dispatch::unary_dispatcher::require_complete_signed_remote_request;
use easynet_axon::pb::axon::v1::{causal_context, Empty};

const CALLER: &str = "easynet:///r/test-realm/device/caller";
const CALLEE: &str = "easynet:///r/test-realm/device/target";
const ABILITY_URA: &str = "easynet:///r/test-realm/ability/device.target.echo";

fn complete_request() -> InvokeRequest {
    let descriptor_ref = test_descriptor_ref(CALLEE, "echo");
    InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                ura: CALLER.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: CALLEE.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: CALLEE.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: vec![7; 16],
            causal_context: Some(easynet_axon::pb::axon::v1::CausalContext {
                form: Some(causal_context::Form::None(Empty {})),
            }),
            caller_signature: Some(CallerSignature {
                algorithm: "ed25519".to_string(),
                signature: vec![3; 64],
                key_id_hint: CALLER.to_string(),
            }),
            ..Envelope::default()
        }),
        function_name: ABILITY_URA.to_string(),
        arguments: br#"{"value":1}"#.to_vec(),
        metadata: HashMap::from([(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            descriptor_ref,
        )]),
        ..InvokeRequest::default()
    }
}

#[test]
fn complete_descriptor_bound_request_is_relayable() {
    require_complete_signed_remote_request(&complete_request()).expect("complete request");
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
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert!(status.message().contains("does not match callee"));
}
