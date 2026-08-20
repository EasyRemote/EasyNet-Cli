# Verification

- `cargo fmt --check`
- `cargo test cli::commands::groups::plugin --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "plugin CLI fallback JSON projection required_string_json" --path .`

Result: all verification passed. The only command-path issue was the stale Codex override path for `codegraph`; rerunning through the installed `/Users/macbook.silan.tech/.local/bin/codegraph` binary succeeded.
