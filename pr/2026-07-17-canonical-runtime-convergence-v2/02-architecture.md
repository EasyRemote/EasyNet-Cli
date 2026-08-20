# Architecture

## Layering

```text
SDK canonical runtime model
  -> descriptor-bound InvocationDraft / request builder
  -> daemon policy tuple input
  -> LocalRuntime
  -> terminal receipt or event
```

The SDK owns generic runtime type shape and conformance. EasyNet-Cli supplies
daemon policy facts and provider implementations, then delegates canonical
runtime behavior to Axon runtime entry points.

## Runtime Ownership

- `DaemonInvocation` is the EasyNet-Cli policy input for complete tuple
  submission. It is not a second canonical envelope.
- Axon-owned builders encode canonical material and descriptor-bound admission
  requests.
- `LocalRuntime` is the only daemon execution entry point for ability
  invocation, stream, and bidi routes.
- Direct daemon response synthesis is limited to boot, health, status, and
  diagnostics.

## SDK Ownership

- Go and Python SDKs are two facades over the same capability matrix.
- A capability state is exactly `Unsupported`, `Seam`, `ProviderBacked`, or
  `CutoverReady`.
- Public syntax may differ by language, but state transitions and error
  contracts come from shared vectors.
- Canonical SDK surfaces must reject EasyNet, EasyRemote, audio, MCP,
  tool-adapter, preset, and Mission product packages unless they are explicitly
  downstream adapters outside the canonical SDK.

## Mission/EAL Boundary

Mission/EAL is a daemon-owned composite `AbilityImpl` strategy. A Mission/EAL
step that calls another ability creates a child invocation with explicit causal
parentage. Axon preserves only generic child-invocation, causal-chain,
cancellation, deadline, and receipt primitives.

## Schema Boundary

EasyNet-Cli may consume generated Axon protocol code, but it must not edit a
forked schema source. Schema-copy checks delegate to the canonical Axon proto
sync script so ownership remains single-source.
