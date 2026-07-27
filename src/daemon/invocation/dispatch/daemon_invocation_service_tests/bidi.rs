use super::*;

const TEST_BIDI_ABILITY: &str = "test.bidi";

#[tokio::test]
async fn exact_bidi_route_family_registers_hub_owned_session_open() {
    let hub_ura = crate::core::ura::hub_ura("test-realm");
    let service = make_unregistered_service_for_route_owner(&hub_ura);
    service
        .register_daemon_bidi_routes(&hub_ura)
        .await
        .expect("register Hub exact bidi route family");
    let runtime = service
        .runtime
        .local_runtime()
        .expect("test service has shared LocalRuntime");
    let runtime_ability = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        &hub_ura,
        ABILITY_SESSION_OPEN,
    )
    .expect("session.open runtime key");
    let options = runtime
        .ability_options(&runtime_ability)
        .await
        .expect("session.open runtime options");
    assert!(matches!(
        options.backpressure,
        axon_sdk::invocation::BackpressurePolicy::Block { .. }
    ));
    let binding =
        crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility::from_wire_target(
            "daemon exact bidi registration test",
            runtime.as_ref(),
            &hub_ura,
            ABILITY_SESSION_OPEN,
        )
        .await
        .expect("session.open must be runtime-registered under the Hub");
    assert!(binding.supports_mode(CallMode::Bidi));
    binding
        .descriptor_ref_for_mode(
            "daemon exact bidi registration test",
            &hub_ura,
            CallMode::Bidi,
            None,
        )
        .expect("session.open must retain its governed Bidi descriptor proof");
}

#[tokio::test]
async fn exact_bidi_route_registration_rejects_device_owner() {
    let service = make_unregistered_service_for_route_owner(TEST_DAEMON_URA);
    let error = service
        .register_daemon_bidi_routes(TEST_DAEMON_URA)
        .await
        .expect_err("session.open cannot register under a Device owner");
    assert!(
        error
            .to_string()
            .contains("canonical realm Authority owner"),
        "{error}"
    );
}

fn forwarded_binary_chunk(frame: LocalBidiHandlerFrame) -> BinaryChunk {
    match frame {
        LocalBidiHandlerFrame::Forward(frame) => match (*frame).payload {
            Some(DownPayload::BinaryChunk(chunk)) => chunk,
            other => panic!("expected forwarded BinaryChunk, got {other:?}"),
        },
        other => panic!("expected forwarded BinaryChunk, got {other:?}"),
    }
}

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
            TEST_BIDI_ABILITY,
            b"{}".to_vec(),
        ))),
    };
    let eo = extract_envelope_open(&frame).expect("extracted");
    assert_eq!(
        crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "test bidi frame",
            eo.target.as_ref(),
        )
        .unwrap(),
        TEST_BIDI_ABILITY
    );
}

#[test]
fn validate_and_extract_bidi_frame0_rejects_non_zero_sequence() {
    let frame = InvokeBidiUp {
        sequence: 7,
        mac: Vec::new(),
        payload: Some(UpPayload::EnvelopeOpen(make_envelope_open(
            TEST_BIDI_ABILITY,
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
    let mut envelope_open = make_envelope_open(TEST_BIDI_ABILITY, b"{}".to_vec());
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
    let chunk = forwarded_binary_chunk(frame);
    assert_eq!(chunk.stream_id, 7);
    assert_eq!(chunk.data, b"hello");
}

#[test]
fn map_local_bidi_handler_exit_remains_data_until_runtime_terminal() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::Pty,
        &serde_json::json!({
            "type": "exit",
            "status": 23,
        }),
        1,
    );
    let chunk = forwarded_binary_chunk(frame);
    let payload: serde_json::Value =
        serde_json::from_slice(&chunk.data).expect("exit JSON payload");
    assert_eq!(payload["type"], "exit");
    assert_eq!(payload["status"], 23);
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
    let chunk = forwarded_binary_chunk(frame);
    assert_eq!(chunk.stream_id, 11);
    assert_eq!(chunk.data, b"file-bytes");
}

#[test]
fn map_local_bidi_handler_file_transfer_complete_remains_data_until_runtime_terminal() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::FileTransfer,
        &serde_json::json!({
            "type": "complete",
            "sha256": "deadbeef",
            "bytes": 9,
        }),
        1,
    );
    let chunk = forwarded_binary_chunk(frame);
    let payload: serde_json::Value = serde_json::from_slice(&chunk.data).expect("json payload");
    assert_eq!(payload["sha256"], "deadbeef");
    assert_eq!(payload["bytes"], 9);
}

#[test]
fn map_local_bidi_handler_file_transfer_error_remains_data_until_runtime_terminal() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::FileTransfer,
        &serde_json::json!({
            "type": "error",
            "code": "disk_full",
            "message": "no space left on device",
        }),
        1,
    );
    let chunk = forwarded_binary_chunk(frame);
    let payload: serde_json::Value = serde_json::from_slice(&chunk.data).expect("json payload");
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["code"], "disk_full");
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
            control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
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
    let chunk = forwarded_binary_chunk(frame);
    assert_eq!(chunk.stream_id, 3);
    let payload: serde_json::Value =
        serde_json::from_slice(&chunk.data).expect("json frame payload");
    assert_eq!(payload["type"], "frame");
    assert_eq!(payload["seq"], 7);
    assert_eq!(payload["image_bytes_b64"], "abc");
}

#[test]
fn map_local_bidi_handler_json_error_remains_data_until_runtime_terminal() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::JsonFrames,
        &serde_json::json!({
            "type": "error",
            "code": "permission_denied",
            "message": "screen capture permission denied",
        }),
        3,
    );
    let chunk = forwarded_binary_chunk(frame);
    let payload: serde_json::Value = serde_json::from_slice(&chunk.data).expect("json payload");
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["code"], "permission_denied");
}

#[test]
fn map_local_bidi_handler_json_closed_remains_data_until_runtime_terminal() {
    let frame = map_local_bidi_handler_frame(
        LocalBidiWireKind::JsonFrames,
        &serde_json::json!({
            "type": "closed",
            "reason": "client_closed",
        }),
        3,
    );
    let chunk = forwarded_binary_chunk(frame);
    let payload: serde_json::Value = serde_json::from_slice(&chunk.data).expect("json payload");
    assert_eq!(payload["type"], "closed");
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
    let chunk = forwarded_binary_chunk(frame);
    assert_eq!(chunk.stream_id, 9);
    assert_eq!(chunk.data, b"\xff\xd8raw-jpeg\xff\xd9");
}

#[test]
fn map_local_bidi_ability_terminal_defers_to_runtime_receipt_projection() {
    let frame = map_local_bidi_ability_frame(
        LocalBidiWireKind::JsonFrames,
        AbilityFrame {
            payload: br#"{"type":"closed"}"#.to_vec(),
            content_type: "application/json".to_string(),
            terminal: true,
        },
        9,
    );
    assert!(matches!(frame, LocalBidiHandlerFrame::Terminal));
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
async fn local_bidi_down_stream_preserves_supplied_initial_frame_before_handler_frames() {
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

    let initial = InvokeBidiDown {
        payload: Some(DownPayload::Control(BidiControl::default())),
        ..InvokeBidiDown::default()
    };
    let mut stream = LocalBidiDownStream::with_admission(down_rx, initial);
    let first = stream
        .next()
        .await
        .expect("initial frame")
        .expect("frame is ok");
    match first.payload {
        Some(DownPayload::Control(_)) => {
            assert_eq!(first.sequence, 0);
        }
        other => panic!("expected supplied initial frame at sequence 0, got {other:?}"),
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

/// Remote bidi open has exactly one frame shape: canonical DispatchCall.
/// Missing envelopes and non-bidi call modes fail closed; neither can fall
/// back to the retired JSON BidiOpen projection.
#[tokio::test]
async fn remote_bidi_open_frame_is_canonical_and_fail_closed() {
    use axon_sdk::pb::axon::v1::EnvelopeOpen;

    let svc = make_service().with_session_realm("test-realm");
    let target_ura = "easynet:///r/test-realm/device/bidi-target";
    publish_test_route(&svc, target_ura, "remote_desktop.attach");
    let route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(target_ura, "remote_desktop.attach")
        .expect("published route resolves");

    let initial_args = br#"{"session_id":"rd-9"}"#.to_vec();
    let envelope_open = EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            "easynet:///r/test-realm/user/alice",
            target_ura,
            target_ura,
            "remote_desktop.attach",
            &initial_args,
            &test_device_signing_key(),
        )),
        target: Some(
            wire_invocation_target(
                test_descriptor_ref(target_ura, "remote_desktop.attach"),
                "remote_desktop.attach",
            )
            .expect("typed descriptor target"),
        ),
        initial_args,
        ..Default::default()
    };

    let frame = build_remote_bidi_open_frame(7, &route, &envelope_open, CallMode::Bidi)
        .expect("canonical bidi frame builds");
    match frame.frame.payload.expect("payload") {
        axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::DispatchCall(call) => {
            assert_eq!(call.call_id, 7);
            assert!(call.open_bidi, "bidi open must set open_bidi");
            let request = call
                .request
                .expect("complete InvokeRequest rides the frame");
            assert_eq!(invocation_function_name(&request), route.dispatch_key());
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
        other => panic!("expected canonical DispatchCall, got {other:?}"),
    }

    let hollow = EnvelopeOpen {
        envelope: None,
        ..envelope_open.clone()
    };
    let err = build_remote_bidi_open_frame(8, &route, &hollow, CallMode::Bidi)
        .expect_err("hollow canonical frame must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let err = build_remote_bidi_open_frame(9, &route, &envelope_open, CallMode::Rpc)
        .expect_err("non-bidi route cannot construct a bidi-open carrier");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test]
async fn remote_bidi_rejects_governance_catalogue_route_before_carrier_frame() {
    use axon_sdk::pb::axon::v1::EnvelopeOpen;

    const TARGET_DEVICE_URA: &str = "easynet:///r/test-realm/device/bidi-catalogue-target";
    const CATALOGUE_READ: &str = crate::daemon::ability::names::governance::META_LIST_ABILITIES;

    let svc = make_service().with_session_realm("test-realm");
    publish_test_route_with_mode(
        &svc,
        TARGET_DEVICE_URA,
        CATALOGUE_READ,
        crate::daemon::ability::CallMode::Bidi,
    );
    let route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(TARGET_DEVICE_URA, CATALOGUE_READ)
        .expect("published bidi catalogue route resolves before governance gate");

    let initial_args = br#"{"scope":"local"}"#.to_vec();
    let descriptor_ref = test_descriptor_ref(TARGET_DEVICE_URA, CATALOGUE_READ);
    let envelope_open = EnvelopeOpen {
        envelope: Some(signed_test_envelope_with_descriptor_ref(
            "easynet:///r/test-realm/user/alice",
            TARGET_DEVICE_URA,
            TARGET_DEVICE_URA,
            descriptor_ref.clone(),
            &initial_args,
            &test_device_signing_key(),
        )),
        target: Some(
            wire_invocation_target(&descriptor_ref, CATALOGUE_READ)
                .expect("typed descriptor target"),
        ),
        initial_args,
        ..Default::default()
    };

    let err = build_remote_bidi_open_frame(10, &route, &envelope_open, CallMode::Bidi)
        .expect_err("bidi catalogue read must use the unary catalogue path");

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("CANONICAL_CATALOGUE_READ_REQUIRED")
            && err.message().contains("InvokeBidi")
            && err.message().contains(CATALOGUE_READ)
            && err
                .message()
                .contains("canonical unary Invoke catalogue read path"),
        "unexpected bidi catalogue route denial: {}",
        err.message()
    );
}

// Remote canonical relay coverage lives in canonical_relay.rs.
// Legacy JSON dispatch tests were removed with the wrapper protocol.

#[tokio::test]
async fn pending_stream_presence_offline_watcher_delivers_terminal_failure() {
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URA.to_string()),
    );
    let pending_stream = Arc::new(PendingStreamDispatchMap::new());
    let _svc = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending_stream(Arc::clone(&pending_stream));

    let target_ura = "easynet:///r/test-realm/device/target-stream";
    let mut handle = pending_stream.register_pending_for(target_ura);
    assert_eq!(
        pending_stream.try_push_chunk(handle.call_id(), b"partial".to_vec()),
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered
    );

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (sender, _rx) = tokio::sync::mpsc::channel::<
                Result<crate::daemon::invocation::bidi::state::presence::DispatchFrame, tonic::Status>,
            >(1);
            insert_test_dispatch_presence(&presence, target_ura, sender)
                .expect("canonical presence key");
            presence.remove(
                target_ura,
                crate::daemon::invocation::bidi::state::presence::OfflineReason::StreamClosed,
            );

            match tokio::time::timeout(std::time::Duration::from_millis(20), handle.recv()).await {
                Ok(Some(crate::daemon::invocation::bidi::state::pending_dispatch::DispatchStreamEvent::Chunk(bytes))) => {
                    assert_eq!(bytes, b"partial");
                }
                Ok(Some(crate::daemon::invocation::bidi::state::pending_dispatch::DispatchStreamEvent::Admission(_))) => {
                    panic!("offline cancellation fixture did not dispatch an invocation");
                }
                Ok(Some(crate::daemon::invocation::bidi::state::pending_dispatch::DispatchStreamEvent::Terminal(
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
fn realm_from_ura_extracts_realm_component() {
    assert_eq!(
        realm_from_ura("easynet:///r/realm-a/device/laptop-1"),
        Some("realm-a".to_string())
    );
    assert_eq!(
        realm_from_ura("easynet:///r/realm-a/device/device-1"),
        Some("realm-a".to_string())
    );
    assert_eq!(
        realm_from_ura(&crate::core::ura::hub_ura("peer-realm")),
        Some("peer-realm".to_string())
    );
    assert_eq!(
        realm_from_ura("easynet:///r/peer-realm/authority"),
        Some("peer-realm".to_string())
    );
    assert_eq!(
        realm_from_ura("easynet:///r/peer-realm/authority/extra"),
        None
    );
}

#[test]
fn realm_from_ura_rejects_noncanonical_extra_path_segments() {
    // Realm extraction goes through the canonical URA parser, so
    // malformed alias path tails no longer slip through.
    assert_eq!(
        realm_from_ura("easynet:///r/realm-a/agent/n1/skill/foo"),
        None
    );
}

#[test]
fn realm_from_ura_rejects_non_easynet_scheme() {
    assert_eq!(realm_from_ura("https://example.com/foo"), None);
    assert_eq!(realm_from_ura("file:///r/realm/agent/x"), None);
}

#[test]
fn realm_from_ura_rejects_empty_realm() {
    // Malformed URA with empty realm component must reject —
    // never silently treat as `realm = ""` which would always
    // miss the federated_peers map and surface as
    // "realm unknown" instead of "URA malformed".
    assert_eq!(realm_from_ura("easynet:///r//device/n1"), None);
}
