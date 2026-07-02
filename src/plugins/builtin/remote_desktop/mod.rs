// EasyNet CLI — builtin remote desktop plugin binding
// ===================================================
//
// File: src/plugins/builtin/remote_desktop/mod.rs
// Description: Compiled binding for the `easynet.remote_desktop` plugin.

use crate::daemon::plugins::package::{BuiltinPluginAbilitySpec, BuiltinPluginBinding};
use crate::daemon::plugins::{PluginContributionBuilder, PluginRuntimeLimits, Result};

pub(crate) mod config;
pub(crate) mod constants;
pub(crate) mod errors;
pub(crate) mod event_log;
pub(crate) mod handlers;
pub(crate) mod input;
pub(crate) mod invoke_bidi;
pub(crate) mod media;
pub(crate) mod network;
pub(crate) mod permissions;
pub(crate) mod registration;
pub(crate) mod request;
pub(crate) mod resource;
pub(crate) mod runtime;
pub(crate) mod schema;
#[cfg(target_os = "macos")]
pub(crate) mod screencapturekit_capture;
pub(crate) mod sdp;
pub(crate) mod session;
pub(crate) mod session_access;
pub(crate) mod session_consent;
pub(crate) mod session_events;
pub(crate) mod session_identity;
pub(crate) mod session_lease;
pub(crate) mod session_lifecycle;
pub(crate) mod session_signaling;
pub(crate) mod session_state;
pub(crate) mod session_store;
pub(crate) mod session_transport_state;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod transport;
#[cfg(target_os = "macos")]
pub(crate) mod videotoolbox_encoder;
pub(crate) mod view;
pub(crate) mod view_device;
pub(crate) mod view_transport;

const MANIFEST_PATH: &str = "plugins/remote-desktop/plugin.toml";
const MANIFEST_BODY: &str = include_str!("../../../../plugins/remote-desktop/plugin.toml");
const ENTRYPOINT: &str = "easynet_cli::plugins::remote_desktop::contribute";
const ENABLED_ENV_VAR: &str = "EASYNET_REMOTE_DESKTOP_PLUGIN";

/// Return the compiled binding consumed by the plugin host.
pub fn binding() -> BuiltinPluginBinding {
    BuiltinPluginBinding {
        manifest_path: MANIFEST_PATH,
        manifest_body: MANIFEST_BODY,
        expected_entrypoint: ENTRYPOINT,
        enabled_env_var: Some(ENABLED_ENV_VAR),
        ability_specs,
        contribute,
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
