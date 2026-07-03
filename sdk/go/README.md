# Go Daemon SDK

Go is the P0 facade for the EasyNet backend/Hub. It must expose daemon
lifecycle, Runtime Core, Directory + Identity, Receipt, Events, Admin +
Gateway, Surface, Compatibility, and selected wrapper profiles without
importing Axon packages or generated Axon protobufs in public APIs.

Current status: Runtime Core discovery/connection/health/errors/invocation-
draft/unary/stream/bidi/handle/prepare-submit seam partial. The package exposes typed
feature/version discovery, runtime connection state, runtime health readiness
facts, schema-backed SDK error projection, complete Invocation draft
construction, prepared/signed Invocation DTOs, unary InvocationResult
projection, StreamHandle state observation, BidiSession frame ordering,
half-close, cancel, and terminal-close observation, InvocationHandle
await/cancel/events observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/submit-signed
methods behind narrow JSON transport seams. Concrete daemon lifecycle/transport,
profile clients, concrete bidi adapters, and backend cutover gates remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.
