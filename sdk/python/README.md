# Python Runtime SDK

The Python package is the Python binding of the canonical, product-neutral
EasyNet runtime SDK. It is not an EasyRemote SDK and it is not a product
profile bundle.

Products such as EasyNet Backend, EasyRemote and future applications own their
workflow names, request/result DTOs, account policy, HTTP routes, UI
projections and product lifecycle. They consume the generic runtime concepts in
this package instead of importing raw FFI handles, Axon protobufs, daemon
configuration files or product-specific compatibility modules.

## Runtime surface

The package exposes the same capability families represented in
`../conformance/sdk-parity-matrix.json`:

- SDK environment and daemon/runtime lifecycle;
- canonical Addressing delegated to Axon;
- complete Invocation draft, prepare/sign/submit, unary, stream and bidi
  lifecycle;
- runtime identity projection and daemon-managed signing handles;
- managed signing through the daemon key-service;
- product-neutral PrincipalLifecycle, enrollment, public-key bindings,
  recovery and authorization grants;
- product-neutral access-control and authority metadata projection;
- canonical Directory resolve/list/subscribe;
- receipt, causal reference and trace projection;
- bounded runtime event streams with cursors and resume semantics;
- runtime administration, health, diagnostics and typed errors.

Missing or unavailable providers fail explicitly. The SDK does not search for
fallback transports, parse product directories, infer daemon state from product
environment variables or expose private key material.

## Product boundary

The following belong downstream, not in this package:

- Mission/EAL product workflows;
- Backend account, OAuth, HTTP, pairing and dashboard DTOs;
- EasyRemote Control, Pipeline, Context and remote-control workflow objects;
- Publication, Pages/Surface, OpenAI compatibility, HostBinding, wrappers,
  desktop companion and other product helper clients;
- product Directory views, product receipt pages and product event
  presentations.

Downstream products may build typed local facades over `RuntimeClient`,
`AddressingClient`, `PrincipalClient`, `DirectoryClient`, `ReceiptClient`,
`RuntimeEventClient` and `RuntimeAdminClient`. Those facades should live in the
product repository that owns the behavior.

## Signing custody

`SignatureProvider` is the generic seam for signatures produced by an external
signer selected by the consumer. The Python SDK never accepts or stores a
private key or seed. Keys managed by the local EasyNet runtime are available
only through daemon-backed runtime signing and managed-signing surfaces.

## Verification

The public Python exports are covered by
`../conformance/canonical-public-api.json`. Go/Python capability state is
covered by `../conformance/sdk-parity-matrix.json`. Product-neutrality is
enforced by `../../tools/scripts/check-sdk-product-neutrality.sh`.
