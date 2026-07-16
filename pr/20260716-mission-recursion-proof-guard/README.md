# Mission Recursion Proof Guard

## Goal

Close the stale mission-recursion evidence path that depended on a permanently
ignored external Claude CLI E2E. Mission recursion and child invocation must be
proved by deterministic runtime tests, not by tests that require local auth or a
developer-installed agent binary.

## Root Fork

The obsolete path treated `easynet agent send claude ...` as architecture
evidence while marking it `#[ignore]`. That creates a second proof owner:
human/manual local CLI success beside the canonical runtime child-invocation
tests.

## Decision

The canonical proof owner is `DaemonMissionInvocationGateway` and Axon
`AbilityContext::prepare_child_dispatch`. Existing runtime tests prove:

- child invocations inherit the parent receipt anchor, subject, trace metadata,
  and parent deadline;
- parent cancellation propagates to the Mission child;
- external agent dispatch starts at depth zero without depending on inherited
  environment state.

The ignored external CLI test is removed. A convergence guard now rejects
reintroducing `agent_send_desugar_e2e` as ignored evidence.

## Boundary Proof

- CLI command shape is not a protocol proof boundary.
- Mission child dispatch remains bound to Axon runtime admission and receipt
  anchoring.
- External agent process availability is not required for architecture
  convergence verification.
- No compatibility path or fallback is introduced.

## Verification Plan

- `cargo test --features axon-pb --lib mission::invocation_gateway -- --nocapture`
- `cargo test --features axon-pb --lib mission::dispatch -- --nocapture`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `cargo check --features axon-pb --lib --all-targets`
- `git diff --check`
