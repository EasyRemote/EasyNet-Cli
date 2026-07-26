# Verification

Completed:

- `cargo fmt --check`
- `cargo test policy_broker_reports_missing_permission_action_path_as_action_unavailable --lib`
- `cargo test daemon::plugins::broker --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph explore --max-files 3 -p . PluginRealtimePermissionStatus ActionUnavailable PluginPolicyBroker`
- `rg -n "PluginRealtimePermissionStatus::Unknown|pub enum PluginRealtimePermissionStatus[\\s\\S]*Unknown|permission_unknown_state_retired|ActionUnavailable|action_unavailable" src/daemon/plugins tools/scripts/check-canonical-runtime-convergence-v2.sh`

Evidence:

- The broker now maps missing permission status/request abilities to `PluginRealtimePermissionStatus::ActionUnavailable`.
- The permission status enum no longer contains `Unknown`.
- SPEC v2 rejects the retired permission `Unknown` state and includes a self-test fixture that rewrites the current source back to `Unknown`.
- Existing plugin broker tests still pass.
