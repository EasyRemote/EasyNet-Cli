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
    use easynet_axon::invocation::{make_ability, LocalRuntime};
    use futures::StreamExt;

    let rt = LocalRuntime::new();
    rt.register_streaming_ability(
        "browser.capture_viewport",
        make_ability(|ctx| async move {
            let args: serde_json::Value =
                serde_json::from_slice(&ctx.payload).unwrap_or(serde_json::Value::Null);
            Ok(serde_json::to_vec(&serde_json::json!({
                "MARKER-LOCAL-STREAM": "dispatched",
                "session_ura": args.get("session_ura").and_then(|v| v.as_str()),
            }))
            .unwrap())
        }),
    )
    .await
    .unwrap();
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "browser.capture_viewport");

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: "browser.capture_viewport".to_string(),
            arguments: br#"{"session_ura":"easynet:///r/local/resource/daemon.browser/s1"}"#
                .to_vec(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("registered local stream returns Ok");

    let mut stream = resp.into_inner();
    let first = stream.next().await.expect("one frame").expect("frame Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
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
        frame.get("session_ura").and_then(|value| value.as_str()),
        Some("easynet:///r/local/resource/daemon.browser/s1")
    );

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("snapshot stream closes promptly");
    assert!(close.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_bidi_file_transfer_download_emits_business_frames() {
    use base64::Engine as _;
    use easynet_axon::invocation::LocalRuntime;

    let rt = LocalRuntime::new();
    let mut catalog =
        crate::runtime::ability_dispatch::AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt));
    crate::runtime::agents::file_transfer_ability::register(&mut catalog);

    let path = std::env::temp_dir().join(format!(
        "easynet-admitted-bidi-download-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let bytes = b"admitted-bidi-download-proof";
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
    let open = make_envelope_open(
        crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
        args,
    );
    let wire = crate::runtime::axon_bridge::dispatch_shim::admitted_from_envelope_open(&open)
        .expect("wire dispatch");
    let handle = crate::runtime::axon_bridge::dispatch_shim::open_bidi_admitted(&rt, wire)
        .await
        .expect("open admitted bidi");
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
        "admitted file_transfer download must emit complete"
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
    // PR-7 commit 4/N (DEC-013 Option D): trust-anchor membership
    // is the first non-loopback check. A URA absent from the
    // anchor short-circuits to `permission_denied` before any
    // §5.2 work — the gating reject, identical to the PR-1 URA-
    // only behaviour for unknown callers. Same `PermissionDenied`
    // wire code as before, refreshed message text.
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
    // Same DEC-013 dispatch as `invoke_rejects_caller_not_in_trust_anchor`.
    // Stream surface shares the same membership check.
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
