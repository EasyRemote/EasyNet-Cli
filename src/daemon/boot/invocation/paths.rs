use std::path::PathBuf;

/// Expand a `~/...` prefix using the current user's HOME. Existing
/// EasyNet code uses several different helpers for this (some via
/// `dirs::home_dir`, some via `std::env::var("HOME")`); we mirror
/// the simplest one used by `daemon::control::transport` to keep
/// behaviour consistent across the daemon's UDS bind sites.
pub(super) fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}
