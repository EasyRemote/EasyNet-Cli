use super::*;

#[tokio::test]
async fn invoke_stream_dispatches_subscribe_directory_initial_frame_then_pump() {
    use futures::StreamExt;

    // Build the service with our own presence Arc so the test can drive the
    // product broadcast pump while exact stream dispatch still enters through
    // the production route registration path.
    let presence = Arc::new(PresenceRegistry::new());
    let svc = make_service_with_presence(Arc::clone(&presence));

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
        Result<crate::daemon::invocation::bidi::state::presence::DispatchFrame, tonic::Status>,
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

    drop(svc);
    drop(presence);

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("pump closes within 2 s after route lifecycle drops");
    let terminal = close
        .expect("route lifecycle close yields terminal chunk")
        .expect("terminal chunk is Ok");
    assert!(terminal.terminal);
    assert!(terminal.terminal_receipt.is_some());
    let end = stream.next().await;
    assert!(end.is_none());
}

#[tokio::test]
async fn invoke_stream_subscribe_directory_closes_when_route_lifecycle_drops() {
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let svc = make_service_with_presence(Arc::clone(&presence));
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY.to_string(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("subscribe_directory stream starts");
    let mut stream = resp.into_inner();
    let _first = stream.next().await.expect("snapshot").expect("Ok");

    drop(svc);
    drop(presence);

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("route lifecycle closes v1 stream within 2 s");
    let terminal = close
        .expect("route lifecycle close yields terminal chunk")
        .expect("terminal chunk is Ok");
    assert!(terminal.terminal);
    assert!(terminal.terminal_receipt.is_some());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn invoke_stream_dispatches_subscribe_directory_v2_emits_directory_events() {
    // PR-N3 N3-streaming-1. v2 stream emits DirectoryEvent
    // shapes (Snapshot first, then Upsert/Remove).
    use crate::daemon::federation::directory::DirectoryEvent;
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let svc = make_service_with_presence(Arc::clone(&presence));

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
        Result<crate::daemon::invocation::bidi::state::presence::DispatchFrame, tonic::Status>,
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
                crate::daemon::federation::directory::SigningAuthority::SelfSigned
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
        crate::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
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

    drop(svc);
    drop(presence);

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("pump closes within 2 s after route lifecycle drops");
    let terminal = close
        .expect("route lifecycle close yields terminal chunk")
        .expect("terminal chunk is Ok");
    assert!(terminal.terminal);
    assert!(terminal.terminal_receipt.is_some());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn invoke_stream_subscribe_directory_v2_accepts_resume_sequence() {
    use crate::daemon::federation::directory::DirectoryEvent;
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let svc = make_service_with_presence(Arc::clone(&presence));

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(test_envelope()),
            function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2.to_string(),
            arguments: serde_json::to_vec(&serde_json::json!({
                "resume_sequence": 7_u64,
                "resume_token": "directory:7",
            }))
            .expect("resume args"),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("v2 resume dispatch returns Ok");

    let mut stream = resp.into_inner();
    let first = stream.next().await.expect("first frame").expect("Ok");
    assert_eq!(first.sequence, 8, "snapshot resumes after supplied cursor");
    let evt: DirectoryEvent =
        serde_json::from_slice(&first.payload).expect("decodes DirectoryEvent");
    assert!(
        matches!(evt, DirectoryEvent::Snapshot { .. }),
        "resume still starts from a convergent snapshot"
    );
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
    use crate::daemon::federation::directory::DirectoryEvent;
    use futures::StreamExt;

    let presence = Arc::new(PresenceRegistry::new());
    let svc = make_service_with_presence_and_heartbeat(
        Arc::clone(&presence),
        Some(std::num::NonZeroU64::new(50).unwrap()),
    );

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
    assert!(
        presence.snapshot().is_empty(),
        "directory heartbeat is stream keepalive only; it must not create product presence"
    );

    drop(svc);
    drop(presence);
}

#[tokio::test]
async fn invoke_stream_dispatches_registered_local_stream_ability() {
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode};
    use futures::StreamExt;

    const ABILITY: &str = "test.local_stream";
    let rt = test_runtime_with_default_trust();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ABILITY).expect("stream ability URA");
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
                .signed_envelope()
                .expect("runtime stores descriptor-bound stream envelope")
                .clone()
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
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            "invoke",
            [0x33; 32],
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();
    let svc = make_service_with_test_runtime(Arc::clone(&rt));
    publish_test_stream_route(&svc, TEST_DAEMON_URA, ABILITY);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ABILITY,
        crate::daemon::ability::CallMode::Stream,
    )
    .await;

    let function_name = ABILITY.to_string();
    let arguments = br#"{"session_ura":"easynet:///r/local/resource/daemon.browser/s1"}"#.to_vec();
    let subject_ura = "easynet:///r/test-realm/resource/browser.capture/s1";
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URA,
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
        Some(crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA)
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
async fn invoke_stream_cancels_local_runtime_when_client_drops_response() {
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode};
    use futures::StreamExt;

    const ABILITY: &str = "test.local_stream_cancel_on_drop";
    let rt = test_runtime_with_default_trust();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ABILITY).expect("stream ability URA");
    let first_frame_sent = Arc::new(std::sync::Mutex::new(None));
    let cancel_observed = Arc::new(std::sync::Mutex::new(None));
    let (first_tx, first_rx) = tokio::sync::oneshot::channel();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    *first_frame_sent.lock().expect("first signal lock") = Some(first_tx);
    *cancel_observed.lock().expect("cancel signal lock") = Some(cancel_tx);

    rt.register_ability_with_options(
        runtime_ability,
        make_ability({
            let first_frame_sent = Arc::clone(&first_frame_sent);
            let cancel_observed = Arc::clone(&cancel_observed);
            move |ctx| {
                let first_frame_sent = Arc::clone(&first_frame_sent);
                let cancel_observed = Arc::clone(&cancel_observed);
                async move {
                    ctx.emit_progress(
                        serde_json::to_vec(&serde_json::json!({
                            "MARKER-CANCEL-DROP": "progress-before-drop"
                        }))
                        .expect("progress JSON"),
                        FEDERATION_RESULT_CONTENT_TYPE,
                    )
                    .await?;
                    if let Some(tx) = first_frame_sent.lock().expect("first signal lock").take() {
                        let _ = tx.send(());
                    }
                    ctx.wait_for_cancel().await;
                    if let Some(tx) = cancel_observed.lock().expect("cancel signal lock").take() {
                        let _ = tx.send(());
                    }
                    Ok(Vec::new())
                }
            }
        }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            "invoke",
            [0x33; 32],
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();
    let svc = make_service_with_test_runtime(Arc::clone(&rt));
    publish_test_stream_route(&svc, TEST_DAEMON_URA, ABILITY);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ABILITY,
        crate::daemon::ability::CallMode::Stream,
    )
    .await;

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URA,
                    TEST_DAEMON_URA,
                )
                .expect("valid unsigned loopback stream envelope")
                .into_inner(),
            ),
            function_name: ABILITY.to_string(),
            arguments: br#"{}"#.to_vec(),
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("registered local stream returns Ok");

    let mut stream = resp.into_inner();
    tokio::time::timeout(std::time::Duration::from_secs(2), first_rx)
        .await
        .expect("stream ability emits initial frame")
        .expect("initial frame signal sent");
    let first = stream
        .next()
        .await
        .expect("one progress frame")
        .expect("progress frame is Ok");
    assert!(!first.terminal);
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);

    drop(stream);

    tokio::time::timeout(std::time::Duration::from_secs(2), cancel_rx)
        .await
        .expect("dropping InvokeStream response cancels local runtime")
        .expect("cancel signal sent");
}

#[tokio::test]
async fn invoke_stream_accepts_descriptor_ref_function_name() {
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode};
    use futures::StreamExt;

    let ability = "browser.descriptor_stream";
    let rt = test_runtime_with_default_trust();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).expect("stream ability URA");
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            "invoke",
            [0x33; 32],
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(Arc::clone(&rt));
    publish_test_stream_route(&svc, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Stream,
    )
    .await;

    let arguments = br#"{"descriptor":"stream-function-name"}"#.to_vec();
    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Stream,
    );
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URA,
                    TEST_DAEMON_URA,
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
    use crate::daemon::ability::dispatch::{
        AbilityAuthorityContext, AxonAbilityCatalog, LocalStreamHandler, OwnerKind, StreamSource,
    };
    use futures::StreamExt;

    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        test_runtime_with_default_trust(),
        AbilityAuthorityContext::for_device_authority_root(TEST_DAEMON_URA)
            .expect("test daemon URA is a valid device authority root"),
    );
    let handler: LocalStreamHandler = Arc::new(|_args| {
        Ok(StreamSource::Snapshot(vec![serde_json::json!({
            "MARKER-SNAPSHOT": "progress-before-empty-terminal"
        })]))
    });
    catalog.register_stream_with_spec(
        "browser.snapshot_once",
        OwnerKind::Device,
        test_route_manifest("browser.snapshot_once"),
        handler,
    );
    let rt = catalog.runtime().expect("catalog attaches a LocalRuntime");
    let svc = make_service_with_test_runtime(Arc::clone(&rt));
    publish_test_stream_route(&svc, TEST_DAEMON_URA, "browser.snapshot_once");

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URA,
                    TEST_DAEMON_URA,
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
    use easynet_axon::invocation::{make_ability, AbilityOptions, CallMode};
    use futures::StreamExt;

    const NON_DEFAULT_VERSION: &str = "2.0.0";
    const ABILITY: &str = "test.versioned_stream";
    let rt = test_runtime_with_default_trust();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ABILITY).expect("stream ability URA");
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|_ctx| async move {
            Ok(serde_json::to_vec(&serde_json::json!({"MARKER-V2": "dispatched"})).unwrap())
        }),
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            NON_DEFAULT_VERSION,
            "invoke",
            [0x33; 32],
            [0x11; 32],
            [0x22; 32],
        ),
    )
    .await
    .unwrap();
    let svc = make_service_with_test_runtime(Arc::clone(&rt));
    publish_test_stream_route(&svc, TEST_DAEMON_URA, ABILITY);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ABILITY,
        crate::daemon::ability::CallMode::Stream,
    )
    .await;

    let function_name = ABILITY.to_string();
    let arguments = br#"{}"#.to_vec();
    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(
                ProtoEnvelope::targeted(
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                    TEST_DAEMON_URA,
                    TEST_DAEMON_URA,
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

#[tokio::test]
async fn invoke_stream_dispatches_remote_selected_route_over_presence_session() {
    use crate::daemon::invocation::bidi::state::pending_dispatch::PendingStreamDispatchMap;
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use futures::StreamExt;

    const TARGET_DEVICE_URA: &str = "easynet:///r/test-realm/device/stream-target";
    const ABILITY: &str = "dev.semop.chat";
    const OWNER_USER_ID: &str = "stream-owner";

    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let arguments = br#"{"message":"hello"}"#.to_vec();
    let signing_key = test_device_signing_key();
    let subject_owner = format!("user.{OWNER_USER_ID}");
    let subject_ura =
        crate::core::ura::resource_dot_ura("test-realm", &subject_owner, "stream/dev-semop-chat");
    let receipt_anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: TEST_DAEMON_URA.to_string(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("remote caller trust anchor"),
    );
    let receipt_runtime = test_local_runtime(SharedTrustAnchor::new(Arc::clone(&receipt_anchor)));
    let receipt_catalog = catalog_with_json_echo_on_runtime(
        TARGET_DEVICE_URA,
        ABILITY,
        "fixture",
        "receipt",
        Arc::clone(&receipt_runtime),
    );
    let descriptor_ref = catalog_test_descriptor_ref(
        receipt_catalog.as_ref(),
        TARGET_DEVICE_URA,
        ABILITY,
        crate::daemon::ability::CallMode::Rpc,
    );
    let envelope = signed_test_envelope_with_descriptor_ref(
        TEST_DAEMON_URA,
        TARGET_DEVICE_URA,
        &subject_ura,
        descriptor_ref.clone(),
        &arguments,
        &signing_key,
    );
    let metadata = std::collections::HashMap::from([(
        crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        descriptor_ref,
    )]);
    grant_child_access_for_test(ChildAccessGrantInput {
        owner_user_id: OWNER_USER_ID,
        principal_kind: PrincipalKind::Device,
        principal_ura: TEST_DAEMON_URA,
        token_class: None,
        callee_ura: TARGET_DEVICE_URA,
        subject_ura: &subject_ura,
        ability_ura: &test_owner_ability_ura(TARGET_DEVICE_URA, ABILITY),
        action: AccessAction::Invoke,
    });

    // The simulated target must finalize the exact request that crossed the
    // session. Reusing a receipt from a different ability would defeat the
    // seven-tuple verifier and mask a production binding break.
    let receipt_admission = AdmissionFacade::new(
        Arc::clone(&receipt_anchor),
        Some(TARGET_DEVICE_URA.to_string()),
    )
    .with_ability_catalog(Arc::clone(&receipt_catalog));
    let receipt_service =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), receipt_admission)
            .with_hub_signer(test_hub_signer("test-realm"))
            .with_local_ability_catalog(Arc::clone(&receipt_catalog))
            .with_local_runtime(Arc::clone(&receipt_runtime));
    publish_test_route(&receipt_service, TARGET_DEVICE_URA, ABILITY);
    let receipt_response = receipt_service
        .invoke(Request::new(InvokeRequest {
            envelope: Some(envelope.clone()),
            function_name: ABILITY.to_string(),
            arguments: arguments.clone(),
            metadata: metadata.clone(),
            ..InvokeRequest::default()
        }))
        .await
        .expect("target runtime produces canonical receipt pair")
        .into_inner();
    let admission_receipt = receipt_response
        .admission_receipt
        .expect("target runtime produces admission receipt");
    let terminal_receipt = receipt_response
        .terminal_receipt
        .expect("target runtime produces terminal receipt");
    let expected_terminal_payload = terminal_receipt.payload.clone();
    let canonical_invocation_id = admission_receipt.invocation_id.clone();
    assert_eq!(terminal_receipt.invocation_id, canonical_invocation_id);

    let target_receipt_key = receipt_runtime
        .resolve_receipt_signer_key(TARGET_DEVICE_URA)
        .expect("resolve target receipt signer")
        .expect("target local-fast runtime minted a receipt signer");

    let pending_stream = Arc::new(PendingStreamDispatchMap::new());
    let source_anchor = RealmTrustAnchor::from_entries(vec![
        TrustedAgent {
            agent_ura: TEST_DAEMON_URA.to_string(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        },
        TrustedAgent {
            agent_ura: TARGET_DEVICE_URA.to_string(),
            public_key_b64: BASE64_STANDARD.encode(target_receipt_key.to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        },
    ])
    .expect("source receipt trust anchor");
    let source_cell = SharedTrustAnchor::new(Arc::new(source_anchor));
    let source_admission = AdmissionFacade::with_trust_anchor_cell(
        source_cell.clone(),
        Some(TEST_DAEMON_URA.to_string()),
    );
    let source_runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        Some(Arc::new(
            crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(source_cell),
        )),
        None,
    );
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), source_admission)
        .with_hub_signer(test_hub_signer("test-realm"))
        .with_local_runtime(source_runtime)
        .with_pending_stream(Arc::clone(&pending_stream));
    let (target_tx, mut target_rx) = mpsc::channel(4);
    svc.directory.presence.insert_negotiated(
        TARGET_DEVICE_URA.to_string(),
        target_tx,
        crate::daemon::invocation::bidi::state::presence::SessionContract {
            version: 1,
            claimant_boot_nonce: vec![1; 16],
        },
    );
    publish_test_route(&svc, TARGET_DEVICE_URA, ABILITY);

    let resp = svc
        .invoke_stream(Request::new(InvokeServerStreamRequest {
            envelope: Some(envelope),
            function_name: ABILITY.to_string(),
            arguments: arguments.clone(),
            metadata,
            ..InvokeServerStreamRequest::default()
        }))
        .await
        .expect("remote stream route should dispatch over presence");

    let dispatch_frame = target_rx
        .recv()
        .await
        .expect("remote target receives dispatch frame")
        .expect("dispatch frame is Ok");
    let call_id = match dispatch_frame.frame.payload.expect("dispatch payload") {
        DownPayload::DispatchCall(call) => {
            let request = call.request.expect("canonical request is present");
            let envelope = request
                .envelope
                .as_ref()
                .expect("signed envelope is present");
            assert_eq!(
                envelope
                    .callee
                    .as_ref()
                    .map(|identity| identity.ura.as_str()),
                Some(TARGET_DEVICE_URA)
            );
            assert_eq!(
                envelope
                    .subject
                    .as_ref()
                    .map(|identity| identity.ura.as_str()),
                Some(subject_ura.as_str())
            );
            assert_eq!(request.function_name, ABILITY);
            assert_eq!(request.arguments, arguments);
            assert!(request
                .metadata
                .contains_key(crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY));
            call.call_id
        }
        other => panic!("expected canonical DispatchCall carrier, got {other:?}"),
    };

    assert_eq!(
        pending_stream
            .deliver_admission(call_id, admission_receipt.clone())
            .await,
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered
    );
    assert_eq!(
        pending_stream.try_push_chunk(call_id, br#"{"delta":"part-1"}"#.to_vec()),
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered
    );
    assert_eq!(
        pending_stream.try_finish(
            call_id,
            DispatchResult {
                payload: br#"{"done":true}"#.to_vec(),
                admission_receipt: None,
                terminal_receipt: Some(terminal_receipt.clone()),
                error: None,
                failure: None,
                request_id: Some(canonical_invocation_id.clone()),
            },
        ),
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered
    );

    let mut stream = resp.into_inner();
    let admission = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("admission arrives")
        .expect("stream remains open")
        .expect("admission is Ok");
    assert_eq!(admission.sequence, 0);
    assert!(!admission.terminal);
    assert!(admission.payload.is_empty());
    assert_eq!(admission.invocation_id, canonical_invocation_id);
    assert_eq!(admission.admission_receipt, Some(admission_receipt));

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("chunk arrives")
        .expect("stream remains open")
        .expect("chunk is Ok");
    assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
    assert_eq!(first.sequence, 1);
    assert_eq!(
        first.state,
        easynet_axon::invocation::InvocationState::Running.to_wire_i32()
    );
    assert!(!first.terminal);
    assert_eq!(first.payload, br#"{"delta":"part-1"}"#);
    assert_eq!(
        first.selected_node_id,
        format!(
            "route-ref::{}",
            crate::core::ura::owner_ability_ura(TARGET_DEVICE_URA, ABILITY)
                .expect("expected ability URA")
        )
    );

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("terminal arrives")
        .expect("stream remains open until terminal")
        .expect("terminal is Ok");
    assert_eq!(
        terminal.state,
        easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
    );
    assert!(terminal.terminal);
    assert_eq!(terminal.sequence, 2);
    assert_eq!(terminal.invocation_id, canonical_invocation_id);
    assert_eq!(terminal.payload, expected_terminal_payload);
    assert!(terminal.error.is_none());
    assert_eq!(terminal.terminal_receipt, Some(terminal_receipt));

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("stream closes after terminal");
    assert!(close.is_none());
    assert_eq!(pending_stream.outstanding(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_signed_bidi_file_transfer_download_emits_business_frames() {
    use base64::Engine as _;

    let rt = test_runtime_with_default_trust();
    let mut catalog =
        crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&rt),
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                TEST_DAEMON_URA,
            )
            .expect("test daemon URA is a valid device authority root"),
        );
    crate::daemon::ability::builtins::device_control::file_transfer::register(&mut catalog);

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
        "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
            &path,
            crate::daemon::resources::files::FilesystemResourceCapability::Read,
        )
        .expect("local fs ResourceRef"),
    }))
    .unwrap();
    let file_transfer_descriptor_ref = catalog_test_descriptor_ref(
        &catalog,
        TEST_DAEMON_URA,
        crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER,
        crate::daemon::ability::CallMode::Bidi,
    );
    let open = make_descriptor_ref_envelope_open(&file_transfer_descriptor_ref, args);
    let wire = crate::daemon::axon_bridge::dispatch_shim::external_signed_from_envelope_open(&open)
        .expect("wire dispatch");
    let handle = crate::daemon::axon_bridge::dispatch_shim::open_bidi_external_signed(&rt, wire)
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
            assert_eq!(err.code(), tonic::Code::NotFound);
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
    let caller_ura = "easynet:///r/realm/agent/test.external";
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[0x55; 32]);
    let arguments = br#"{"agent_ura":"easynet:///r/realm/agent/test.external"}"#.to_vec();
    let descriptor_ref = test_descriptor_ref(TEST_DAEMON_URA, ABILITY_FEDERATION_HEARTBEAT);
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            AdmissionFacade::new(
                Arc::new(RealmTrustAnchor::default()),
                Some(TEST_DAEMON_URA.to_string()),
            ),
        )
        .with_local_runtime(test_runtime_with_default_trust()),
        TEST_DAEMON_URA,
    );
    match svc
        .invoke(Request::new(InvokeRequest {
            envelope: Some(signed_test_envelope_with_descriptor_ref(
                caller_ura,
                TEST_DAEMON_URA,
                TEST_DAEMON_URA,
                descriptor_ref.clone(),
                &arguments,
                &signing_key,
            )),
            function_name: descriptor_ref.clone(),
            arguments,
            metadata: std::collections::HashMap::from([(
                crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                    .to_string(),
                descriptor_ref,
            )]),
            ..InvokeRequest::default()
        }))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::PermissionDenied, "{}", err.message());
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
