# Verification

## Targeted checks

- `bash tools/scripts/go-sdk-live-smoke.sh --self-test`

## Required live checks

- `bash tools/scripts/go-sdk-live-smoke.sh`
- `bash tools/scripts/check-architecture-convergence.sh`

## Notes

The broader SDK cutover gate can still fail for stale generated evidence after Axon SDK source changes. Those failures should be separated from runtime invocation correctness: the Go smoke callee binding is a concrete runtime architecture fix.
