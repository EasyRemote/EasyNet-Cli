# Verification

- `cargo test -q self_uninstall_cleanup_enumerates_desired_state_records`
  - Passed.
- `cargo test -q self_uninstall_cleanup_removes_orphan_state_and_status_directory`
  - Passed.
- `cargo test -q daemon::plugins::companion`
  - Passed: 32 tests.
- `cargo test -q cli::commands::groups::selfcmd`
  - Passed: 4 tests.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `rg -n "\b[U]R[I]\b|\bu[r]i\b" src/daemon/plugins/companion/mod.rs src/cli/commands/groups/selfcmd.rs pr/20260708-companion-self-uninstall-state-driven -g '!target'`
  - Passed: no matches.
