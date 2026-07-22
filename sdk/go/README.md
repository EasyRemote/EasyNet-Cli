# Go Runtime SDK

This package is the Go implementation of the canonical, product-neutral
runtime model. It exposes only the shared seams used by downstream products:

- runtime host discovery, lifecycle, connection and health;
- Axon-backed canonical URA and descriptor-reference addressing;
- authority metadata and policy-safe invocation drafts;
- prepare/sign/submit with provider-owned key custody;
- unary, server-stream and bidirectional invocation state machines;
- typed errors and terminal receipt facts.

EasyNet, EasyRemote and future products own their ability names, DTOs,
directory views, account policy, HTTP routes and other workflows. They lower
those workflows through `RuntimeClient` and `Addressing`; no product profile
clients, profile bundles, product C symbols or local key vaults belong here.

The SDK never accepts or stores private key material. Runtime signing is an
opaque capability backed by the daemon key service, and all endpoint paths are
explicitly supplied by the embedding runtime.
