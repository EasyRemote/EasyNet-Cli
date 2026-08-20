# Verification

## Commands

- `cargo test child_route_requires_selected_route_ref --features axon-pb`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`

## Result

- Focused child invocation route-ref regression passed.
- SPEC v2 self-test passed.
- SPEC v2 main gate passed.
- Rust formatting passed.
- Whitespace diff check passed.

## Architectural delta

`ChildInvocationBuilder` now fails closed when route selection does not provide
a route reference. Built child invocations can no longer carry an empty
`route_ref` while otherwise appearing descriptor-bound.

The SPEC v2 gate now pins this invariant and rejects legacy self-comparing
failure-code assertions in this route-ref traceability area.
