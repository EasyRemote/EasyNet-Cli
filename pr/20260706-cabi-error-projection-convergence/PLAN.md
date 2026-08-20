# C ABI Error Projection Convergence Plan

## Objective

Converge Go SDK C ABI profile transports onto one private typed-error
projection helper. The SDK must preserve daemon typed error JSON consistently
across Runtime, Directory + Identity, Publication, Host Binding, Mission,
Admin, Events, Surface, Compatibility, and Receipt C ABI facades.

## Invariants

- The SPEC remains unchanged.
- C ABI integer return codes remain stable.
- Typed daemon error JSON, when present, is the authoritative error projection.
- Generic code-based errors use one shared SDKError construction path.
- No profile transport owns its own retry/stage/default error taxonomy.
- No product-specific naming or non-URA terminology is introduced.

## Implementation Steps

1. Audit duplicated Go C ABI `lastErrorOrCode` implementations.
2. Add a private shared helper for typed error JSON decoding and generic code
   projection.
3. Migrate profile-specific C ABI mappers to the shared helper while preserving
   their symbol-specific last-error reads.
4. Remove residual non-URA receipt terminology from touched conformance text.
5. Run Go, Python, conformance runner, scaffold, adapter reports, SPEC diff,
   whitespace, and forbidden terminology checks before committing.

## Boundary Proof

C ABI profile transports own symbol binding and handle lifetime. They do not
own daemon error taxonomy. Typed error JSON is already produced by the daemon
and decoded by SDK error helpers, so profile transports should only read the
profile-specific `easynet_last_error_json` symbol and delegate projection to a
shared SDK helper. This preserves stable C ABI return codes while avoiding a
second per-profile error mapping architecture.

## Verification Plan

- `go test ./...` in `sdk/go`
- `uv run python -m unittest discover tests` in `sdk/python`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- adapter report runs for Rust, C ABI, Go, and Python
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`
- forbidden addressing/product-drift terminology scan on touched files
