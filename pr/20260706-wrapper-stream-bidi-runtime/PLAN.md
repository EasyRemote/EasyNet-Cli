# Wrapper Stream/Bidi Runtime Facades

## Objective

Converge Go and Python Convenience Wrapper facades so terminal, remote desktop,
browser, and media session helpers can open Runtime Core stream and bidi
sessions without creating a wrapper-owned protocol or expanding the mandatory
carrier/projection transport interface.

## Boundary Proof

- Wrapper helpers only lower typed session start requests into complete
  Invocation drafts and delegate lifecycle to `RuntimeClient.InvokeStream` or
  `RuntimeClient.OpenBidi`.
- Axon/Runtime remains the owner of stream and bidi terminal-state semantics,
  ordering, backpressure, and frame/event decoding.
- Wrapper facade methods are optional convenience entry points over governed
  wrapper abilities; the underlying Invocation carrier builders remain
  inspectable and complete.
- Existing wrapper carrier/projection transports must not become responsible
  for stream/bidi sessions. They fail closed unless composed with
  `WrapperRuntimeTransport`/`RuntimeWrapperTransport`.

## Implementation Steps

1. Add optional Go/Python wrapper session transport interfaces/protocols.
2. Add `WrapperClient` stream/bidi methods for terminal, remote desktop,
   browser, and media start requests.
3. Implement the optional methods in runtime-backed wrapper transports by
   reusing existing Invocation draft construction.
4. Add Go/Python tests proving RuntimeClient stream/bidi delegation and
   fail-closed behavior for carrier-only transports.
5. Update wrapper conformance/parity docs to remove the daemon-SDK-side
   concrete stream/bidi adapter gap while preserving product bridge cutovers.

## Verification

- `go test ./... -run 'Wrapper|Conformance'`
- `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_wrappers tests.test_conformance`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Go/Python `sdk-conformance-runner`
- `git diff --check`

## Remaining After This Slice

- Backend HTTP/WebSocket bridges, storage policy, and external product wrapper
  cutovers remain outside the daemon SDK facade.
- Product-specific browser/terminal/media transport UX remains product-owned
  and must not be moved into SDK wrapper helpers.
