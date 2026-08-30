# Verification

## Targeted

- `cd sdk/go && go test -tags=runtime_direct,runtime_cabi -run '^TestRuntimeAbilityClientDeadlineIsProviderOwned$' -count=1 -v ./...`
- Go SDK conformance report slice via `sdk-conformance-runner --language go --conformance-report sdk/conformance/runner/go-runtime-conformance-report.json`

## Gate Evidence

- Refresh conformance report evidence after this test file changes.
- Re-run SDK conformance reports and SDK live parity matrix with the generated live result directory.
