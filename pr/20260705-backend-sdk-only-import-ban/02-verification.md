Verification for backend SDK-only import-ban gate:

- `bash tools/scripts/check-backend-sdk-only-boundary.sh --self-test`
- `bash -n tools/scripts/check-backend-sdk-only-boundary.sh`
- `go test ./...` from `sdk/go`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-backend-sdk-only-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend || true`

The sibling EasyNet backend currently reports raw Axon imports, generated Axon
protobuf imports, and direct daemon transport imports. This is expected evidence
that the new gate is active and that backend source cutover remains incomplete.
