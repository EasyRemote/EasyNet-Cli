# Runtime Python SDK

Install the `easynet-sdk` distribution and import it as `easynet_sdk`:

```bash
pip install easynet-sdk
```

The Python package is the Python binding of the canonical, product-neutral
runtime SDK. It is not a product profile bundle.

Downstream products own their workflow names, request/result DTOs, account
policy, HTTP routes, UI projections and product lifecycle. They consume the
generic runtime concepts in this package instead of importing raw FFI handles,
Axon protobufs, daemon configuration files or product-specific compatibility
modules.

## Runtime surface

The package exposes the same capability families represented in
`../conformance/sdk-parity-matrix.json`:

- SDK environment and runtime-host lifecycle;
- canonical Addressing delegated to Axon;
- complete Invocation draft, prepare/sign/submit, unary, stream and bidi
  lifecycle;
- runtime identity projection and runtime-managed signing handles;
- managed signing through the runtime key service;
- product-neutral PrincipalLifecycle, enrollment, public-key bindings,
  recovery and authorization grants;
- product-neutral access-control and authority metadata projection;
- canonical Directory resolve/list/subscribe;
- receipt, causal reference and trace projection;
- bounded runtime event streams with cursors and resume semantics;
- runtime administration, health, diagnostics and typed errors.

Missing or unavailable providers fail explicitly. The SDK does not search for
fallback transports, parse product directories, infer runtime-host state from product
environment variables or expose private key material.

For large native binary payloads, ABI v9 is available only through the explicit
leased-stream surface documented in [LEASED_STREAMS.md](LEASED_STREAMS.md).
The existing owned `StreamEvent` API remains unchanged and does not silently
switch ownership models.

## Product boundary

The following belong downstream, not in this package:

- workflow orchestration and hosted execution product workflows;
- account, OAuth, HTTP, pairing and dashboard DTOs;
- remote-control workflow objects;
- publication, page/model/file, host-binding, wrapper, desktop lifecycle and
  other product helper clients;
- product Directory views, product receipt pages and product event
  presentations.

Downstream products may build typed local facades over `RuntimeClient`,
`AddressingClient`, `PrincipalClient`, `DirectoryClient`, `ReceiptClient`,
`RuntimeEventClient` and `RuntimeAdminClient`. Those facades should live in the
product repository that owns the behavior.

## Signing custody

`SignatureProvider` is the generic seam for signatures produced by an external
signer selected by the consumer. The Python SDK never accepts or stores a
private key or seed. Keys managed by the local runtime are available only
through runtime-backed signing and managed-signing surfaces.

## Verification

The public Python exports are covered by
`../conformance/canonical-public-api.json`. Go/Python capability state is
covered by `../conformance/sdk-parity-matrix.json`. Product-neutrality is
enforced by `../../tools/scripts/check-sdk-product-neutrality.sh`.

Before tagging a Python SDK release, synchronize and verify its independent
version line without changing the runtime-host version:

```bash
SDK_VERSION="$(tide mark --local-only)"
./tools/scripts/update-python-sdk-version.sh "$SDK_VERSION"
./tools/scripts/update-python-sdk-version.sh --check "$SDK_VERSION"
```

Capture the release mark only after functional code is frozen. Keep that value
through the metadata commit and tag; recomputing Tide after another commit would
describe a different HEAD.

## Source release scope

This distribution is a deliberately bounded public SDK, not a source release
of every downstream control-plane service, research mechanism, or evaluation
asset. The [release-scope statement](../../SOURCE_RELEASE_SCOPE.md)
explains the staged policy. It does not restrict the Apache-2.0 rights granted
for files actually included in this distribution.
