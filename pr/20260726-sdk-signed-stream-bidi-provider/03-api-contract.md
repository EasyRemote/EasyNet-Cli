# API Contract

Public-compatible additions:
- Go: `RuntimeClient.OpenSignedStream(ctx, signed)` and `RuntimeClient.OpenSignedBidi(ctx, signed, streams)`.
- Python: `RuntimeClient.open_signed_stream(signed)` and `RuntimeClient.open_signed_bidi(signed, streams)`.

No behavior removed from existing public APIs:
- Existing unsigned `open_stream` / `OpenStream` continue to accept `InvocationDraft`.
- Existing AuthorizedRuntimeSession stream/bidi calls now reach the runtime provider instead of returning provider unavailable.
