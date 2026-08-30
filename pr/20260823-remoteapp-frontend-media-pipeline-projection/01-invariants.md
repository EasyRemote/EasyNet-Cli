# Invariants

1. The frontend must not derive full audio/video readiness from WebRTC stats.
2. `media_pipeline_support.product_ready=false` must stay visible when host
   audio or live media-adaptation E2E is missing.
3. Product blockers are daemon-authored strings; the frontend may display them
   but must not reinterpret them into a ready state.
4. The session details surface should show video scope, codec, drop policy, and
   audio/E2E blockers in one compact label.
