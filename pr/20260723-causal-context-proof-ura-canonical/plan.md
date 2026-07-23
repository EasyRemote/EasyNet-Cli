# Causal context proof URA canonicalization

## Goal

Remove the Go SDK causal-context `dag_root_hex` / `dag_proof_ura` compatibility aliases and require the canonical `root_hex` / `proof_ura` fields.

The SDK is the canonical runtime model. A causal DAG proof has one public shape: `root_hex` plus `proof_ura`. Allowing Go to accept `dag_root_hex` or `dag_proof_ura` while Python rejects it creates language-specific architecture drift and lets old JSON shapes survive through the provider-backed runtime path.

## Boundary proof

- This slice only changes SDK causal-context parsing and conformance gates.
- The canonical `root_hex` / `proof_ura` shape remains supported in Go and Python.
- The public Go API remains source compatible; only retired input JSON alias acceptance is removed.
- Python already requires `root_hex` / `proof_ura`; this slice pins that behavior with a regression test.
- SPEC v2 will reject future reintroduction of `dag_root_hex` / `dag_proof_ura`.

## Invariants

1. Go causal DAG contexts require `root_hex` and `proof_ura`.
2. Go causal DAG contexts reject `dag_root_hex` / `dag_proof_ura` when canonical fields are missing.
3. Python causal DAG contexts reject `dag_root_hex` / `dag_proof_ura` when canonical fields are missing.
4. No active SDK runtime signing parser contains `dag_root_hex` / `dag_proof_ura`.
5. SPEC v2 mutation coverage catches the retired alias.

## Verification plan

- Go runtime signing/causal-context tests.
- Python runtime signing/causal-context tests.
- SPEC v2 gate and self-test.
- SDK product-neutrality and public API gates.
- codegraph sync/status.

## Delta log

- Removed Go SDK fallback parsing for `dag_root_hex` and `dag_proof_ura` from `causalContextForInvocationDraft`.
- Added Go regression coverage proving canonical `root_hex` / `proof_ura` succeeds and retired aliases fail closed.
- Added Python regression coverage proving the direct-runtime causal-context parser continues to reject retired aliases.
- Added SPEC v2 structural and mutation coverage so retired DAG proof aliases cannot re-enter active SDK source.

## Verification results

- `cd sdk/go && go test . -run 'TestRuntimeSigningCausalContextRejectsRetiredDAGProofAliases|TestRuntimeSigningTransportSignsUnsignedInvokeDraft'`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py -q -k 'causal_context_rejects_retired_dag_proof_alias or preserves_complete_caller_supplied_tuple'`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
