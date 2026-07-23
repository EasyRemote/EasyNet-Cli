# API Contract

No public ABI or JSON field is intentionally changed.

Internal contract:

- `stream_chunk_json` and `bidi_down_frame_json` become explicit projection builders;
- callers receive a `CallbackFrameProjection`;
- canonical terminality is consumed through a typed accessor, not JSON lookup.

