# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core discovery/health/errors/invocation-draft partial.
The package exposes typed feature/version discovery, runtime health readiness
facts, schema-backed SDK error projection, and complete Invocation draft
construction behind narrow transport protocols. Prepare/sign/submit,
Invocation transport, profile clients, streams, bidi, host binding, and
EasyRemote cutover gates remain incomplete. See `../SDK_PARITY.md` before
claiming package stability.
