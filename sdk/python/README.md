# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core discovery/health/errors partial. The package
exposes typed feature/version discovery, runtime health readiness facts, and
schema-backed SDK error projection behind narrow transport protocols.
Invocation transport, profile clients, streams, bidi, host binding, and
EasyRemote cutover gates remain incomplete. See `../SDK_PARITY.md` before
claiming package stability.
