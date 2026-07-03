# Go Daemon SDK

Go is the P0 facade for the EasyNet backend/Hub. It must expose daemon
lifecycle, Runtime Core, Directory + Identity, Receipt, Events, Admin +
Gateway, Surface, Compatibility, and selected wrapper profiles without
importing Axon packages or generated Axon protobufs in public APIs.

Current status: Runtime Core discovery/health/errors partial. The package
exposes typed feature/version discovery, runtime health readiness facts, and
schema-backed SDK error projection behind narrow transport interfaces.
Invocation transport, profile clients, streams, bidi, and backend cutover gates
remain incomplete. See `../SDK_PARITY.md` before claiming package stability.
