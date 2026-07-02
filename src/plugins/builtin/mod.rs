// EasyNet CLI — compiled builtin plugin bindings
// ==============================================
//
// File: src/plugins/builtin/mod.rs
// Description: Compile-time bindings for builtin plugin packages.

use crate::daemon::plugins::package::BuiltinPluginBinding;

#[cfg(feature = "remote-desktop")]
pub mod remote_desktop;

/// Return every builtin plugin binding compiled into this binary.
///
/// What this is NOT: runtime loading policy. The plugin host owns env,
/// platform, dependency, and permission checks before a binding is registered.
pub fn builtin_bindings() -> Vec<BuiltinPluginBinding> {
    vec![
        #[cfg(feature = "remote-desktop")]
        remote_desktop::binding(),
    ]
}
