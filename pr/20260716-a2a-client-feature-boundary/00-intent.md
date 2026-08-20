# A2A Client Feature Boundary

## Root Fork

`tools/scripts/check-voice-call-product-contract.sh` builds the voice contract
verifier with `--no-default-features`. That build exposed unused A2A client
imports and helper code that are only meaningful when `axon-pb` is enabled.

The runtime behavior is already correctly split: production A2A forwarding
requires `axon-pb`, while the no-default build returns a structured
unsupported response. The source boundary was weaker than the behavior boundary
because `axon-pb`-only causal/federation helpers were still compiled into the
no-default build.

## CodeGraph Evidence

- `register(...)` always wires `a2a.client.send_task` as an envelope-aware
  ability so the handler can read causal context.
- `send_task_handler(...)` uses `causal_parents_from_env(...)`,
  `DiscoverFederationResolveError`, and remote invocation only inside the
  `#[cfg(feature = "axon-pb")]` block.
- The `#[cfg(not(feature = "axon-pb"))]` block returns a caller-visible
  unsupported response and does not use the envelope context.

## Invariants

- Production `axon-pb` behavior must keep preserving inbound causal parents
  across the A2A forward hop.
- No-default builds must compile without carrying unreachable feature-specific
  helper code.
- The no-default unsupported response remains public behavior.

## Verification Plan

- Gate `DiscoverFederationResolveError` and causal-parent extraction to
  `axon-pb` production builds, while preserving test access for the helper.
- Mark the no-default envelope context as intentionally unused in the
  unsupported branch.
- Re-run `bash tools/scripts/check-voice-call-product-contract.sh`.
- Re-run focused A2A causal tests with `--features axon-pb`.
- Re-run `bash tools/scripts/check-architecture-convergence.sh`.

## Verification Results

- `bash tools/scripts/check-voice-call-product-contract.sh` ->
  `verify-voice-contract: ok`
- `cargo test -p easynet causal_parents_extracted_from_each_causal_context_shape --features axon-pb -- --nocapture`
  passed.
- `rustfmt --check --edition 2021 src/daemon/ability/builtins/integrations/a2a/client.rs`
  passed.
- `git diff --check -- src/daemon/ability/builtins/integrations/a2a/client.rs pr/20260716-a2a-client-feature-boundary/00-intent.md`
  passed.
- `bash tools/scripts/check-architecture-convergence.sh` ->
  `architecture-convergence: OK`
