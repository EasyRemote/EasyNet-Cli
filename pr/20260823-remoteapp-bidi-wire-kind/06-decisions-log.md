# Decisions Log

- 2026-08-23: Treat RemoteApp attach as `metadata_json_plus_binary` because the implementation emits metadata JSON plus raw binary frame chunks.
- 2026-08-23: Keep dispatcher behavior unchanged in this branch to avoid colliding with in-flight bidi dispatcher work; map the new manifest declaration onto the existing binary-capable local adapter.
