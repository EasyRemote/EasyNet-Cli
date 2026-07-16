# Verification

Checks:

- `cargo test -q descriptor_binding --lib`
- `cargo test -q selected_route --lib`
- `cargo test --features axon-pb --lib invoke_stream_dispatches_registered_local_stream_ability`
- `cargo test --features axon-pb --lib invoke_stream_accepts_descriptor_ref_function_name`
- `cargo test --features axon-pb --lib invoke_stream_dispatches_non_default_descriptor_version`
- `rustfmt --edition 2021 --check src/daemon/invocation/routing/route_resolver.rs src/daemon/invocation/dispatch/descriptor_binding.rs`
- `bash -n tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check --features axon-pb`
- `git diff --check`

The focused tests cover:

- resolver-selected RPC/stream/bidi descriptor refs from live catalog proof.
- selected-route failure when the catalog lacks descriptor proof even though
  `LocalRuntime` has options.
- selected-route failure when `LocalRuntime` proof drifts from the live
  catalog proof.
- local `InvokeStream` fixtures use the same catalog-owned Stream descriptor
  proof as production selected-route binding instead of hand-written runtime
  proof bytes.

R29 rejects the retired path where selected-route binding derives descriptor
facts from `LocalRuntime::ability_options` alone.
