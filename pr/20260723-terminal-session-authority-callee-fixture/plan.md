# Terminal Session Authority Callee Fixture Convergence

## Goal

Remove the legacy terminal follow-up fixture binding that minted session authority metadata for
`easynet:///r/default/device/local` while the real-invoke runtime executes against the canonical
combined authority fixture device `easynet:///r/localhost/device/dev-1`.

## Root Abstraction Problem

Terminal follow-up tests were not modeling the same runtime tuple as production dispatch:

- envelope callee: selected by the real-invoke runtime authority context;
- session authority callee: hard-coded by the test helper to a legacy default/local device.

The production validator correctly rejects this as a callee mismatch. Weakening that validator would
reintroduce a second authority path. The fixture must instead bind authority metadata to the same
canonical callee as the invocation envelope.

## Invariants

1. A verified terminal session authority is valid only when `authority.callee_ura == env.callee()`.
2. A verified terminal session authority is valid only when `authority.issuer_ura == env.caller()`.
3. Follow-up terminal handlers must continue to require an explicit session authority before PTY
   table access.
4. Tests must not mint terminal session authority for `default/local` unless that is also the
   envelope callee under test.
5. No production compatibility fallback may be added for mismatched authority/envelope tuples.

## Boundary Proof

- Core runtime/admission remains the source of truth for envelope caller/callee/subject.
- Terminal handlers keep a single authority validator in
  `device_control::terminal::authority::require_session_authority`.
- Real-invoke fixture helpers must construct metadata from the same canonical authority root used by
  `runtime_attached_catalog()`.

## Planned Change

Replace the hard-coded terminal follow-up callee and subject realm in
`src/daemon/ability/builtins/real_invoke_tests.rs` with the canonical real-invoke authority fixture
callee.

## Verification

- Focused terminal real-invoke tests for close/input/read/resize/attach.
- Serial `real_invoke_tests` aggregate.
- `cargo fmt --check`.
- `git diff --check`.
- codegraph status after edits.

## Results

- `cargo test -q real_device_terminal --features axon-pb -- --test-threads=1`
  - Result: 6 passed, 0 failed.
- `cargo test -q real_invoke_tests --features axon-pb -- --test-threads=1`
  - Result: 134 passed, 0 failed.
- `cargo fmt --check`
  - Result: passed after rustfmt normalized the helper.
- `git diff --check`
  - Result: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced 1 changed file.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - Result: index up to date.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.

## Decision Record

The fix is deliberately constrained to the real-invoke fixture. The production terminal authority
validator remains strict because accepting mismatched callee metadata would create a second authority
path and weaken descriptor-bound admission.
