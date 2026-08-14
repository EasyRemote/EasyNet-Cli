// EasyNet CLI — remote desktop test support
// ==========================================
//
// File: plugins/remote-desktop/src/test_support.rs
// Description: Shared fixtures for remote desktop builtin plugin tests.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};

use axon_sdk::invocation::{CausalContext, ReceiptRef};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::persistence::resources::{
    upsert_resource, ResourceBinding, ResourceEntry, ResourceType, ResourceUpsert, ResourcesFile,
};
use crate::daemon::plugins::remote_desktop::constants::DEFAULT_FRAME_QUEUE_DEPTH;
use crate::daemon::plugins::remote_desktop::request::{
    RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
};
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSessionInit;
use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
use crate::daemon::plugins::remote_desktop::session_lifecycle::stop_session_transports;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetResolver, ResourceEntryTargetResolver,
};
use crate::daemon::plugins::PluginRuntimeLimits;

static REMOTE_DESKTOP_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(in crate::daemon::plugins::remote_desktop) fn test_lock() -> MutexGuard<'static, ()> {
    REMOTE_DESKTOP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(in crate::daemon::plugins::remote_desktop) fn test_plugin() -> Arc<RemoteDesktopPlugin> {
    RemoteDesktopPlugin::new(
        Arc::new(SyntheticScreenBackend),
        test_runtime_limits().into(),
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn test_runtime_limits() -> PluginRuntimeLimits {
    PluginRuntimeLimits::new(128, DEFAULT_FRAME_QUEUE_DEPTH as usize)
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
    mut args: serde_json::Value,
) -> serde_json::Value {
    let issued = plugin
        .consent_registry()
        .issue(
            env.caller(),
            env.subject(),
            crate::daemon::plugins::remote_desktop::consent_registry::CONSENT_INTENT,
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
            metadata: json!({"primary_display": true, "backend": "xcap"}),
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
            metadata: json!({"backend": "xcap", "primary_display": true}),
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
