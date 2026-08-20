# Pluginexec sidecar schema convergence

## Goal

Converge declarative exec plugin sidecar helpers on one canonical runtime frame schema across Go, Python, Rust, Java, and Node.

## Problem

Provider helpers currently contain a dedicated `retired tuple field` rejection branch for legacy aliases (`caller`, `callee`, `ability`, `subject`). That branch preserves migration-era vocabulary inside the canonical provider model even though the exact-schema gate already rejects every non-canonical invocation field.

## Architecture decision

The provider helper owns only the canonical runtime sidecar frame:

- `caller_ura`
- `callee_ura`
- `ability_ura`
- `subject_ura`
- `invocation_nonce`
- `causal_context`
- `args`

Legacy aliases are not a separate lifecycle state. They are simply unknown fields and must be rejected by the exact canonical schema gate.

## Implementation steps

1. Remove retired-alias helper functions and calls in all sidecar helper implementations.
2. Reclassify retired-alias tests as exact-schema unknown-field tests.
3. Update the canonical runtime convergence gate so it requires exact schema rejection and bans retired tuple helper branches in production helper code.
4. Run language-level tests and convergence gates.

## Verification

- Go pluginexec tests.
- Python pluginexec tests.
- Rust pluginexec tests.
- Node pluginexec tests.
- Java pluginexec test harness where available.
- `check-canonical-runtime-convergence-v2.sh`.
- `check-sdk-product-neutrality.sh`.
- `check-architecture-convergence.sh`.
