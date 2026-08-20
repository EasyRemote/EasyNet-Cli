# Swift Runtime Core Seam Boundary Proof

## Generic Runtime Model

The Swift package defines Runtime Core surface objects that mirror the shared SDK model: `FeatureSet`, `SDKError`, `InvocationTuple`, `InvocationDraft`, `RuntimeClient`, `StreamHandle`, and `BidiSession`. These are not product abstractions and do not encode EasyNet or EasyRemote lifecycle state.

## Provider Boundary

Swift accepts `DiscoveryTransport`, `RuntimeTransport`, `StreamSource`, and `BidiSource` as injected provider interfaces. The seam never creates a daemon process, opens a product route, or binds to a C ABI symbol. That keeps the seam testable while preventing a false provider-backed claim.

## Protocol Boundary

The public Swift API intentionally avoids generated protocol packages and protocol-specific public flags. Feature discovery uses generic `profiles`, `symbols`, and `protocolBridgeAvailable` fields. Invocation construction carries a complete canonical tuple through SDK-owned value types, but signing, canonical bytes, and authority verification remain outside this seam.

## Product Boundary

The Swift seam contains no product-specific directory model, receipt model, health route, identity route, backend state, EasyRemote process lifecycle, or product namespace. Those responsibilities stay in downstream consumers or future provider-backed SDK layers.
