use super::*;

#[test]
fn remote_bidi_target_ura_preserves_canonical_device_ura() {
    let open = make_envelope_open_with_callee("  easynet:///r/test-realm/device/dev-B  ");
    assert_eq!(
        remote_bidi_target_ura(&open).as_deref(),
        Some("easynet:///r/test-realm/device/dev-B")
    );
}

#[test]
fn remote_bidi_target_ura_preserves_non_device_callee_for_rejection() {
    let open = make_envelope_open_with_callee("easynet:///r/test-realm/agent/alice.dev-B");
    assert_eq!(
        remote_bidi_target_ura(&open).as_deref(),
        Some("easynet:///r/test-realm/agent/alice.dev-B"),
        "remote bidi target extraction must preserve non-device callee URAs so \
         self-target and presence lookup reject unsupported targets naturally"
    );
}

#[test]
fn extract_envelope_open_returns_inner_for_envelope_open_frame() {
    let frame = InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: Some(UpPayload::EnvelopeOpen(make_envelope_open(
            ABILITY_INVOKE_REMOTE,
            b"{}".to_vec(),
        ))),
    };
    let eo = extract_envelope_open(&frame).expect("extracted");
    assert_eq!(
        eo.target.as_ref().unwrap().ability_name,
        ABILITY_INVOKE_REMOTE
    );
}

#[test]
fn validate_and_extract_bidi_frame0_rejects_non_zero_sequence() {
    let frame = InvokeBidiUp {
        sequence: 7,
        mac: Vec::new(),
        payload: Some(UpPayload::EnvelopeOpen(make_envelope_open(
            ABILITY_INVOKE_REMOTE,
            b"{}".to_vec(),
        ))),
    };
    let err = validate_and_extract_bidi_frame0(&frame).expect_err("must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains(REASON_BIDI_FIRST_FRAME_SEQUENCE),
        "wire reason must be preserved, got: {}",
        err.message()
    );
}

#[test]
fn validate_and_extract_bidi_frame0_rejects_non_strict_ordering() {
    let mut envelope_open = make_envelope_open(ABILITY_INVOKE_REMOTE, b"{}".to_vec());
    envelope_open.streams.push(StreamDescriptor {
        stream_id: 9,
        ordering: "UNORDERED".to_string(),
        ..StreamDescriptor::default()
    });
    let frame = InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
    };
    let err = validate_and_extract_bidi_frame0(&frame).expect_err("must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains(REASON_BIDI_NON_STRICT_ORDERING),
        "wire reason must be preserved, got: {}",
        err.message()
    );
}

#[test]
fn extract_envelope_open_rejects_binary_chunk_first_frame() {
    let frame = InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: Some(UpPayload::BinaryChunk(BinaryChunk::default())),
    };
    let err = extract_envelope_open(&frame).expect_err("must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("EnvelopeOpen"));
}

#[test]
fn extract_envelope_open_rejects_control_first_frame() {
    let frame = InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: Some(UpPayload::Control(BidiControl::default())),
    };
    let err = extract_envelope_open(&frame).expect_err("must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[test]
fn extract_envelope_open_rejects_payload_none() {
    let frame = InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: None,
    };
    let err = extract_envelope_open(&frame).expect_err("must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("no payload"));
}

#[test]
fn map_local_bidi_handler_stdout_decodes_to_binary_chunk() {
    use base64::Engine as _;

    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::Pty,
        &serde_json::json!({
            "type": "stdout",
            "data": base64::engine::general_purpose::STANDARD.encode(b"hello"),
        }),
        7,
    );
    match frame {
        LocalBidiHandlerFrame::Forward(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(chunk)),
            ..
        }) => {
            assert_eq!(chunk.stream_id, 7);
            assert_eq!(chunk.data, b"hello");
        }
        other => panic!("expected stdout → BinaryChunk, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_exit_becomes_completed_receipt() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::Pty,
        &serde_json::json!({
            "type": "exit",
            "status": 23,
        }),
        1,
    );
    match frame {
        LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        }) => {
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
            );
            assert!(
                receipt.reason.contains("23"),
                "exit status should surface in the terminal receipt reason"
            );
        }
        other => panic!("expected exit → terminal receipt, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_file_transfer_chunk_decodes_to_binary_chunk() {
    use base64::Engine as _;

    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::FileTransfer,
        &serde_json::json!({
            "type": "chunk",
            "data": base64::engine::general_purpose::STANDARD.encode(b"file-bytes"),
        }),
        11,
    );
    match frame {
        LocalBidiHandlerFrame::Forward(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(chunk)),
            ..
        }) => {
            assert_eq!(chunk.stream_id, 11);
            assert_eq!(chunk.data, b"file-bytes");
        }
        other => panic!("expected file_transfer chunk → BinaryChunk, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_file_transfer_complete_becomes_receipt_with_payload() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::FileTransfer,
        &serde_json::json!({
            "type": "complete",
            "sha256": "deadbeef",
            "bytes": 9,
        }),
        1,
    );
    match frame {
        LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        }) => {
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
            );
            assert_eq!(receipt.payload_content_type, "application/json");
            assert!(
                receipt.cleanup_complete,
                "terminal file_transfer completion receipt must close the bidi lifecycle"
            );
            assert!(
                receipt.failure.is_none(),
                "completed receipts must not carry typed failure"
            );
            let payload: serde_json::Value =
                serde_json::from_slice(&receipt.payload).expect("json payload");
            assert_eq!(payload["sha256"], "deadbeef");
            assert_eq!(payload["bytes"], 9);
        }
        other => panic!("expected file_transfer complete → terminal receipt, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_file_transfer_error_becomes_failed_receipt_with_payload() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::FileTransfer,
        &serde_json::json!({
            "type": "error",
            "code": "disk_full",
            "message": "no space left on device",
        }),
        1,
    );
    match frame {
        LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        }) => {
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Failed.to_wire_i32()
            );
            assert!(receipt.reason.contains("disk_full"));
            assert!(receipt.reason.contains("no space left on device"));
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "DISK_FULL");
            assert_eq!(failure.message, receipt.reason);
            assert_eq!(failure.stage, ErrorStage::Execution as i32);
            let payload: serde_json::Value =
                serde_json::from_slice(&receipt.payload).expect("json payload");
            assert_eq!(payload["type"], "error");
        }
        other => panic!("expected file_transfer error → failed receipt, got {other:?}"),
    }
}

#[test]
fn terminal_receipt_extracts_admission_failure_code_from_reason() {
    let frame = build_bidi_terminal_receipt(
        easynet_axon::invocation::InvocationState::Failed,
        "CALLER_SIGNATURE_INVALID: rejected session.open",
    );
    match frame {
        InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        } => {
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "CALLER_SIGNATURE_INVALID");
            assert_eq!(failure.stage, ErrorStage::CallerAuthentication as i32);
            assert_eq!(failure.security_class, SecurityClass::Authentication as i32);
        }
        other => panic!("expected failed receipt, got {other:?}"),
    }
}

#[test]
fn terminal_receipt_extracts_presence_registry_failure_code_from_reason() {
    let frame = build_bidi_terminal_receipt(
        easynet_axon::invocation::InvocationState::Failed,
        "target device is not in PresenceRegistry; the owning daemon is offline",
    );
    match frame {
        InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        } => {
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "TARGET_NOT_IN_PRESENCE_REGISTRY");
            assert_eq!(failure.stage, ErrorStage::Transport as i32);
            assert_eq!(failure.security_class, SecurityClass::Transport as i32);
        }
        other => panic!("expected failed receipt, got {other:?}"),
    }
}

#[test]
fn terminal_receipt_projects_route_negative_to_resolution_stage() {
    let frame = build_bidi_terminal_receipt(
        easynet_axon::invocation::InvocationState::Failed,
        "ROUTE_NEGATIVE: namespace.resolve negative for `browser.open`: NEGATIVE_REASON_NOROUTE",
    );
    match frame {
        InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        } => {
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "ROUTE_NEGATIVE");
            assert_eq!(failure.stage, ErrorStage::AbilityResolution as i32);
            assert_eq!(failure.security_class, SecurityClass::Unspecified as i32);
        }
        other => panic!("expected failed receipt, got {other:?}"),
    }
}

#[test]
fn terminal_receipt_marks_timeout_retryable() {
    let frame = build_bidi_terminal_receipt(
        easynet_axon::invocation::InvocationState::TimedOut,
        "terminal read timed out",
    );
    match frame {
        InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        } => {
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "INVOCATION_TIMED_OUT");
            assert_eq!(failure.stage, ErrorStage::Execution as i32);
            assert_eq!(failure.security_class, SecurityClass::Unspecified as i32);
            assert!(failure.retryable);
        }
        other => panic!("expected timed-out receipt, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_up_payload_translates_file_transfer_binary_chunk() {
    use base64::Engine as _;

    let mapped = map_local_bidi_up_payload(
        LocalBidiWireKind::FileTransfer,
        UpPayload::BinaryChunk(BinaryChunk {
            data: b"abc".to_vec(),
            ..BinaryChunk::default()
        }),
    );
    match mapped {
        LocalBidiUpFrame::Forward(value) => {
            assert_eq!(value["type"], "chunk");
            assert_eq!(
                value["data"],
                base64::engine::general_purpose::STANDARD.encode(b"abc")
            );
        }
        other => panic!("expected file_transfer binary → chunk JSON, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_up_payload_translates_file_transfer_eof_control() {
    let mapped = map_local_bidi_up_payload(
        LocalBidiWireKind::FileTransfer,
        UpPayload::Control(BidiControl {
            control: Some(easynet_axon::pb::axon::v1::bidi_control::Control::Eof(true)),
        }),
    );
    match mapped {
        LocalBidiUpFrame::ForwardAndClose(value) => {
            assert_eq!(value["type"], "eof");
        }
        other => panic!("expected file_transfer eof → eof JSON, got {other:?}"),
    }
}

#[test]
#[cfg(feature = "remote-desktop")]
fn remote_desktop_bidi_uses_json_frame_wire_kind() {
    let registry = crate::daemon::ability::wire::AbilityWireRegistry::load_default_profile()
        .expect("remote desktop plugin wire profile loads");
    assert_eq!(
        registry.bidi_wire_kind_for("remote_desktop.attach"),
        Some(LocalBidiWireKind::JsonFrames)
    );
}

#[test]
fn map_local_bidi_handler_json_frames_preserves_json_payload() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::JsonFrames,
        &serde_json::json!({
            "type": "frame",
            "seq": 7,
            "image_bytes_b64": "abc",
        }),
        3,
    );
    match frame {
        LocalBidiHandlerFrame::Forward(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(chunk)),
            ..
        }) => {
            assert_eq!(chunk.stream_id, 3);
            let payload: serde_json::Value =
                serde_json::from_slice(&chunk.data).expect("json frame payload");
            assert_eq!(payload["type"], "frame");
            assert_eq!(payload["seq"], 7);
            assert_eq!(payload["image_bytes_b64"], "abc");
        }
        other => panic!("expected JSON frame → BinaryChunk, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_json_frames_error_becomes_failed_receipt() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::JsonFrames,
        &serde_json::json!({
            "type": "error",
            "code": "permission_denied",
            "message": "screen capture permission denied",
        }),
        3,
    );
    match frame {
        LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        }) => {
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Failed.to_wire_i32()
            );
            assert_eq!(receipt.payload_content_type, "application/json");
            assert_eq!(
                receipt.reason,
                "permission_denied: screen capture permission denied"
            );
            let failure = receipt.failure.as_ref().expect("typed receipt failure");
            assert_eq!(failure.code, "PERMISSION_DENIED");
            assert_eq!(failure.message, receipt.reason);
            let payload: serde_json::Value =
                serde_json::from_slice(&receipt.payload).expect("json payload");
            assert_eq!(payload["type"], "error");
        }
        other => panic!("expected JSON error → failed receipt, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_handler_json_frames_closed_becomes_completed_receipt() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::JsonFrames,
        &serde_json::json!({
            "type": "closed",
            "reason": "client_closed",
        }),
        3,
    );
    match frame {
        LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(receipt)),
            ..
        }) => {
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
            );
            assert!(receipt.failure.is_none());
            assert_eq!(receipt.payload_content_type, "application/json");
            let payload: serde_json::Value =
                serde_json::from_slice(&receipt.payload).expect("json payload");
            assert_eq!(payload["type"], "closed");
        }
        other => panic!("expected JSON closed → completed receipt, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_ability_json_frames_forwards_raw_binary_payload() {
    let frame = map_local_bidi_ability_frame(
        LocalBidiWireKind::JsonFrames,
        AbilityFrame {
            payload: b"\xff\xd8raw-jpeg\xff\xd9".to_vec(),
            content_type: "image/jpeg".to_string(),
            terminal: false,
        },
        9,
    );
    match frame {
        LocalBidiHandlerFrame::Forward(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(chunk)),
            ..
        }) => {
            assert_eq!(chunk.stream_id, 9);
            assert_eq!(chunk.data, b"\xff\xd8raw-jpeg\xff\xd9");
        }
        other => panic!("expected raw binary JsonFrames payload → BinaryChunk, got {other:?}"),
    }
}

#[test]
fn map_local_bidi_up_payload_json_frames_forwards_json_control() {
    let mapped = map_local_bidi_up_payload(
        LocalBidiWireKind::JsonFrames,
        UpPayload::BinaryChunk(BinaryChunk {
            data: br#"{"type":"close","reason":"test"}"#.to_vec(),
            ..BinaryChunk::default()
        }),
    );
    match mapped {
        LocalBidiUpFrame::Forward(value) => {
            assert_eq!(value["type"], "close");
            assert_eq!(value["reason"], "test");
        }
        other => panic!("expected JSON BinaryChunk → handler JSON, got {other:?}"),
    }
}

#[tokio::test]
async fn local_bidi_down_stream_emits_admission_receipt_before_handler_frames() {
    use futures::StreamExt as _;

    let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
    down_tx
        .send(Ok(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: 9,
                data: b"payload".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        }))
        .await
        .expect("enqueue payload frame");
    drop(down_tx);

    let mut stream = LocalBidiDownStream::new(down_rx);
    let first = stream
        .next()
        .await
        .expect("admission receipt frame")
        .expect("receipt is ok");
    match first.payload {
        Some(DownPayload::Receipt(receipt)) => {
            assert_eq!(first.sequence, 0);
            assert_eq!(
                receipt.state,
                easynet_axon::invocation::InvocationState::Admitted.to_wire_i32()
            );
        }
        other => panic!("expected admission receipt at sequence 0, got {other:?}"),
    }

    let second = stream
        .next()
        .await
        .expect("payload frame")
        .expect("payload is ok");
    match second.payload {
        Some(DownPayload::BinaryChunk(chunk)) => {
            assert_eq!(second.sequence, 1);
            assert_eq!(chunk.stream_id, 9);
            assert_eq!(chunk.data, b"payload");
        }
        other => panic!("expected payload BinaryChunk at sequence 1, got {other:?}"),
    }

    assert!(
        stream.next().await.is_none(),
        "stream should end after the queued payload frame"
    );
}

#[test]
fn validate_session_realm_accepts_same_realm() {
    let anchor = RealmTrustAnchor::default();
    validate_session_realm(
        "easynet:///r/realm-a/device/device-1",
        Some("realm-a"),
        &anchor,
    )
    .expect("same-realm caller must pass");
}

#[test]
fn validate_session_realm_accepts_same_realm_device_ura() {
    let anchor = RealmTrustAnchor::default();
    validate_session_realm(
        "easynet:///r/realm-a/device/device-1",
        Some("realm-a"),
        &anchor,
    )
    .expect("same-realm device URA must pass");
}

#[test]
fn validate_session_realm_rejects_cross_realm_without_trust() {
    let anchor = RealmTrustAnchor::default();
    let err = validate_session_realm(
        "easynet:///r/realm-b/device/device-1",
        Some("realm-a"),
        &anchor,
    )
    .expect_err("cross-realm caller without trust entry must be rejected");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message()
            .contains("not present in the realm trust anchor"),
        "got: {}",
        err.message()
    );
}

#[test]
fn validate_session_realm_accepts_cross_realm_when_trust_anchor_has_caller() {
    // Federated identity path: caller URA lives in realm-b
    // but the local trust anchor on realm-a's hub has an
    // explicit entry for it. Mirrors the admission gate's
    // existing FederatedKeyResolver hit; closes LB-49.
    use crate::daemon::trust::anchor::{TrustedAgent, TrustedAgentRole};
    let entry = TrustedAgent {
        agent_ura: "easynet:///r/realm-b/device/device-1".to_string(),
        public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
        role: TrustedAgentRole::Device,
        added_at_unix_ms: 1_777_640_000_000,
        origin_realm: Some("federated-tenant".to_string()),
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
    validate_session_realm(
        "easynet:///r/realm-b/device/device-1",
        Some("realm-a"),
        &anchor,
    )
    .expect("cross-realm caller with trust-anchor entry must pass");
}

#[test]
fn validate_session_realm_rejects_malformed_ura() {
    let anchor = RealmTrustAnchor::default();
    let err = validate_session_realm("not-a-ura", Some("realm-a"), &anchor)
        .expect_err("malformed URA must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("canonical"));
}

#[test]
fn build_remote_bidi_open_dispatch_frame_carries_resource_binding() {
    let frame = build_remote_bidi_open_dispatch_frame(
        43,
        "easynet:///r/realm/device/dev",
        Some("easynet:///r/realm/resource/display-1"),
        "remote_desktop.attach",
        br#"{"session_id":"rd-1"}"#,
        HashMap::new(),
    )
    .expect("built");
    let payload = match frame.frame.payload.expect("frame has payload") {
        DownPayload::BinaryChunk(chunk) => chunk,
        _ => panic!("expected BinaryChunk"),
    };
    assert_eq!(payload.stream_id, INVOKE_REMOTE_STREAM_ID);
    let parsed: SessionDispatch =
        serde_json::from_slice(&payload.data).expect("decode SessionDispatch");
    match parsed {
        SessionDispatch::BidiOpen {
            call_id,
            callee_ura,
            subject_ura,
            ability,
            args,
            ..
        } => {
            assert_eq!(call_id, 43);
            assert_eq!(callee_ura.as_deref(), Some("easynet:///r/realm/device/dev"));
            assert_eq!(
                subject_ura.as_deref(),
                Some("easynet:///r/realm/resource/display-1")
            );
            assert_eq!(ability, "remote_desktop.attach");
            assert_eq!(args, br#"{"session_id":"rd-1"}"#);
        }
        _ => panic!("expected BidiOpen variant"),
    }
}

/// step-3b hub arm (DEC-F004): the bidi-open carrier follows the
/// execution host's negotiated contract. Three cells: v1 host with a
/// seven-tuple envelope rides the canonical DispatchCall (selected
/// callee transplanted, open_bidi set); a v1 host WITHOUT an envelope
/// pins to JSON (hollow-canonical-frame doctrine, mirroring the unary
/// slot fallback); a v0 host keeps JSON for the deletion window.
#[tokio::test]
async fn remote_bidi_open_frame_rides_carrier_by_negotiated_contract() {
    use easynet_axon::pb::axon::v1::EnvelopeOpen;

    let svc = make_service().with_session_realm("test-realm");
    let target_ura = "easynet:///r/test-realm/device/bidi-target";
    publish_test_route(&svc, target_ura, "remote_desktop.attach");
    let route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(target_ura, "remote_desktop.attach")
        .expect("published route resolves");

    let envelope_open = EnvelopeOpen {
        envelope: Some(
            crate::daemon::invocation::ProtoEnvelope::targeted(
                "easynet:///r/test-realm/user/alice",
                "easynet:///r/test-realm/device/caller-supplied",
                "easynet:///r/test-realm/device/caller-supplied",
            )
            .expect("valid remote bidi open envelope")
            .into_inner(),
        ),
        initial_args: br#"{"session_id":"rd-9"}"#.to_vec(),
        ..Default::default()
    };

    // Cell 1: v1 + envelope → canonical frame, callee re-selected.
    let frame = build_remote_bidi_open_frame_for_contract(true, 7, &route, &envelope_open)
        .expect("v1 frame builds");
    match frame.frame.payload.expect("payload") {
        easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::DispatchCall(call) => {
            assert_eq!(call.call_id, 7);
            assert!(call.open_bidi, "bidi open must set open_bidi");
            let request = call
                .request
                .expect("complete InvokeRequest rides the frame");
            assert_eq!(request.function_name, route.dispatch_key());
            assert_eq!(request.arguments, envelope_open.initial_args);
            assert_eq!(
                request
                    .envelope
                    .expect("envelope transplanted")
                    .callee
                    .expect("callee")
                    .ura,
                route.callee_ura,
                "resolver-selected callee must replace the caller-supplied one"
            );
        }
        other => panic!("expected DispatchCall on a v1 host, got {other:?}"),
    }

    // Cell 2: v1 host, no envelope → JSON (hollow canonical frame pin).
    let hollow = EnvelopeOpen {
        envelope: None,
        ..envelope_open.clone()
    };
    let frame = build_remote_bidi_open_frame_for_contract(true, 8, &route, &hollow)
        .expect("fallback frame builds");
    assert!(
        matches!(
            frame.frame.payload,
            Some(easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::BinaryChunk(_))
        ),
        "v1 host without an envelope must still ride JSON"
    );

    // Cell 3: v0 host → JSON regardless of envelope.
    let frame = build_remote_bidi_open_frame_for_contract(false, 9, &route, &envelope_open)
        .expect("v0 frame builds");
    assert!(
        matches!(
            frame.frame.payload,
            Some(easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::BinaryChunk(_))
        ),
        "v0 host keeps the JSON shape until the deletion window"
    );
}

#[test]
fn invoke_remote_up_request_serde_round_trip_via_session_dispatch_pin() {
    // Pins the invariant that PR-3 sub-spec §2.1 frame-0 JSON
    // (InvokeRemoteUp::Request) and PR-3 sub-spec §2.3 session
    // dispatch JSON (SessionDispatch::Dispatch) are *separate*
    // wire shapes. A regression that conflates them would let
    // a frame from one side decode into the other type — this
    // test asserts they don't.
    let req_json = serde_json::to_vec(&InvokeRemoteUp::Request {
        subject_device: "easynet:///r/realm/device/dev-B".into(),
        subject_ura: "easynet:///r/realm/device/dev-B".into(),
        ability_ura: "easynet:///r/realm/ability/device.dev-B.echo".into(),
        args: b"hi".to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: HashMap::new(),
        origin_caller: None,
    })
    .unwrap();
    // Decoding as the wrong type must fail.
    let mistaken: Result<SessionDispatch, _> = serde_json::from_slice(&req_json);
    assert!(
        mistaken.is_err(),
        "InvokeRemoteUp::Request must NOT decode as SessionDispatch — \
         the discriminator tags differ ('request' vs 'dispatch')"
    );
}

// dispatch_invoke_remote happy/sad-path integration tests
// require a real `tonic::Streaming<InvokeBidiUp>` which is
// gRPC-codegen-only constructible (no public `new_empty()`
// ctor). The same constraint that `#[ignore]`-marked
// `invoke_bidi_test_deferred_to_pr2_tier1` above applies here:
// those paths land as Tier 1 integration tests once PR-2's
// `session.open` accept enables a real round-trip. Until
// then the helpers below pin the units this method composes.
//
// Coverage assertion: every early-return code path of
// `dispatch_invoke_remote` is reachable from the helpers
// tested above:
//   * malformed initial_args → serde_json::from_slice (covered
//     by invoke_remote_up_request_serde_round_trip in
//     `invoke_remote_initiator::tests`)
//   * pending map None → trivial Option::ok_or_else (no-op
//     to test in isolation)
//   * target offline → PresenceRegistry::lookup returns None
//     (covered by presence_registry tests)
//   * try_send Full / Closed → matched by literal pattern,
//     same shape as commit 8/9's try_push_forward_invoke_frame
//     which is integration-tested
//   * pending oneshot dropped → covered by pending_dispatch
//     `dropped_completer_surfaces_to_handle_as_recv_error`

#[tokio::test]
async fn pending_stream_presence_offline_watcher_delivers_terminal_failure() {
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    let pending_stream = Arc::new(PendingStreamDispatchMap::new());
    let _svc = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending_stream(Arc::clone(&pending_stream));

    let target_ura = "easynet:///r/test-realm/device/target-stream";
    let mut handle = pending_stream.register_pending_for(target_ura);
    assert_eq!(
        pending_stream.try_push_chunk(handle.call_id(), b"partial".to_vec()),
        crate::daemon::invocation::state::pending_dispatch::StreamDeliver::Delivered
    );

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (sender, _rx) = tokio::sync::mpsc::channel::<
                Result<crate::daemon::invocation::state::presence::DispatchFrame, tonic::Status>,
            >(1);
            presence.insert(target_ura.to_string(), sender);
            presence.remove(
                target_ura,
                crate::daemon::invocation::state::presence::OfflineReason::StreamClosed,
            );

            match tokio::time::timeout(std::time::Duration::from_millis(20), handle.recv()).await {
                Ok(Some(crate::daemon::invocation::state::pending_dispatch::DispatchStreamEvent::Chunk(bytes))) => {
                    assert_eq!(bytes, b"partial");
                }
                Ok(Some(crate::daemon::invocation::state::pending_dispatch::DispatchStreamEvent::Terminal(
                    result,
                ))) => break result,
                Ok(None) => panic!("stream handle closed before terminal failure"),
                Err(_) => {
                    if pending_stream.outstanding() == 0 {
                        panic!("pending stream entry was removed without terminal delivery");
                    }
                }
            }
        }
    })
    .await
    .expect("presence offline watcher cancels the pending stream");

    assert!(terminal.payload.is_empty());
    assert_eq!(terminal.error.as_deref(), Some("target_offline"));
    let failure = terminal.failure.expect("typed terminal failure");
    assert_eq!(failure.code, "TARGET_OFFLINE");
    assert!(failure.retryable);
    assert_eq!(pending_stream.outstanding(), 0);
}

// ── PR-N1 commit 3a/N: federation client plumbing tests ──

#[test]
fn parse_realm_from_ura_extracts_realm_component() {
    assert_eq!(
        parse_realm_from_ura("easynet:///r/realm-a/device/laptop-1"),
        Some("realm-a".to_string())
    );
    assert_eq!(
        parse_realm_from_ura("easynet:///r/realm-a/device/device-1"),
        Some("realm-a".to_string())
    );
    assert_eq!(
        parse_realm_from_ura(&crate::ura::hub_ura("peer-realm")),
        Some("peer-realm".to_string())
    );
    assert_eq!(
        parse_realm_from_ura("easynet:///r/peer-realm/hub"),
        Some("peer-realm".to_string())
    );
    assert_eq!(
        parse_realm_from_ura("easynet:///r/peer-realm/hub/extra"),
        None
    );
}

#[test]
fn parse_realm_from_ura_rejects_noncanonical_extra_path_segments() {
    // Realm extraction goes through the canonical URA parser, so
    // malformed alias path tails no longer slip through.
    assert_eq!(
        parse_realm_from_ura("easynet:///r/realm-a/agent/n1/skill/foo"),
        None
    );
}

#[test]
fn parse_realm_from_ura_rejects_non_easynet_scheme() {
    assert_eq!(parse_realm_from_ura("https://example.com/foo"), None);
    assert_eq!(parse_realm_from_ura("file:///r/realm/agent/x"), None);
}

#[test]
fn parse_realm_from_ura_rejects_empty_realm() {
    // Malformed URA with empty realm component must reject —
    // never silently treat as `realm = ""` which would always
    // miss the federated_peers map and surface as
    // "realm unknown" instead of "URA malformed".
    assert_eq!(parse_realm_from_ura("easynet:///r//device/n1"), None);
}
