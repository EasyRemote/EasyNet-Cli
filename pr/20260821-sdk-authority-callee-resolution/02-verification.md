# Verification

## Targeted

- `cd sdk/go && go test -run 'Test(DelegationProof|SessionAuthority|AuthorityClient|CanonicalAuthority|AuthorizedRuntimeSession|RuntimeAbilityClientSessionAuthority|RuntimeClientResolveDescriptorRefRequiresCallMode|CABIRuntimeProviderProjectsDescriptorResolverLastError)' -count=1 ./...`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests/test_authority.py sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime_ability.py`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests/test_authority.py sdk/python/tests/test_authorized_runtime_session.py`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test ./internal/runtimecontract ./internal/svc`

## Completed gates

- `cd sdk/go && go test ./...`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python pytest -q sdk/python/tests`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test ./...`
- `python3 sdk/conformance/rebuild_public_api_model.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --write`
- `python3 sdk/conformance/refresh_conformance_report_evidence.py --check`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`

## Readiness note

The first full readiness run exposed a real downstream seam: EasyNet backend test/adapters still minted session authority fixtures with Device URAs as concrete `callee_ura` / `audience`. The SDK contract now rejects concrete Device authority targets. Backend fixtures were updated to use device-sponsored SystemAgent callees while preserving Device as execution/resource substrate. The final readiness run passed.
