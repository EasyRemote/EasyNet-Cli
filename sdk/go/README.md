# Go Runtime SDK

This package is the Go implementation of the canonical, product-neutral
runtime model. It exposes only the shared seams used by downstream products:

- runtime host discovery, lifecycle, connection and health;
- Axon-backed canonical URA and descriptor-reference addressing;
- authority metadata and policy-safe invocation drafts;
- prepare/sign/submit with provider-owned key custody;
- unary, server-stream and bidirectional invocation state machines;
- typed errors and terminal receipt facts.

Downstream products own their ability names, DTOs, directory views, account
policy, HTTP routes and other workflows. They lower those workflows through
`RuntimeClient` and `Addressing`; no product profile clients, profile bundles,
product C symbols or local key vaults belong here.

The SDK never accepts or stores private key material. Runtime signing is an
opaque capability backed by a runtime key service, and all endpoint paths are
explicitly supplied by the embedding runtime.

## ABI v9 leased streams

`RuntimeClient.InvokeLeasedStream` and `OpenSignedLeasedStream` are the
explicit native ABI v9 path for large binary server-stream payloads. They are
enabled only when feature discovery and all three v9 symbols agree. Existing
`InvokeStream` remains on the Go-owned v8 `StreamEvent` representation.

Each `LeasedStreamEvent` owns at most one `LeasedPayload`. Consume it with
`ToBytes` (copy then release) or `WriteTo` (copy into Go-owned storage, write,
then release), or
call `Release`/`Close`. `Retain` creates another independently releasable
owner. Stream close and RuntimeClient close release outstanding native leases;
there is intentionally no finalizer-based correctness path.
