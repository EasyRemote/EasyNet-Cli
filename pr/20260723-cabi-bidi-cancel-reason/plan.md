# C ABI bidi cancel reason convergence

## Goal

Remove the Go C ABI bidi cancellation reason fallback that projects every local
cancel request as `"cancelled"` instead of the caller-owned cancellation reason.

## Root abstraction problem

Bidi cancellation is a lifecycle transition with caller-owned intent. The SDK
model already carries a reason on `BidiOutcome`, and the Python C ABI transport
echoes the explicit caller reason. The Go C ABI transport currently discards
that input and synthesizes a fixed reason. That creates a language-specific
runtime projection and hides caller intent from products.

## Invariants

1. Go C ABI bidi cancel outcome uses the explicit `reason` argument.
2. Go C ABI bidi cancel no longer contains a fixed `"cancelled"` reason
   projection or `_ = reason` discard.
3. The C ABI wire call remains unchanged in this slice; this patch does not
   claim daemon-side stream/bidi cancel reason custody.
4. Public Go SDK shape remains stable because `BidiOutcome.Reason()` already
   exists.
5. SPEC v2 rejects reintroducing this fixed reason fallback.

## Boundary proof

- The C ABI transport owns the local SDK outcome projected after submitting a
  cancel request to the native runtime.
- The native C ABI owns whether the cancel request reaches the runtime.
- The SDK projection must not invent lifecycle intent; it can only echo the
  caller-owned reason it was given.

## Verification plan

- Go C ABI runtime focused tests.
- Canonical runtime convergence v2 gate.
- Canonical public API gate if source hashes change.
- Repository formatting checks.
- Codegraph sync/status.

## Decisions

- Keep the native C ABI symbol shape unchanged in this slice because stream and
  bidi cancel reason custody at the daemon wire layer needs a separate ABI
  revision.
- Remove only the SDK-side invented reason value. The local C ABI transport now
  projects the caller-owned reason it already receives through the Go
  `BidiTransport` contract.
- Use `json.Marshal` for the outcome projection instead of hand-built JSON so
  cancellation reasons remain valid JSON regardless of caller text.

## Delta

- Go C ABI bidi cancel no longer discards `reason`.
- Go C ABI bidi cancel outcome now encodes `reason` from the caller-owned
  method argument.
- Go C ABI bidi lifecycle regression test now asserts reason preservation.
- SPEC v2 gate now rejects the fixed `"cancelled"` reason fallback and missing
  regression coverage.
- Go C ABI provider source attestation was refreshed.

## Results

- `go test . -tags runtime_cabi -run 'TestCABIRuntimeProviderRequestsBidiCancelBeforeCanonicalTerminal|TestCABIRuntimeProviderMemoizesConcurrentBidiCancellation|TestCABIRuntimeProviderDispatchesBidiBeforeTerminal'`
  passed in `sdk/go`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `cargo fmt --check` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `tools/scripts/check-sdk-canonical-public-api.sh` passed.
- `go test . -tags runtime_cabi -run 'TestCABIRuntimeProvider.*Bidi|TestCABIBidiFrameJSON|TestProjectCABIOrderedEventKeepsCanonicalBidiReceipts'`
  passed in `sdk/go`.
- `codegraph sync . && codegraph status .` completed with an up-to-date index.
