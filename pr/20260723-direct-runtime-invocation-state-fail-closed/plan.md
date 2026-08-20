# Direct runtime invocation state fail-closed

## Goal

Remove Go/Python direct runtime fallback projection of unknown invocation lifecycle states to `Unspecified`.

The direct runtime provider is the SDK's provider-backed implementation path. If it silently maps missing or unknown lifecycle enum values into a display string, downstream products can observe a non-canonical lifecycle state instead of a protocol failure.

## Boundary proof

- This slice only changes SDK direct runtime projection of invocation lifecycle state.
- Generated protobuf fields and enum constants remain untouched.
- Known invocation states continue to project to the same canonical names.
- Unknown or `INVOCATION_STATE_UNSPECIFIED` values fail closed with protocol errors before result/event/receipt dicts are emitted.
- Go and Python converge on the same behavior.

## Invariants

1. Unary direct runtime responses cannot emit `terminal_state = "Unspecified"`.
2. Stream direct runtime chunks cannot emit `state = "Unspecified"`.
3. Receipt projections cannot emit `state = "Unspecified"`.
4. Bidi receipt frames fail closed before projection if their receipt state is unknown or unspecified.
5. SPEC v2 gate rejects reintroduction of default `Unspecified` fallback in Go/Python direct runtime state projection.

## Verification plan

- Go direct runtime focused tests.
- Python direct runtime focused tests.
- SPEC v2 gate.
- SDK product-neutrality, architecture convergence, public API gates.
- codegraph sync/status.

## Delta log

- Made Go direct runtime invocation state projection fallible instead of defaulting unknown states to `Unspecified`.
- Made Go receipt projection fallible so receipt dictionaries cannot carry unsupported lifecycle states.
- Made Python direct runtime `_state_name` fail closed with `PROTOCOL` instead of returning `Unspecified`.
- Added Go/Python tests for unary and stream UNSPECIFIED state rejection.
- Added SPEC v2 structural and mutation coverage for direct runtime state projection.
- Rebuilt SDK public API/conformance manifests after the Go provider implementation hash changed.
- Verified focused Go/Python direct runtime tests, fmt, SPEC v2, SDK product-neutrality, architecture convergence, public API, and codegraph.
