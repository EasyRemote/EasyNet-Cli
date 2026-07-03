# Go Daemon SDK

Go is the P0 facade for the EasyNet backend/Hub. It must expose daemon
lifecycle, Runtime Core, Directory + Identity, Receipt, Events, Admin +
Gateway, Surface, Compatibility, and selected wrapper profiles without
importing Axon packages or generated Axon protobufs in public APIs.

Current status: Runtime Core discovery/health/errors/invocation-draft/unary/
prepare-submit seam partial. The package exposes typed feature/version
discovery, runtime health readiness facts, schema-backed SDK error projection,
complete Invocation draft construction, prepared/signed Invocation DTOs, unary
InvocationResult projection, and RuntimeClient invoke/prepare/submit-signed
methods behind narrow JSON transport seams. Concrete daemon transport,
await/cancel/events convenience methods, profile clients, streams, bidi, and
backend cutover gates remain incomplete. See `../SDK_PARITY.md` before claiming
package stability.
