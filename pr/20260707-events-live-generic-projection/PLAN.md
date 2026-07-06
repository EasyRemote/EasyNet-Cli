# Events Live Generic Projection

Date: 2026-07-07

## Goal

Complete the Events profile live-stream projection contract for device and
invocation streams without moving event semantics into Go or Python facades.

Target dependency direction:

```text
daemon raw stream payload -> EasyNet-Cli Events contract/C ABI -> Go/Python EventStream
```

## Boundary Proof

- The SDK must not create a new event bus or backend fan-out path.
- Go/Python must not classify daemon event payloads locally.
- The Rust protocol contract owns the SDK `EventFrame` projection DTO.
- C ABI exposes the same projection entry point used by language bindings.
- Already projected `EventFrame` payloads remain accepted as a fast validation
  path, but raw daemon payloads must fail closed unless a projection provider is
  available.

## Invariants

1. Directory raw payloads continue to project through `DirectoryEvent` rules.
2. Device raw live payloads project through the same daemon SDK event row
   contract as device history, with live-stream metadata.
3. Invocation raw live payloads produce typed invocation subject refs without
   fabricating URAs.
4. Stream cursor sequence comes from Runtime Core stream events.
5. Go and Python use the same projection provider shape.
6. Missing projection provider remains an explicit SDK error, not a fallback
   parser.

## Acceptance

- Add a shared `project_live_event` Rust contract and C ABI symbol.
- Add Go `ProjectLiveEvent` transport/client path and wire it into
  `EventStream.Next` for directory/device/invocation raw payloads.
- Add Python `project_live_event` transport/client path and wire it into
  runtime-backed `EventStream.next`.
- Extend conformance and parity docs to mark device/invocation raw live payload
  projection as provider-backed, while leaving backend SSE/WebSocket cutover
  incomplete.
- Run Rust targeted tests, Go SDK tests, Python SDK tests, and SDK scaffold/
  parity gates before commit.

## Remaining After This Slice

- Backend repository cutover from raw Axon/proto/direct daemon transports.
- Backend SSE/WebSocket fan-out over the Events SDK.
- Authority minting transports and trust-anchor admission execution cutover.
- RFC-007 receipt URA construction after Axon/daemon decision.
- Trust/certificate persistence and product pairing lifecycle cutover.
