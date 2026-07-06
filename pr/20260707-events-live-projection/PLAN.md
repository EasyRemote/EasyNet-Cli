# Events Live Projection Plan

## Goal

Bridge Runtime Core server-stream chunks carrying daemon raw `DirectoryEvent`
payloads into SDK `EventFrame` values through the daemon/Rust Events projection
contract.

## Boundary Proof

- The SDK does not parse or classify daemon `DirectoryEvent` variants locally.
- Go/Python first accept already-projected `EventFrame` payloads for tests and
  transports that project at the source.
- When a runtime stream yields raw directory payloads, SDK facades call the
  Events projection provider/C ABI contract with an explicit cursor.
- Device/session/invocation streams remain unchanged until daemon-owned raw
  stream payload contracts exist.

## Invariants

1. Raw directory stream chunks are projected through daemon SDK Events contract.
2. The projection cursor is derived from the Runtime Core stream sequence.
3. Missing projection providers fail closed instead of fabricating frames.
4. Already-projected `EventFrame` payloads continue to work.
5. Go and Python expose the same EventStream `next` behavior for directory
   streams where a projection provider is wired.

## Verification

- Go Events runtime tests cover raw directory payload fallback.
- Python Events tests cover `EventStream.next` raw directory projection.
- Full Go/Python SDK tests and SDK scaffold/parity/cutover gates.

## Remaining After This Slice

- Daemon-owned raw payload contracts for device/invocation live streams.
- Backend SSE/WebSocket cutover to SDK event streams.
- Authority minting transport.
- RFC-007 receipt URA builder after Axon/daemon decision lands.
- Trust/certificate policy persistence.
