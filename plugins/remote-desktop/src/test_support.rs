// EasyNet CLI — remote desktop test support
// ==========================================
//
// File: plugins/remote-desktop/src/test_support.rs
// Description: Shared fixtures for remote desktop builtin plugin tests.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};

use axon_sdk::invocation::{CausalContext, ReceiptRef};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    ScreenSnapshotBackend, SyntheticScreenBackend,
};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext};
use crate::daemon::persistence::resources::{
    upsert_resource, ResourceBinding, ResourceEntry, ResourceType, ResourceUpsert, ResourcesFile,
};
use crate::daemon::plugins::remote_desktop::constants::DEFAULT_FRAME_QUEUE_DEPTH;
use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
use crate::daemon::plugins::remote_desktop::request::RemoteDesktopVideoConstraints;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{
    RemoteDesktopSession, RemoteDesktopSessionInit,
};
use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
use crate::daemon::plugins::remote_desktop::session_creation::RemoteAppTargetBindingVerifier;
use crate::daemon::plugins::remote_desktop::session_lifecycle::stop_session_transports;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::{
    AppSurfaceLayoutProof, AppWindowSetProof, RemoteAppTargetBinding, RemoteAppTargetError,
    RemoteDesktopTargetKind, ResolvedCaptureTargetProof, ResourceEntryTargetResolver,
    TargetGeometry,
};
use crate::daemon::plugins::{
    DaemonPluginBinder, PluginContributionBuilder, PluginContributionSet, PluginKind,
    PluginRequirementSet, PluginRuntimeLimits,
};

static REMOTE_DESKTOP_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(in crate::daemon::plugins::remote_desktop) fn test_lock() -> MutexGuard<'static, ()> {
    REMOTE_DESKTOP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(in crate::daemon::plugins::remote_desktop) fn test_plugin() -> Arc<RemoteDesktopPlugin> {
    RemoteDesktopPlugin::with_target_binding_verifier(
        Arc::new(SyntheticScreenBackend),
        Arc::new(TestRemoteAppTargetBindingVerifier),
        test_runtime_limits().into(),
    )
}

/// Assemble the executable RemoteApp catalog used by unit-level invocation
/// tests. Production registration remains bound to platform capture services;
/// callers of this seam must inject their deterministic capture backend.
pub(crate) fn test_catalog_with_screen_backend(
    backend: Arc<dyn ScreenSnapshotBackend>,
) -> AxonAbilityCatalog {
    let limits = test_runtime_limits();
    let mut builder = PluginContributionBuilder::new(
        "easynet.remote_desktop",
        "0.1.0",
        PluginKind::Builtin,
        limits,
        PluginRequirementSet::default(),
        Vec::new(),
    );
    crate::daemon::plugins::remote_desktop::registration::contribute_with_platform_services(
        &mut builder,
        backend,
        Arc::new(TestRemoteAppTargetBindingVerifier),
        limits,
    )
    .expect("remote desktop test contribution");
    let contribution = builder
        .finish()
        .expect("remote desktop test contribution finish");
    let contributions = PluginContributionSet::new(vec![contribution]);
    let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
        None,
    );
    let mut catalog = AxonAbilityCatalog::new_test_runtime_for_device_authority(
        runtime,
        "easynet:///r/acme/device/01DEV",
    );
    DaemonPluginBinder::static_catalog(&mut catalog)
        .bind_set(&contributions)
        .expect("bind remote desktop test contribution");
    catalog
}

pub(in crate::daemon::plugins::remote_desktop) fn test_runtime_limits() -> PluginRuntimeLimits {
    PluginRuntimeLimits::new(128, DEFAULT_FRAME_QUEUE_DEPTH as usize)
}

pub(in crate::daemon::plugins::remote_desktop) fn admit_decoded_video_for_test(
    session: &mut RemoteDesktopSession,
    epoch: TransportEpoch,
    media_pipeline_id: &str,
) {
    let session_id = session.session_id().to_string();
    let binding = session.target_binding();
    let subject_ura = binding.subject_ura().to_string();
    let binding_id = binding.binding_id().to_string();
    let binding_epoch = binding.binding_epoch();
    let media_source_epoch = binding.media_source_epoch();
    session.record_media_stats(
        epoch,
        json!({
            "media_pipeline_id": media_pipeline_id,
            "backend_id": media_pipeline_id,
            "video_codec": "h264",
            "video_transport": "webrtc",
        }),
    );
    assert!(session.merge_client_media_stats(
        epoch,
        json!({
            "render_probe": {
                "evidence_authority": "client_reported",
                "probe_source": "browser_webrtc_receiver",
                "selected_resource_ura": subject_ura,
                "session_id": session_id,
                "transport_epoch": epoch.value(),
                "binding_id": binding_id,
                "binding_epoch": binding_epoch,
                "media_source_epoch": media_source_epoch,
                "media_pipeline_id": media_pipeline_id,
                "video_codec": "h264",
                "video_transport": "webrtc",
                "decoded_video_frames": 1,
                "frame_width": 1280,
                "frame_height": 720,
            }
        }),
    ));
}

pub(in crate::daemon::plugins::remote_desktop) fn reset_store(plugin: &RemoteDesktopPlugin) {
    plugin.session_store().with_sessions(|sessions| {
        for (session_id, session) in sessions.iter_mut() {
            stop_session_transports(plugin, session_id, session);
        }
        sessions.clear();
    });
    plugin.transport_manager().clear_endpoints();
}

pub(in crate::daemon::plugins::remote_desktop) fn env_for(subject: &str) -> EnvelopeContext {
    env_for_caller(subject, "easynet:///r/acme/user/test-caller")
}

pub(in crate::daemon::plugins::remote_desktop) fn with_consent_ticket(
    plugin: &RemoteDesktopPlugin,
    env: &EnvelopeContext,
    args: serde_json::Value,
) -> serde_json::Value {
    with_consent_ticket_grants(plugin, env, args, false)
}

pub(in crate::daemon::plugins::remote_desktop) fn with_input_control_consent_ticket(
    plugin: &RemoteDesktopPlugin,
    env: &EnvelopeContext,
    args: serde_json::Value,
) -> serde_json::Value {
    with_consent_ticket_grants(plugin, env, args, true)
}

fn with_consent_ticket_grants(
    plugin: &RemoteDesktopPlugin,
    env: &EnvelopeContext,
    args: serde_json::Value,
    input_control_granted: bool,
) -> serde_json::Value {
    with_consent_ticket_scope(plugin, env, args, input_control_granted, false)
}

fn with_consent_ticket_scope(
    plugin: &RemoteDesktopPlugin,
    env: &EnvelopeContext,
    mut args: serde_json::Value,
    input_control_granted: bool,
    remote_focus_granted: bool,
) -> serde_json::Value {
    let issued = plugin
        .consent_registry()
        .issue_with_grants(
            env.caller(),
            env.subject(),
            crate::daemon::plugins::remote_desktop::consent_registry::CONSENT_INTENT,
            input_control_granted,
            remote_focus_granted,
        )
        .expect("test consent ticket issues");
    args.as_object_mut()
        .expect("test create arguments must be an object")
        .insert(
            "consent_ticket".to_string(),
            serde_json::json!(issued.ticket),
        );
    args
}

pub(in crate::daemon::plugins::remote_desktop) fn create_test_session(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let args = with_consent_ticket(&plugin, &env, args);
    crate::daemon::plugins::remote_desktop::handlers::create_session::handle(plugin, env, args)
}

pub(in crate::daemon::plugins::remote_desktop) struct TestRemoteAppTargetBindingVerifier;

impl RemoteAppTargetBindingVerifier for TestRemoteAppTargetBindingVerifier {
    fn verify_for_session(
        &self,
        _ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
        let locator = binding.native_locator();
        Ok(
            ResolvedCaptureTargetProof::new(locator.capture_backend(), binding.target_kind())
                .with_native_identity(
                    locator.display_id(),
                    locator.window_id(),
                    locator.pid(),
                    locator.app_identity().map(ToOwned::to_owned),
                    locator.bundle_id().map(ToOwned::to_owned),
                )
                .with_process_instance_id(locator.process_instance_id().map(ToOwned::to_owned))
                .with_native_dimensions(Some((1280, 720))),
        )
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn env_for_caller(
    subject: &str,
    caller: &str,
) -> EnvelopeContext {
    EnvelopeContext::for_test(caller, subject).with_causal_context(default_consent_receipt())
}

pub(in crate::daemon::plugins::remote_desktop) fn env_for_caller_with_causal(
    subject: &str,
    caller: &str,
    causal_context: serde_json::Value,
) -> EnvelopeContext {
    env_for_caller(subject, caller).with_causal_context(causal_context)
}

// Borrowed receipt-URA shape (`resource/<owner>.invocations/<id>`, the
// Axon ledger.rs test convention) — no production receipt-body builder
// exists yet; canonicalization tracked by RFC-007/008 (F-042).
fn default_consent_receipt() -> serde_json::Value {
    json!({
        "form": "scalar",
        "receipt_ura": "easynet:///r/acme/resource/alice.invocations/test-local-consent",
        "receipt_hash": "4242424242424242424242424242424242424242424242424242424242424242",
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn test_consent_causal_context() -> CausalContext {
    CausalContext::Scalar(ReceiptRef {
        receipt_ura: "easynet:///r/acme/resource/alice.invocations/test-local-consent".to_string(),
        receipt_hash: [0x42; 32],
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn live_remote_target_metadata(
    mut metadata: Value,
) -> Value {
    let map = metadata
        .as_object_mut()
        .expect("remote desktop live target metadata fixture must be an object");
    map.insert("availability".to_string(), json!("available"));
    map.insert(
        "freshness".to_string(),
        json!({
            "observed_at_ms": 1,
            "stale_after_ms": u64::MAX,
            "source": "live_refresh",
        }),
    );
    if !map.contains_key("front_to_back_surfaces") {
        let window_ids = map
            .get("resolved_window_ids")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
            .unwrap_or_default();
        if !window_ids.is_empty() {
            let geometries = window_ids
                .iter()
                .enumerate()
                .map(|(index, window_id)| {
                    (
                        *window_id,
                        TargetGeometry {
                            x: Some(index as f64 * 120.0),
                            y: Some(0.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                    )
                })
                .collect::<Vec<_>>();
            let layout = AppSurfaceLayoutProof::from_front_to_back_geometries(
                geometries
                    .iter()
                    .map(|(window_id, geometry)| (*window_id, geometry)),
            )
            .expect("test application surface layout");
            map.insert(
                "front_to_back_surfaces".to_string(),
                layout.to_value()["front_to_back_surfaces"].clone(),
            );
            map.insert(
                "surface_layout_epoch".to_string(),
                json!(layout.layout_epoch()),
            );
        }
    }
    metadata
}

pub(in crate::daemon::plugins::remote_desktop) fn test_session_init(
    session_id: &str,
    subject: &str,
    transport_preferences: Vec<String>,
) -> RemoteDesktopSessionInit {
    let env = env_for(subject);
    let entry = ResourceEntry {
        resource_ura: subject.to_string(),
        owner_agent: "easynet:///r/acme/agent/device.01DEV.remote-desktop".to_string(),
        kind: ResourceType::Display,
        binding: ResourceBinding::LocalDevice,
        hardware_id: "test-display".to_string(),
        display_name: "Test Display".to_string(),
        metadata: json!({"primary_display": true, "backend": "xcap"}),
        first_seen_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let target_binding = ResourceEntryTargetResolver
        .resolve_for_session("test.ability", &entry, "view_only", 1)
        .expect("test target binding resolves");
    RemoteDesktopSessionInit {
        session_id: session_id.to_string(),
        session_token: "token".to_string(),
        creator_caller_ura: env.caller().to_string(),
        consent: RemoteDesktopConsentGrant::from_envelope_for_test(&env),
        target_binding,
        mode: "view_only".to_string(),
        lease_ttl_ms: 5_000,
        transport_preferences,
        video: RemoteDesktopVideoConstraints::default(),
        input_policy: RemoteDesktopInputPolicy::default(),
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn test_application_target_binding(
) -> RemoteAppTargetBinding {
    let app_window_set = AppWindowSetProof::new(
        42,
        Some("com.example.Editor".to_string()),
        Some(9001),
        vec![10, 11],
    );
    let mut binding = ResourceEntryTargetResolver
        .resolve_for_session(
            "test.ability",
            &ResourceEntry {
                resource_ura: "easynet:///r/acme/resource/application.test".into(),
                owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
                kind: ResourceType::Application,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "application:macos:pid:9001".into(),
                display_name: "Editor".into(),
                metadata: live_remote_target_metadata(json!({
                    "display_id": 42,
                    "bundle_id": "com.example.Editor",
                    "app_identity": "com.example.Editor",
                    "primary_pid": 9001,
                    "resolved_window_ids": [10, 11],
                    "window_set_epoch": app_window_set.window_set_epoch(),
                    "target_identity_epoch": app_window_set.window_set_epoch(),
                    "x": 10,
                    "y": 20,
                    "width": 200,
                    "height": 100,
                })),
                first_seen_at: "2026-06-01T00:00:00Z".into(),
            },
            "view_only",
            1,
        )
        .expect("application target binding resolves");
    let proof = ResolvedCaptureTargetProof::new(
        binding.native_locator().capture_backend(),
        RemoteDesktopTargetKind::Application,
    )
    .with_native_identity(
        Some(42),
        None,
        Some(9001),
        Some("com.example.Editor".to_string()),
        Some("com.example.Editor".to_string()),
    )
    .with_native_dimensions(Some((200, 100)))
    .with_app_window_set(app_window_set)
    .with_app_surface_layout(
        AppSurfaceLayoutProof::from_front_to_back_geometries([
            (
                10,
                &TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(100.0),
                    height: Some(100.0),
                },
            ),
            (
                11,
                &TargetGeometry {
                    x: Some(110.0),
                    y: Some(20.0),
                    width: Some(100.0),
                    height: Some(100.0),
                },
            ),
        ])
        .expect("test application surface layout"),
    );
    binding
        .commit_capture_proof("test.ability", proof)
        .expect("application proof commits");
    binding
}

pub(in crate::daemon::plugins::remote_desktop) fn test_window_target_binding(
) -> RemoteAppTargetBinding {
    let mut binding = ResourceEntryTargetResolver
        .resolve_for_session(
            "test.ability",
            &ResourceEntry {
                resource_ura: "easynet:///r/acme/resource/window.test".into(),
                owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
                kind: ResourceType::Window,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "window:macos:cgwindow:10:42".into(),
                display_name: "Editor".into(),
                metadata: live_remote_target_metadata(json!({
                    "window_id": 42,
                    "pid": 10,
                    "app_name": "Editor",
                    "x": 100,
                    "y": 200,
                    "width": 800,
                    "height": 600,
                    "target_identity_epoch": 7,
                    "geometry_revision": 3,
                })),
                first_seen_at: "2026-06-01T00:00:00Z".into(),
            },
            "view_only",
            1,
        )
        .expect("window target binding resolves");
    let locator = binding.native_locator();
    let proof =
        ResolvedCaptureTargetProof::new(locator.capture_backend(), RemoteDesktopTargetKind::Window)
            .with_native_identity(
                locator.display_id(),
                locator.window_id(),
                locator.pid(),
                locator.app_identity().map(ToOwned::to_owned),
                locator.bundle_id().map(ToOwned::to_owned),
            )
            .with_process_instance_id(locator.process_instance_id().map(ToOwned::to_owned))
            .with_native_dimensions(Some((800, 600)));
    binding
        .commit_capture_proof("test.ability", proof)
        .expect("window proof commits");
    binding
}

pub(in crate::daemon::plugins::remote_desktop) fn test_window_session_init(
    session_id: &str,
    transport_preferences: Vec<String>,
) -> RemoteDesktopSessionInit {
    let subject = "easynet:///r/acme/resource/window.test";
    let env = env_for(subject);
    RemoteDesktopSessionInit {
        session_id: session_id.to_string(),
        session_token: "token".to_string(),
        creator_caller_ura: env.caller().to_string(),
        consent: RemoteDesktopConsentGrant::from_envelope_for_test(&env),
        target_binding: test_window_target_binding(),
        mode: "view_only".to_string(),
        lease_ttl_ms: 5_000,
        transport_preferences,
        video: RemoteDesktopVideoConstraints::default(),
        input_policy: RemoteDesktopInputPolicy::default(),
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn test_application_session_init(
    session_id: &str,
    transport_preferences: Vec<String>,
) -> RemoteDesktopSessionInit {
    let subject = "easynet:///r/acme/resource/application.test";
    let env = env_for(subject);
    RemoteDesktopSessionInit {
        session_id: session_id.to_string(),
        session_token: "token".to_string(),
        creator_caller_ura: env.caller().to_string(),
        consent: RemoteDesktopConsentGrant::from_envelope_for_test(&env),
        target_binding: test_application_target_binding(),
        mode: "view_only".to_string(),
        lease_ttl_ms: 5_000,
        transport_preferences,
        video: RemoteDesktopVideoConstraints::default(),
        input_policy: RemoteDesktopInputPolicy::default(),
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn seed_display(
    file: &mut ResourcesFile,
    hardware_id: &str,
) -> String {
    upsert_resource(
        file,
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "Test Display",
            metadata: json!({"display_id": 1, "primary_display": true, "backend": "xcap"}),
        },
    )
    .expect("seed remote desktop display")
}

pub(in crate::daemon::plugins::remote_desktop) fn seed_xcap_display(
    file: &mut ResourcesFile,
    hardware_id: &str,
) -> String {
    upsert_resource(
        file,
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "xcap Display",
            metadata: json!({"backend": "xcap", "display_id": 1, "primary_display": true}),
        },
    )
    .expect("seed remote desktop xcap display")
}

#[cfg(not(target_os = "macos"))]
pub(in crate::daemon::plugins::remote_desktop) fn seed_window(
    file: &mut ResourcesFile,
    hardware_id: &str,
) -> String {
    upsert_resource(
        file,
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "Test Window",
            metadata: live_remote_target_metadata(json!({
                "window_id": 42,
                "pid": 10,
                "app_name": "Test Window",
                "x": 0,
                "y": 0,
                "width": 800,
                "height": 600,
            })),
        },
    )
    .expect("seed remote desktop window")
}
