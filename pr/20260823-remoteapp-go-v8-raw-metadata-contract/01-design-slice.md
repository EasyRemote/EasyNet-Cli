# RemoteApp Go v8 raw stream metadata contract

## Product seam

RemoteApp high-frequency media streams need ABI v8 raw bytes, but raw bytes must
remain only a transport representation. The Go SDK accepted v8 raw stream
metadata through the normal JSON event decoder, so metadata could omit canonical
Runtime Core lifecycle, receipt, and error fields while still delivering raw
payload bytes. Python already rejected that shape.

## Invariants

- ABI v8 raw stream metadata must explicitly include `sequence`, `kind`,
  `state`, `terminal`, `transport_terminal`, `payload_content_type`,
  `admission_receipt`, `terminal_receipt`, and `error`.
- `terminal` and `transport_terminal` must be booleans, `state` and
  `payload_content_type` must be strings.
- Raw metadata must not duplicate payload through `payload_base64` or
  `payload_json`.
- Payload bytes may be empty for terminal, error, receipt, or EOF frames; EOF
  remains transport-owned and is not modeled as a data frame.

## Expected impact

This closes the Go facade seam where EasyRemote/RemoteApp could use v8 for
binary payload efficiency while silently weakening stream lifecycle proof. The
SDK still preserves v7 fallback behavior; the stricter contract only applies to
raw v8 packets.

## Verification

- Failed first because the command was run from the repository root instead of
  the Go module: `go test ./sdk/go -run 'TestRawStreamPacket|TestCABIRuntimeProvider(DispatchesStreamBeforeTerminal|FallsBackToV7StreamOpen|PreservesStreamOrderAndSingleTerminal|RejectsCallbackBackpressure|MemoizesConcurrentStreamCancellation)'`.
- Failed before fixture repair, proving the new gate caught incomplete v8
  metadata: `go test -tags runtime_cabi . -run 'TestCABIRuntimeProvider(DispatchesStreamBeforeTerminal|FallsBackToV7StreamOpen|PreservesStreamOrderAndSingleTerminal|RejectsCallbackBackpressure|MemoizesConcurrentStreamCancellation)'`.
- Passed: `go test . -run 'TestRawStreamPacket'`.
- Passed: `go test .`.
- Passed: `go test -tags runtime_cabi . -run 'TestCABIRuntimeProvider(DispatchesStreamBeforeTerminal|FallsBackToV7StreamOpen|PreservesStreamOrderAndSingleTerminal|RejectsCallbackBackpressure|MemoizesConcurrentStreamCancellation)'`.
