# Stream/Bidi Cancel Terminal Authority Gate

## Objective

Close the public-boundary part of A69/A70 with an executable architecture rule.
Stream and bidi cancellation at the current EasyNet C ABI provider boundary is
a local resource/callback cancellation request, not canonical runtime
terminality.

## Invariants

1. ABI v5 must describe stream and bidi cancel as cancel-request operations.
2. ABI v5 must forbid local cancel from claiming lifecycle terminality without
   a canonical terminal receipt.
3. Go C ABI stream cancel projection returns `CancelRequested` with
   `terminal=false`.
4. Go C ABI bidi cancel projection returns `CancelRequested` with
   `terminal=false`.
5. Python C ABI stream cancel projection returns `CancelRequested` with
   `terminal=False`.
6. Python C ABI bidi cancel projection returns `CancelRequested` with
   `terminal=False`.
7. The Rust C ABI stream/bidi cancel path must not synthesize
   `InvocationState::Cancelled` or terminal JSON.

## Effect

This slice preserves public behavior and makes the remaining lifecycle-control
gap explicit. The provider-local cancel path remains a request/resource state
until a later slice routes stream/bidi cancellation through Axon lifecycle
control and awaits canonical terminal receipt proof.
