# Invariants

- Python does not construct Axon Invocation semantics directly. It asks the C ABI compatibility builder for the complete Invocation JSON.
- Python does not project OpenAI stream chunks into SDK DTOs. It aggregates daemon stream payloads and calls `easynet_compatibility_project_chat_stream`.
- Every opened stream is closed on success, projection failure, timeout, or protocol failure.
- Terminal frames end aggregation. Non-terminal frames must carry `payload_json` chunks from Runtime Core.
- The slice must leave admin, events, directory subscription, publication management, and surface health gaps untouched until their lower contracts exist.
