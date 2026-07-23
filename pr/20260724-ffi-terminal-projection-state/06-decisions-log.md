# Decisions Log

- Decision: do not make the public JSON field the lifecycle authority. The public field remains as an output compatibility shape; the internal reader receives terminality from the projection object.
- Decision: keep `stream_chunk_json` / `bidi_down_frame_json` public JSON shape unchanged for SDK compatibility, but change their return type to an internal `CallbackFrameProjection`.
- Decision: mark the JSON accessor test-only. Production control flow consumes `is_canonical_terminal()` and `into_json_bytes()` only.
