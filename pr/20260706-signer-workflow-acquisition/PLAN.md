# Signer Workflow Acquisition

## Objective

Close the SDK object-model gap between Directory + Identity signer-handle
projection and Runtime Core signing workflows. The SDK should let product
callers acquire a usable signer workflow from daemon-owned identity inventory
without hand-composing a `SignerHandle` and a signature provider in product
code.

## Boundary

- Axon still owns canonical signing material and Invocation bytes.
- The daemon/C ABI still owns identity inventory projection and signer-handle
  provenance.
- Go/Python SDK facades only compose existing SDK objects:
  `IdentityClient` obtains a daemon-authorized `SignerHandle`, then returns a
  Runtime Core `Signer` bound to the caller-supplied `SignatureProvider`.
- The SDK must not store private keys, choose keyring policy, fabricate signer
  provenance, or bypass signer-handle validation.

## Invariants

1. Existing `IdentityClient.signer` / `Signer` handle APIs remain available.
2. New acquisition methods validate the provider before contacting transport.
3. Returned signer objects have already passed signer-handle provenance checks.
4. Failure paths preserve typed SDK errors from the existing identity/signing
   boundary.
5. No Axon grammar, canonical byte, receipt, or keyring policy logic is added
   to language facades.

## Implementation

- Add Go `IdentityClient.AcquireSigner(ctx, req, provider)`.
- Add Python `IdentityClient.acquire_signer(request, provider)`.
- Add focused Go/Python tests proving transport-backed handle acquisition,
  provider validation, and forged handle rejection through the composed object.
- Update parity/conformance notes to distinguish SDK-owned signer workflow
  acquisition from remaining live daemon keyring policy cutover.

## Verification

- `go test ./... -run 'Identity|Signing|Conformance'`
- `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_identity tests.test_signing tests.test_conformance`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- `git diff --check`
