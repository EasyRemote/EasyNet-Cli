# Verification

- `codegraph sync .` — refreshed analysis after changing plugin output read models.
- `codegraph query "PluginRealtimeActivationPlan" --path .` — confirms construction/consumer references remain in realtime/broker/host paths.
- `codegraph query "PluginSurfaceReport" --path .` — confirms internal report ownership stays in daemon construction/composition paths.
- `codegraph query "invoke_plugin_status" --path .` — confirms CLI plugin status now returns `Option<Value>`.
- `cargo test daemon::plugins::realtime --lib` — 5 passed.
- `cargo test daemon::plugins::surface --lib` — 6 passed.
- `cargo test daemon::plugins::broker --lib` — 3 passed.
- `cargo test cli::commands::groups::plugin --lib` — 15 passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — OK.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — OK.
- `bash tools/scripts/check-architecture-convergence.sh` — OK.
- `git diff --check` — OK.
- `cargo fmt --check` — OK.

Observed but not used as pass evidence:

- `cargo test daemon::plugins --lib` compiled successfully, then failed 3 existing `remote_desktop::handlers::attach` tests because the local machine has no joined device credentials. This is outside the output-read-model boundary.
