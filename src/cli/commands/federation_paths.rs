// EasyNet CLI — federation state paths
// ====================================
//
// File: src/cli/federation_paths.rs
// Description: Shared local path resolution for CLI federation wiring commands.
//
// Protocol Responsibility:
// - Keeps host-mode daemon federation state rooted under an explicit user home.
// - Rejects missing or blank HOME before resolving `~/.easynet/*` paths.
//
// Implementation Approach:
// - Centralize path preconditions used by `device join` auto-wire and
//   `federation peers` inspection so read/write surfaces cannot drift.
//
// Usage Contract:
// - Missing files remain fresh empty state for readers after a valid state root
//   has been resolved.
// - Missing/blank HOME is a precondition error, not permission to use cwd.
//
// Architectural Position:
// - CLI daemon-product state path helper. It is intentionally not in Axon SDK:
//   Axon owns protocol; EasyNet-Cli owns local daemon configuration files.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

pub(crate) fn daemon_config_path(context: &str) -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        anyhow::anyhow!("HOME is required for federation daemon-config {context} path")
    })?;
    if home.to_string_lossy().trim().is_empty() {
        anyhow::bail!("HOME is required for federation daemon-config {context} path");
    }
    Ok(PathBuf::from(home).join(".easynet/daemon-config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct HomeEnvGuard(Option<OsString>);

    impl HomeEnvGuard {
        fn capture() -> Self {
            Self(std::env::var_os("HOME"))
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn assert_home_precondition(error: anyhow::Error, context: &str) {
        let message = format!("{error:#}");
        assert!(
            message.contains(&format!(
                "HOME is required for federation daemon-config {context} path"
            )),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn daemon_config_path_rejects_missing_home_before_cwd_fallback() {
        let _lock = crate::cli::commands::test_support::env_lock();
        let _guard = HomeEnvGuard::capture();
        std::env::remove_var("HOME");

        let error =
            daemon_config_path("auto-wire").expect_err("missing HOME must not resolve under cwd");

        assert_home_precondition(error, "auto-wire");
    }

    #[test]
    fn daemon_config_path_rejects_blank_home_before_relative_state_path() {
        let _lock = crate::cli::commands::test_support::env_lock();
        let _guard = HomeEnvGuard::capture();
        std::env::set_var("HOME", " ");

        let error =
            daemon_config_path("inspection").expect_err("blank HOME must not resolve under cwd");

        assert_home_precondition(error, "inspection");
    }
}
