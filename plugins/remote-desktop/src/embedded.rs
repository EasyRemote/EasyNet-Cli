// EasyNet CLI — builtin remote desktop plugin binding
// ===================================================
//
// File: plugins/remote-desktop/src/mod.rs
// Description: Package-owned provider for the `easynet.remote_desktop` plugin.

use std::sync::Arc;

use crate::daemon::plugins::package::BuiltinPluginAbilitySpec;
use crate::daemon::plugins::{
    PluginContributionBuilder, PluginProvider, PluginProviderKind, PluginRuntimeLimits, Result,
};

pub(crate) mod config;
pub(crate) mod consent_registry;
pub(crate) mod constants;
pub(crate) mod contract;
#[cfg(all(feature = "remoteapp-e2e-fault-injection", unix))]
pub(crate) mod e2e_fault_injection;
pub(crate) mod errors;
pub(crate) mod event_log;
pub(crate) mod handlers;
pub(crate) mod input;
pub(crate) mod invoke_bidi;
pub(crate) mod lease_monitor;
pub(crate) mod lifecycle_worker;
pub(crate) mod media;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(crate) mod media_host_probe;
pub(crate) mod native_host_process;
pub(crate) mod network;
pub(crate) mod permissions;
pub(crate) mod registration;
pub(crate) mod relay_lease;
pub(crate) mod request;
pub(crate) mod resource;
pub(crate) mod runtime;
pub(crate) mod schema;
pub(crate) mod sdp;
pub(crate) mod session;
pub(crate) mod session_access;
pub(crate) mod session_consent;
pub(crate) mod session_consent_state;
pub(crate) mod session_creation;
pub(crate) mod session_events;
pub(crate) mod session_identity;
pub(crate) mod session_lease;
pub(crate) mod session_lifecycle;
pub(crate) mod session_recovery;
pub(crate) mod session_signaling;
pub(crate) mod session_state;
pub(crate) mod session_store;
pub(crate) mod session_transport_state;
pub(crate) mod target;
pub(crate) mod target_focus;
pub(crate) mod target_monitor;
pub(crate) mod target_observer;
pub(crate) mod target_snapshot;
pub(crate) mod target_tracking;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod transport;
pub(crate) mod transport_blocker;
pub(crate) mod view;
pub(crate) mod view_device;
pub(crate) mod view_transport;

const MANIFEST_PATH: &str = "plugins/remote-desktop/plugin.toml";
const MANIFEST_BODY: &str = include_str!("../plugin.toml");
const ENTRYPOINT: &str = "easynet_plugin_remote_desktop::provider";
const ENABLED_ENV_VAR: &str = "EASYNET_REMOTE_DESKTOP_PLUGIN";
pub(super) const NATIVE_HOST_EXECUTABLE: &str = "easynet-remoteapp-native-host";
pub(super) const MEDIA_HOST_EXECUTABLE: &str = "easynet-remoteapp-media-host";

struct RemoteDesktopProvider {
    relay_lease_provider: Arc<dyn relay_lease::RemoteDesktopRelayLeaseProvider>,
}

/// Return the package-owned provider consumed by the generic plugin host.
pub fn provider() -> Arc<dyn PluginProvider> {
    provider_with_relay_lease_provider(Arc::new(
        relay_lease::UnavailableRemoteDesktopRelayLeaseProvider,
    ))
}

pub(in crate::daemon) fn provider_with_relay_lease_provider(
    relay_lease_provider: Arc<dyn relay_lease::RemoteDesktopRelayLeaseProvider>,
) -> Arc<dyn PluginProvider> {
    Arc::new(RemoteDesktopProvider {
        relay_lease_provider,
    })
}

impl PluginProvider for RemoteDesktopProvider {
    fn package_id(&self) -> &'static str {
        "easynet.remote_desktop"
    }

    fn provider_kind(&self) -> PluginProviderKind {
        PluginProviderKind::NativeStatic
    }

    fn manifest_body(&self) -> &'static str {
        MANIFEST_BODY
    }

    fn manifest_path(&self) -> &'static str {
        MANIFEST_PATH
    }

    fn expected_entrypoint(&self) -> &'static str {
        ENTRYPOINT
    }

    fn enabled_env_var(&self) -> Option<&'static str> {
        Some(ENABLED_ENV_VAR)
    }

    fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec> {
        ability_specs()
    }

    fn contribute(
        &self,
        builder: &mut PluginContributionBuilder,
        limits: PluginRuntimeLimits,
    ) -> Result<()> {
        contribute_with_relay_lease_provider(
            builder,
            limits,
            Arc::clone(&self.relay_lease_provider),
        )
    }
}

/// Single runtime-side source for every exported remote desktop ability.
pub fn ability_specs() -> Vec<BuiltinPluginAbilitySpec> {
    registration::ability_specs()
}

/// Contribute the plugin's ability handlers to the daemon binder.
pub fn contribute(
    builder: &mut PluginContributionBuilder,
    limits: PluginRuntimeLimits,
) -> Result<()> {
    registration::contribute(builder, limits)
}

fn contribute_with_relay_lease_provider(
    builder: &mut PluginContributionBuilder,
    limits: PluginRuntimeLimits,
    relay_lease_provider: Arc<dyn relay_lease::RemoteDesktopRelayLeaseProvider>,
) -> Result<()> {
    registration::contribute_with_relay_lease_provider(builder, limits, relay_lease_provider)
}
