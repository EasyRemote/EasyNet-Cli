# Canonical Runtime Convergence Continuation

## Goal

Continue diagnosing, reviewing, and fixing canonical runtime convergence defects without narrowing the active SPEC objective.

## Scope

- Preserve the canonical runtime model defined by `docs/spec/canonical-runtime-convergence-v2.md`.
- Treat Axon as owner of protocol/runtime/SDK canonical semantics.
- Treat EasyNet-Cli daemon as product/device runtime consumer of Axon semantics.
- Do not preserve legacy product-specific protocol fields for compatibility.

## Current Invariants

1. Public SDK projections must expose canonical runtime fields only.
2. Generated Axon protobuf consumers must not depend on retired product-specific field names.
3. Product e2e must exercise the public daemon/CLI ingress, not a test-only runtime bypass.
4. Stream and bidi product operations must produce one terminal, verifiable receipt chain per operation.
5. Docker/product failures must be fixed at the owning boundary, not hidden with permissive fallback.

## Work Log

### 2026-07-30

- Verified EasyNet-Cli, EasyNet-Axon, and EasyNet worktrees were clean after the prior commits.
- Confirmed Docker is available in the current environment.
- Observed stale `easynet-media-bidi-*` Docker projects still running; this run will use isolated project names and may remove stale e2e projects if they interfere.
- Removed only stale `easynet-media-bidi-*` Docker containers, networks, and volumes; left `easynet-dev-*` untouched.
- Ran `tools/scripts/docker-media-bidi-e2e.sh --self-test`: pass.
- Ran actual Docker media/bidi e2e with project `easynet-media-bidi-fresh-20260730074234`: pass.
- Report: `target/e2e/docker-media-bidi/easynet-media-bidi-fresh-20260730074234/report.md`.

## Verified Product Facts

- Caller discovered provider stream and bidi abilities through public catalog.
- Caller resolved descriptor refs for both remote stream and bidi abilities.
- Provider and caller stream invocations preserved the invocation tuple.
- Provider and caller bidi invocations preserved the invocation tuple.
- Stream produced exactly two product operation records with one terminal receipt chain each.
- Bidi produced exactly two product operation records with one terminal receipt chain each.
- All stream and bidi receipt chains were verified.
- Plugin removal unpublished the media abilities and rejected subsequent stream/bidi route attempts.

## Verification Plan

- Run `tools/scripts/docker-media-bidi-e2e.sh --self-test`.
- Run the actual Docker media/bidi e2e with a fresh project name.
- If the e2e fails, classify the failing boundary before editing:
  - build/runtime artifact assembly,
  - Hub pairing/join,
  - descriptor publication/resolution,
  - public stream/bidi invocation,
  - receipt/chain verification,
  - plugin uninstall/route rejection.
- After edits, run SPEC v2 gate plus focused tests covering the failed boundary.
