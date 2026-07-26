# Verification

## Completed

- `cargo test hosted_agent_delegation --lib` — passed 10 tests.
- `cargo fmt --check` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `git diff --check` — passed.
- `codegraph sync .` followed by `codegraph query "HostedAgentDelegationIngress materialize_request_metadata loopback_admitted"` — confirmed the materializer now receives `HostedAgentDelegationIngress`.
