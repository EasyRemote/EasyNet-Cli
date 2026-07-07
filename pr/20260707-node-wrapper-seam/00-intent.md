# Node Wrapper Seam Intent

Add a Node/TypeScript Convenience Wrapper profile seam that matches
`docs/spec/daemon-sdk-requirements-v1.md` without making wrapper helpers the
canonical access path or importing backend HTTP/WebSocket bridge policy into the
SDK.

## Scope

- Expose Node projection DTOs for file records, terminal sessions, remote
  desktop sessions, browser sessions, and media sessions.
- Delegate wrapper projection to injected transport methods.
- Declare Node for `wrappers/profile_records` only with direct Node test
  evidence.

## Out Of Scope

- No backend HTTP/WebSocket bridge.
- No browser, terminal, remote desktop, or media runtime implementation.
- No replacement for direct Runtime Core Invocation/stream/bidi access.
