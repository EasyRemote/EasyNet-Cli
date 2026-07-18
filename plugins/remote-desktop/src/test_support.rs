// EasyNet CLI — remote desktop test support
// ==========================================
//
// File: plugins/remote-desktop/src/test_support.rs
// Description: Shared fixtures for remote desktop builtin plugin tests.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::json;

use axon_sdk::invocation::{CausalContext, ReceiptRef};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::persistence::resources::{
    upsert_resource, ResourceBinding, ResourceType, ResourceUpsert, ResourcesFile,
};
use crate::daemon::plugins::remote_desktop::constants::DEFAULT_FRAME_QUEUE_DEPTH;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::stop_session_transports;
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
        "kind": "scalar",
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

pub(in crate::daemon::plugins::remote_desktop) fn seed_display(
    file: &mut ResourcesFile,
    hardware_id: &str,
) -> String {
    upsert_resource(
        file,
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/device/01DEV",
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "Test Display",
            metadata: json!({}),
        },
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn seed_xcap_display(
    file: &mut ResourcesFile,
    hardware_id: &str,
) -> String {
    upsert_resource(
        file,
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/device/01DEV",
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "xcap Display",
            metadata: json!({"backend": "xcap"}),
        },
    )
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
            owner_agent: "easynet:///r/acme/device/01DEV",
            kind: ResourceType::Window,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "Test Window",
            metadata: json!({}),
        },
    )
}
