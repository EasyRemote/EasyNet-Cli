# Process singleton product-boundary convergence

## Goal

Keep the shared process-wide singleton helper product-neutral. The helper models lifecycle storage semantics (`once` versus `last_writer_wins`); it must not explain its generic contract through a product-specific OpenAI compatibility path.

## Invariants

- `ProcessSingleton<T>` remains a generic platform abstraction.
- Product adapters may choose a singleton mode, but product names and compatibility vocabulary do not belong in the shared helper's architecture contract.
- Runtime behavior and public APIs are unchanged.
- The OpenAI-compatible product adapter remains in its owning integration module.

## Boundary proof

- `src/support/platform/process_singleton.rs` sits below daemon product integrations.
- Product integrations consume this helper; they do not define the helper's abstraction.
- The correct generic distinction is lifecycle storage mode: production write-once versus test-rebindable fixture storage.

## Refactoring plan

1. Replace product-specific compatibility wording in the helper contract with product-neutral lifecycle language.
2. Add a SPEC v2 gate that rejects product-specific OpenAI compatibility vocabulary inside the shared singleton helper.
3. Verify with SPEC v2, focused unit tests, codegraph, and formatting checks.

## Verification

- `cargo test process_singleton --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- `codegraph query "ProcessSingleton product adapter compatibility boundary"`
