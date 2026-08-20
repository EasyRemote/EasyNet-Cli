# Daemon system subject policy state convergence

## Goal

Replace the implicit daemon-system subject fallback with an explicit subject
policy state so local runtime target issuance no longer hides tuple derivation
behind `unwrap_or_else`.

## Root abstraction problem

`daemon_system_subject_ura_for_descriptor` encoded two subject policies as a
parser chain:

- hub-owned abilities use the ability URA as the system subject;
- every other ability falls back to the callee URA.

That behavior is intentional for current public behavior, but the expression
was not explicit lifecycle/state. It looked like a generic fallback rather than
a named daemon-system tuple policy, making future ingress convergence harder to
audit.

## Architectural decision

Introduce a focused `DaemonSystemSubjectPolicy` enum owned by
`target.rs`. The enum names the two allowed daemon-system subject policies and
projects them into a checked subject URA.

## Boundary invariants

1. Daemon-system subject derivation must use an explicit policy enum.
2. `daemon_system_subject_ura_for_descriptor` must not use
   `unwrap_or_else(|| callee_ura.to_string())`.
3. Public ingress remains explicit and cannot use daemon-system derivation.
4. SPEC v2 must reject reintroducing the implicit subject fallback expression.

## Verification

Completed:

- `cargo fmt --check`
- `cargo test -q --features axon-pb daemon_system_subject_policy_names_callee_owner_subject --lib`
- `cargo test -q --features axon-pb daemon_system_subject_policy_names_hub_ability_subject --lib`
- `cargo test -q --features axon-pb daemon_system_subject_resolves_to_callee_for_non_hub_ability --lib`
- `cargo check --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
