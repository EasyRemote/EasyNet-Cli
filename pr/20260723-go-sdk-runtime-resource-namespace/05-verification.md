# Verification

## Planned checks

- `gofmt -w sdk/go/resource_namespace.go sdk/go/ura.go`
- `go test ./...`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `codegraph query runtimeResourceURA --limit 40`
- `codegraph query productResourceURA --limit 40`
- `rg -n "productResource|EasyNet's provider namespace" sdk/go/resource_namespace.go sdk/go/ura.go`

## Results

- `gofmt -w sdk/go/resource_namespace.go sdk/go/ura.go sdk/go/ura_test.go`
  — applied formatting.
- `go test ./...` from `sdk/go` — passed.
- Initial root-gate attempt from `sdk/go` did not count because relative
  repository paths were invalid from that working directory.
- `cargo fmt --check` from repository root — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` from
  repository root — passed.
- `bash tools/scripts/check-architecture-convergence.sh` from repository root
  — passed.
- `git diff --check` from repository root — passed.
- `codegraph index .` — passed; indexed 1,018 files.
- `codegraph query runtimeResourceURA --limit 40` — found
  `runtimeResourceURA` and public `RuntimeResourceURA`.
- `codegraph query productResourceURA --limit 40` — no results.
- `rg -n "productResource|EasyNet's provider namespace|product namespace"
  sdk/go/resource_namespace.go sdk/go/ura.go` — no matches.
