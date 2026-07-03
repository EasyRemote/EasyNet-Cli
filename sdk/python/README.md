# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core discovery/connection/health/errors/invocation-
draft/unary/stream/handle/prepare-submit seam partial. The package exposes typed
feature/version discovery, runtime connection state, runtime health readiness
facts, schema-backed SDK error projection, complete Invocation draft
construction, prepared/signed Invocation DTOs, unary InvocationResult
projection, StreamHandle state observation, InvocationHandle await/cancel/events
observation, and RuntimeClient invoke/invoke-stream/prepare/submit-signed
methods behind narrow transport protocols. Concrete daemon lifecycle/transport,
profile clients, bidi, host binding, and EasyRemote cutover gates remain
incomplete. See `../SDK_PARITY.md` before claiming package stability.
