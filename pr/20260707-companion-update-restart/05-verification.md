# Verification

Passed:

- `cargo test -q running_package_update_restarts_with_stop_then_start`
- `cargo test -q stopped_package_update_preserves_desired_without_restart`
- `cargo test -q daemon::plugins::companion`
- `cargo test -q daemon::plugins::install`
- `cargo test -q plugin_host_install`
- `cargo fmt --check`
- `git diff --check`
- `rg -n "\b[U]R[I]\b|\bu[r]i\b" src/daemon/plugins/companion/mod.rs pr/20260707-companion-update-restart -g '!target'`

Result:

- A running companion package update records the existing `restart` action and executes supervisor operations in deterministic `install`, `enable`, `stop`, `start` order.
- A stopped companion package update preserves desired enabled state without invoking `stop` or `start`.
- No new forbidden address terminology was introduced in this slice.
