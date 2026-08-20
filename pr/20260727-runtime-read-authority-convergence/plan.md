# Runtime Read Authority Convergence Plan

Date: 2026-07-27

## Goal

Converge product-visible runtime catalogue/history reads onto one canonical authority path so device pages and CLI calls no longer produce descriptor lookup, owner-offline, caller-signer, or authority-subject errors after a clean daemon start.

## User-Visible Failures

- `invocation.history.list` failed with `AUTHORITY_SUBJECT_MISMATCH`.
- `runtime_resolve_descriptor_ref` failed with missing caller signer for the active user URA.
- `meta.list_abilities` / `meta.list_resources` failed with descriptor lookup routed through remote namespace resolution and negative owner-online state.
- Product UI concluded no `browser.open_session` route was visible even though the daemon was running.

## Root Abstraction Problem

Catalogue/history reads are governance/runtime-state reads, not generic product action invocations. If callers submit them through the public remote descriptor-bound invoke path, the tuple policy is selected by the wrong layer:

- public action ingress treats the callee device as invocation subject;
- history/session authority expects a runtime-state read subject owned by the caller/user;
- descriptor resolution may route remote before proving local runtime catalogue ownership;
- caller signer readiness may depend on stale product keyring state instead of daemon Ready proof.

The fix must remove the alternate authority path rather than add fallback probes.

## Boundary Invariants

1. `meta.list_abilities` and `meta.list_resources` are LocalRuntime catalogue reads for the runtime owner, not hidden remote probes.
2. `invocation.history.list` is a canonical history read path with explicit caller, subject, authority metadata, nonce, and causal context.
3. Product/UI/CLI reads must not default a placeholder all-zero user subject.
4. Descriptor resolution must fail locally with a stable descriptor-catalog error before remote namespace resolution is attempted for local catalogue routes.
5. Daemon Ready in device/both mode must prove paired User caller signer custody before publishing runtime projection.
6. Clearing local state is allowed for verification, but production behavior must not depend on legacy-state compatibility.

## Implementation Direction

1. Inspect current canonical read issuers and public invoke gates.
2. Reproduce or create deterministic tests for the observed failures.
3. Refactor the selected ingress to one authority/read issuer.
4. Remove obsolete fallback/compat wording or paths encountered in-scope.
5. Verify with focused tests, architecture gates, rustfmt, and SPEC v2 gate where feasible.

## Acceptance Checks

- Product-visible read paths reject placeholder/all-zero subjects before daemon IO.
- Catalogue reads resolve from local runtime catalogue for the daemon runtime owner.
- History reads use the canonical history read model and cannot pass through generic public remote invoke.
- Error messages do not leak keyring internals and do not route local catalogue reads to `owner is not online`.
- Existing public APIs remain compatible while internal ownership converges.

## Iteration Notes

### 2026-07-27

- Used codegraph and direct source search to trace the reported failures to catalogue/history read ingress and descriptor-provider boundaries.
- Confirmed production `check-runtime-state-read-subject-boundary.sh` already requires catalogue reads to use `LocalRuntimeCatalogueReadIssuer`.
- Found the script self-test fixture was stale: its happy path still represented catalogue targets with `LocalRuntimeStateReadIssuer` and still used the retired fixture-local `LocalRuntimeStateReadSubject` grammar.
- Updated the fixture to mirror production architecture:
  - `LocalRuntimeStateReadAttachment` owns only runtime attachment facts.
  - `core::identity::RuntimeStateReadSubject::new` owns subject grammar.
  - catalogue read fixtures use `LocalRuntimeCatalogueReadIssuer`.
- Clean-state smoke:
  - `easynet device reset --purge-local-state --force -y` removed the local state root.
  - `easynet start` failed closed without credentials instead of creating an all-zero user/device identity.
  - `easynet runtime start --as-hub` failed closed without TLS config.

## Verification Log

- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo test -q --features axon-pb runtime_state_read_subject_boundary_script_holds`
- `cargo test -q --features axon-pb runtime_state_read_subject`
- `cargo test -q --features axon-pb runtime_descriptor_provider`
- `npm test --prefix sdk/node -- --test-name-pattern "runtimeStateReadSubject|session history|public invocation builder rejects"`
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python -m pytest -q sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_ability.py sdk/python/tests/test_errors.py -k 'history or runtime_state or descriptor or governance or caller_signer or owner_offline'`
- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`

### 2026-07-27 Hub authority governance-read subject cutover

- Cleared the local EasyNet state root with `easynet device reset --purge-local-state --force --yes` as allowed by the user.
- Rebuilt a clean Hub runtime from current source with generated local TLS material.
- Reproduced a clean-state defect: `easynet invocation list --format json` failed in Hub mode because the CLI history issuer required paired device/user credentials even when the daemon owner was the realm Authority.
- Added `RuntimeGovernanceReadSubject` as the shared core value object for governance reads. It admits only:
  - canonical user-owned `runtime-state/read` subjects; or
  - the callee realm Authority subject for authority-owned Hub reads.
- Migrated selected-route admission and descriptor-ref receipt-history validation to that shared value object.
- Added `LocalRuntimeGovernanceReadIssuer` so CLI invocation history reads select Hub authority subject from daemon Ready discovery instead of defaulting a missing/all-zero user.
- Switched `easynet invocation ...` from `LocalRuntimeStateReadIssuer` to `LocalRuntimeGovernanceReadIssuer`.
- Verified clean Hub command behavior: `easynet invocation list --limit 5 --format json` now succeeds and returns authority-owned signed receipt chains.
