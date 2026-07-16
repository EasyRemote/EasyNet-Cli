# Verification

Passed checks:

- `cargo test -q agent_aggregate --lib`
- `cargo test -q session_prelude --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --edition 2024 --check src/daemon/persistence/agent_aggregate.rs src/daemon/invocation/bidi/session_initiator/prelude.rs src/daemon/ability/builtins/resources/skills/list.rs`
- `git diff --check -- src/daemon/persistence/agent_aggregate.rs src/daemon/invocation/bidi/session_initiator/prelude.rs src/daemon/ability/builtins/resources/skills/list.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-hosted-advertise-prelude-agent-aggregate pr/20260716-skill-list-agent-aggregate-identity`
- `codegraph sync && codegraph status .`
