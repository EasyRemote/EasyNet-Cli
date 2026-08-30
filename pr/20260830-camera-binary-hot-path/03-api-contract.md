# API contract

- `camera.subscribe` keeps its ability identity and input schema.
- Non-terminal payload content type is `image/jpeg`; payload is the exact JPEG
  byte sequence rather than a JSON object containing `image_bytes_b64`.
- Frame sequence and lifecycle remain Runtime metadata, not JPEG payload data.
- `camera.snapshot` remains RPC-shaped and returns exact JPEG bytes with
  `result_content_type=image/jpeg`. Invocation metadata/receipts remain on the
  Runtime envelope and receipt plane; image bytes are never embedded in JSON.
- `camera.record_stop` keeps its JSON receipt shape while media persistence is
  changed internally to a file commit.

The camera stream and snapshot payload corrections are descriptor-significant
and must be reflected in their descriptor versions/descriptions before
publication.
