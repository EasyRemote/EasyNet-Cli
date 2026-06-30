use super::*;

#[tokio::test]
async fn invoke_stream_dispatches_subscribe_directory_initial_frame_then_pump() {
    use futures::StreamExt;

    // Build the service with our own presence Arc so the test
    // can drive the broadcast sender's close behaviour via Arc
    // drop (the pump only ends when *every* sender drops; the
    // pump itself holds a Weak so dropping the last Arc here
    // closes the channel cleanly).
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    let svc = DaemonInvocationService::new(Arc::clone(&presence), admission);

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY.to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("subscribe_directory initial frame returns Ok");

    let mut stream = resp.into_inner();

    // Frame 1 — the initial empty snapshot.
    let first = stream
        .next()
        .await
        .expect("at least one frame")
        .expect("frame is Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    let initial: federation_wrappers::SubscribeDirectoryInitial =
        serde_json::from_slice(&first.payload).expect("decodes initial");
    assert!(initial.agents.is_empty());

    // Frame 2 — an Online delta after a registry insert is
    // pumped through the broadcast subscriber.
    let (sender, _rx) = tokio::sync::mpsc::channel::<
        Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
    >(1);
    presence.insert("easynet:///r/test-realm/device/n1".to_string(), sender);

    let second = stream
        .next()
        .await
        .expect("delta frame after insert")
        .expect("frame is Ok");
    let delta: serde_json::Value = serde_json::from_slice(&second.payload).expect("decodes");
    assert_eq!(delta.get("kind").and_then(|v| v.as_str()), Some("online"));
    assert_eq!(
        delta.get("membership_ura").and_then(|v| v.as_str()),
        Some("easynet:///r/test-realm/device/n1"),
    );

    // Drop both Arcs holding the broadcast sender so the pump
    // sees `RecvError::Closed` on its next poll and yields None.
    // Without this the stream is intentionally infinite.
    drop(svc);
    drop(presence);

    // Now the pump must close. Bound the wait so a real bug
    // here surfaces as a test failure, not a CI hang.
    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("pump closes within 2 s after senders drop");
    assert!(
        close.is_none(),
        "stream must terminate once all senders drop"
    );
}

#[tokio::test]
async fn invoke_stream_dispatches_subscribe_directory_v2_emits_directory_events() {
    // PR-N3 N3-streaming-1. v2 stream emits DirectoryEvent
    // shapes (Snapshot first, then Upsert/Remove).
    use crate::services::federation_directory::DirectoryEvent;
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    let svc = DaemonInvocationService::new(Arc::clone(&presence), admission);

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2.to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("v2 dispatch returns Ok");

    let mut stream = resp.into_inner();

    // Frame 1: empty Snapshot (registry has no entries yet).
    let first = stream.next().await.expect("first frame").expect("Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    let evt: DirectoryEvent =
        serde_json::from_slice(&first.payload).expect("decodes DirectoryEvent");
    match evt {
        DirectoryEvent::Snapshot { agents, .. } => {
            assert!(
                agents.is_empty(),
                "initial snapshot must reflect empty registry"
            );
        }
        other => panic!("expected Snapshot first; got {other:?}"),
    }

    // Frame 2: AgentAdvertised after a registry insert.
    let (sender, _rx) = tokio::sync::mpsc::channel::<
        Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
    >(1);
    presence.insert("easynet:///r/test-realm/device/n1".to_string(), sender);
    let second = stream.next().await.expect("second frame").expect("Ok");
    let evt2: DirectoryEvent =
        serde_json::from_slice(&second.payload).expect("decodes DirectoryEvent");
    match evt2 {
        DirectoryEvent::AgentAdvertised {
            agent_ura,
            signing_authority,
            ..
        } => {
            assert_eq!(agent_ura, "easynet:///r/test-realm/device/n1");
            assert_eq!(
                signing_authority,
                crate::services::federation_directory::SigningAuthority::SelfSigned
            );
        }
        other => panic!("expected AgentAdvertised; got {other:?}"),
    }

    // Frame 3: AgentRevoked after the device's stream closes (we
    // drop the receiver to trigger the Closed path).
    // PresenceRegistry's drop-on-receiver-close behaviour is
    // exercised by the existing v1 test; here we just
    // explicitly remove via the registry surface.
    presence.remove(
        "easynet:///r/test-realm/device/n1",
        crate::services::presence_registry::OfflineReason::AdminRevoked,
    );
    let third = stream.next().await.expect("third frame").expect("Ok");
    let evt3: DirectoryEvent =
        serde_json::from_slice(&third.payload).expect("decodes DirectoryEvent");
    match evt3 {
        DirectoryEvent::AgentRevoked {
            agent_ura, reason, ..
        } => {
            assert_eq!(agent_ura, "easynet:///r/test-realm/device/n1");
            assert_eq!(reason, "admin_revoked");
        }
        other => panic!("expected AgentRevoked; got {other:?}"),
    }

    // Drop senders → pump closes.
    drop(svc);
    drop(presence);
    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("pump closes within 2 s");
    assert!(close.is_none());
}

#[tokio::test]
async fn invoke_stream_subscribe_directory_v2_emits_heartbeat_when_idle() {
    // PR-N3 N3-streaming-6. Confirm the v2 stream emits a
    // DirectoryEvent::Heartbeat after the heartbeat
    // interval has elapsed with no real events, so the
    // subscriber's 60s idle-timeout watcher does not tear
    // down a healthy stream. The test sets a 50ms cadence
    // via `with_subscribe_v2_heartbeat_interval_ms` so it
    // runs in real time without virtualised clocks; spec
    // §2.3 production cadence is 30 000ms.
    use crate::services::federation_directory::DirectoryEvent;
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    let svc = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_subscribe_v2_heartbeat_interval_ms(std::num::NonZeroU64::new(50).unwrap());

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2.to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("dispatch returns Ok");

    let mut stream = resp.into_inner();

    // Frame 1: empty Snapshot (immediate).
    let first = stream.next().await.expect("first frame").expect("Ok");
    let evt: DirectoryEvent = serde_json::from_slice(&first.payload).expect("Snapshot decodes");
    assert!(matches!(evt, DirectoryEvent::Snapshot { .. }));

    // Frame 2: Heartbeat after the 50ms interval. Bound
    // the wait to 1s so a real bug surfaces as a test
    // timeout rather than a CI hang.
    let hb_frame = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("heartbeat frame within 1s")
        .expect("stream did not end")
        .expect("frame is Ok");
    let hb_evt: DirectoryEvent =
        serde_json::from_slice(&hb_frame.payload).expect("Heartbeat decodes");
    match hb_evt {
        DirectoryEvent::Heartbeat { unix_ms } => {
            assert!(unix_ms > 0, "Heartbeat unix_ms must be a real epoch-ms",);
        }
        other => panic!("expected Heartbeat after idle window; got {other:?}"),
    }

    drop(svc);
    drop(presence);
}

#[tokio::test]
async fn invoke_stream_dispatches_registered_local_stream_ability() {
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode, LocalRuntime};
    use futures::StreamExt;

    let rt = LocalRuntime::new();
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, "browser.capture_viewport")
            .expect("device stream ability URA");
    // Stream-mode descriptor proof so Axon's receipt-proof normalizer
    // admits the dispatch (production stamps the equivalent off the
    // control-plane record). register_streaming_ability leaves the proof
    // unbound, so register with explicit stream-mode options.
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|ctx| async move {
            let args: serde_json::Value =
                serde_json::from_slice(&ctx.payload).unwrap_or(serde_json::Value::Null);
            let envelope = ctx
                .runtime
                .axiom_envelope_of(&ctx.invocation_id)
                .await
                .expect("runtime stores descriptor-bound stream envelope")
                .envelope;
            Ok(serde_json::to_vec(&serde_json::json!({
                "MARKER-LOCAL-STREAM": "dispatched",
                "caller": envelope.caller.ura,
                "subject": envelope.subject.ura,
                "session_ura": args.get("session_ura").and_then(|v| v.as_str()),
            }))
            .unwrap())
        }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "browser.capture_viewport");

    let function_name = "browser.capture_viewport".to_string();
    let arguments = br#"{"session_ura":"easynet:///r/local/resource/daemon.browser/s1"}"#.to_vec();
    let subject_ura = "easynet:///r/test-realm/resource/browser.capture/s1";
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URI,
                    subject_ura,
                )
                .expect("valid unsigned loopback stream envelope")
                .into_inner(),
            ),
            function_name,
            arguments,
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("registered local stream returns Ok");

    let mut stream = resp.into_inner();
    let first = stream.next().await.expect("one frame").expect("frame Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    assert_eq!(
        first
            .header
            .as_ref()
            .map(|header| header.request_id.as_str()),
        Some(first.invocation_id.as_str()),
        "InvokeStream chunks must expose the Axon request id for ledger lookup"
    );
    assert!(
        first.invocation_id.starts_with("inv_"),
        "local stream invocation id must be projected"
    );
    assert!(
        first
            .selected_node_id
            .starts_with("route-ref::easynet:///r/test-realm/ability/device.test-daemon."),
        "selected_node_id should carry the chosen route ref, got {}",
        first.selected_node_id
    );
    assert_eq!(first.sequence, 0);
    assert_eq!(
        first.state,
        easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
    );
    assert!(
        first.admission_receipt.is_some(),
        "first stream chunk must expose the admission receipt"
    );
    assert!(
        first.terminal_receipt.is_some(),
        "terminal stream chunk must expose the terminal receipt"
    );
    assert!(
        first.terminal,
        "local snapshot stream must preserve terminal=true on the daemon InvokeStream chunk"
    );
    let frame: serde_json::Value = serde_json::from_slice(&first.payload).expect("JSON frame");
    assert_eq!(
        frame
            .get("MARKER-LOCAL-STREAM")
            .and_then(|value| value.as_str()),
        Some("dispatched")
    );
    assert_eq!(
        frame.get("caller").and_then(|value| value.as_str()),
        Some(crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA)
    );
    assert_eq!(
        frame.get("subject").and_then(|value| value.as_str()),
        Some(subject_ura)
    );
    assert_eq!(
        frame.get("session_ura").and_then(|value| value.as_str()),
        Some("easynet:///r/local/resource/daemon.browser/s1")
    );

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("snapshot stream closes promptly");
    assert!(close.is_none());
}

#[tokio::test]
async fn invoke_stream_accepts_descriptor_ref_function_name() {
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode, LocalRuntime};
    use futures::StreamExt;

    let ability = "browser.descriptor_stream";
    let rt = LocalRuntime::new();
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, ability).expect("stream ability URA");
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();

    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, ability);

    let arguments = br#"{"descriptor":"stream-function-name"}"#.to_vec();
    let descriptor_ref = test_descriptor_ref(TEST_DAEMON_URI, ability);
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URI,
                    TEST_DAEMON_URI,
                )
                .expect("valid unsigned loopback stream envelope")
                .into_inner(),
            ),
            function_name: descriptor_ref,
            arguments: arguments.clone(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("descriptor-ref stream function_name dispatches");

    let mut stream = resp.into_inner();
    let first = stream.next().await.expect("one frame").expect("frame Ok");
    assert!(first.terminal);
    assert_eq!(first.payload, arguments);
}

#[tokio::test]
async fn invoke_stream_projects_empty_payload_terminal_frame_for_registry_snapshot() {
    use crate::runtime::ability_dispatch::{
        AbilityAuthorityContext, AxonAbilityCatalog, LocalStreamHandler, OwnerKind, StreamSource,
    };
    use easynet_axon::invocation::LocalRuntime;
    use futures::StreamExt;

    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        LocalRuntime::new(),
        AbilityAuthorityContext::for_device_authority_root(TEST_DAEMON_URI)
            .expect("test daemon URI is a valid device authority root"),
    );
    let handler: LocalStreamHandler = Arc::new(|_args| {
        Ok(StreamSource::Snapshot(vec![serde_json::json!({
            "MARKER-SNAPSHOT": "progress-before-empty-terminal"
        })]))
    });
    catalog.register_stream_with_owner("browser.snapshot_once", OwnerKind::Device, handler);
    let rt = catalog.runtime().expect("catalog attaches a LocalRuntime");
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "browser.snapshot_once");

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URI,
                    TEST_DAEMON_URI,
                )
                .expect("valid unsigned loopback stream envelope")
                .into_inner(),
            ),
            function_name: "browser.snapshot_once".to_string(),
            arguments: br#"{}"#.to_vec(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("registered snapshot stream returns Ok");

    let mut stream = resp.into_inner();
    let first = stream
        .next()
        .await
        .expect("snapshot progress frame")
        .expect("snapshot progress frame is Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    assert!(
        !first.terminal,
        "StreamSource::Snapshot progress frame must not be terminal"
    );
    let first_json: serde_json::Value =
        serde_json::from_slice(&first.payload).expect("snapshot JSON frame");
    assert_eq!(
        first_json
            .get("MARKER-SNAPSHOT")
            .and_then(|value| value.as_str()),
        Some("progress-before-empty-terminal")
    );

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("empty terminal frame arrives within 2s")
        .expect("empty terminal frame is projected")
        .expect("empty terminal frame is Ok");
    assert_eq!(second.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    assert!(
        second.terminal,
        "daemon projection must preserve terminal=true even when payload is empty"
    );
    assert!(
        second.payload.is_empty(),
        "finite StreamSource completion is represented by an empty terminal payload"
    );

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("stream closes promptly after terminal");
    assert!(close.is_none());
}

#[tokio::test]
async fn invoke_stream_dispatches_non_default_descriptor_version() {
    // Regression for the version-convergence gap: a stream ability
    // registered at a NON-default descriptor version must dispatch. The
    // dispatcher reads the Stream-mode proof version and stamps the wire
    // ref accordingly; before the fix, reassembly forced 1.0.0 and Axon
    // rejected the call as proof_descriptor_version_mismatch.
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode, LocalRuntime};
    use futures::StreamExt;

    const NON_DEFAULT_VERSION: &str = "2.0.0";
    let rt = LocalRuntime::new();
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, "browser.capture_viewport")
            .expect("device stream ability URA");
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|_ctx| async move {
            Ok(serde_json::to_vec(&serde_json::json!({"MARKER-V2": "dispatched"})).unwrap())
        }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            NON_DEFAULT_VERSION,
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "browser.capture_viewport");

    let function_name = "browser.capture_viewport".to_string();
    let arguments = br#"{}"#.to_vec();
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URI,
                    TEST_DAEMON_URI,
                )
                .expect("valid loopback stream envelope")
                .into_inner(),
            ),
            function_name,
            arguments,
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("non-default-version stream dispatch returns Ok");
    let mut stream = resp.into_inner();
    let first = stream.next().await.expect("one frame").expect("frame Ok");
    let frame: serde_json::Value = serde_json::from_slice(&first.payload).expect("JSON frame");
    assert_eq!(
        frame.get("MARKER-V2").and_then(|v| v.as_str()),
        Some("dispatched"),
        "the 2.0.0-versioned stream handler must have run: {frame}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_signed_bidi_file_transfer_download_emits_business_frames() {
    use base64::Engine as _;
    use easynet_axon::invocation::LocalRuntime;

    let rt = LocalRuntime::new();
    let mut catalog =
        crate::runtime::ability_dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&rt),
            crate::runtime::ability_dispatch::AbilityAuthorityContext::for_device_authority_root(
                TEST_DAEMON_URI,
            )
            .expect("test daemon URI is a valid device authority root"),
        );
    crate::runtime::agents::file_transfer_ability::register(&mut catalog);

    let path = std::env::temp_dir().join(format!(
        "easynet-external-signed-bidi-download-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let bytes = b"external-signed-bidi-download-proof";
    std::fs::write(&path, bytes).unwrap();

    let args = serde_json::to_vec(&serde_json::json!({
        "mode": "download",
        "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
            &path,
            crate::runtime::resources::filesystem::FilesystemResourceCapability::Read,
        )
        .expect("local fs ResourceRef"),
    }))
    .unwrap();
    let file_transfer_descriptor_ref = test_descriptor_ref(
        TEST_DAEMON_URI,
        crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
    );
    let open = make_envelope_open(&file_transfer_descriptor_ref, args);
    let wire =
        crate::runtime::axon_bridge::dispatch_shim::external_signed_from_envelope_open(&open)
            .expect("wire dispatch");
    let handle = crate::runtime::axon_bridge::dispatch_shim::open_bidi_external_signed(&rt, wire)
        .await
        .expect("open external-signed bidi");
    let (input, mut output) = handle.split();

    input
        .send(
            BidiInputFrame::new(serde_json::to_vec(&serde_json::json!({"type":"eof"})).unwrap())
                .with_content_type("application/json"),
        )
        .await
        .expect("send ready/eof");
    let _ = input.close_input().await;

    let mut downloaded = Vec::new();
    let mut got_complete = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or_default();
        let Some(frame) = tokio::time::timeout(remaining, output.next_frame())
            .await
            .expect("bidi output poll should not time out")
        else {
            break;
        };
        let frame = frame.expect("bidi frame ok");
        if frame.payload.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_slice(&frame.payload).expect("file transfer JSON frame");
        match value["type"].as_str() {
            Some("chunk") => {
                let chunk = value["data"].as_str().expect("chunk data");
                downloaded.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(chunk)
                        .expect("chunk base64"),
                );
            }
            Some("complete") => {
                got_complete = true;
                break;
            }
            other => panic!("unexpected file_transfer frame {other:?}: {value}"),
        }
    }
    assert!(
        got_complete,
        "external-signed file_transfer download must emit complete"
    );
    assert_eq!(downloaded, bytes);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn invoke_stream_unknown_function_returns_resolver_negative() {
    let svc = make_service();
    match svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: "custom.stream.ability".to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::FailedPrecondition);
            assert!(
                err.message().contains(ROUTE_NEGATIVE_CODE),
                "expected resolver negative; got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("unknown stream ability must be rejected"),
    }
}

#[tokio::test]
async fn invoke_rejects_caller_not_in_trust_anchor() {
    // Trust-anchor membership is the first non-loopback check. A
    // URA absent from the anchor short-circuits to
    // `permission_denied` before any signature or replay work.
    let svc = DaemonInvocationService::new(
        Arc::new(PresenceRegistry::new()),
        AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None),
    );
    match svc
        .invoke(Request::new(InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/realm/agent/test.external".to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            function_name: ABILITY_FEDERATION_HEARTBEAT.to_string(),
            arguments: br#"{"agent_ura":"easynet:///r/realm/agent/test.external"}"#.to_vec(),
            ..InvokeRequest::default()
        }))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::PermissionDenied);
            assert!(
                err.message().contains("not in the realm trust anchor"),
                "rejection must reference trust-set miss, got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("caller outside trust anchor must be rejected"),
    }
}

#[tokio::test]
async fn invoke_stream_rejects_caller_not_in_trust_anchor() {
    // Stream surface shares the same membership check as unary.
    let svc = DaemonInvocationService::new(
        Arc::new(PresenceRegistry::new()),
        AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None),
    );
    match svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/realm/agent/test.external".to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY.to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::PermissionDenied);
            assert!(
                err.message().contains("not in the realm trust anchor"),
                "rejection must reference trust-set miss, got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("stream caller outside trust anchor must be rejected"),
    }
}

#[ignore = "PR-1 staging — bidi accept/dispatch covered by PR-2 Tier 1 cases 1-11 unignore"]
#[tokio::test]
async fn invoke_bidi_test_deferred_to_pr2_tier1() {
    // Constructing a real `tonic::Streaming<InvokeBidiUp>`
    // requires the full tonic codegen scaffolding. The
    // unimplemented path returns before reading any frame,
    // so a synthetic empty `Streaming` would not exercise
    // anything beyond the trait dispatch table — exactly
    // what PR-2 Tier 1 cases 1-11 cover end-to-end via real
    // gRPC roundtrip. Marking this `#[ignore]` so the test
    // result line surfaces the gap rather than passing
    // vacuously.
    unreachable!();
}
