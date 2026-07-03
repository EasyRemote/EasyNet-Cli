# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core feature-discovery partial. The package exposes
typed feature/version discovery and SDK errors behind a narrow transport
protocol. Invocation transport, profile clients, streams, bidi, host binding,
and EasyRemote cutover gates remain incomplete. See `../SDK_PARITY.md` before
claiming package stability.
