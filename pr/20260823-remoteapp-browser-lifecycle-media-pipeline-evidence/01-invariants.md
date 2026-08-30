# Invariants

1. `media_presented` proves rendered media only; it does not prove media
   capability transparency.
2. Browser/Tauri evidence must separately prove
   `media_pipeline_support_visible`.
3. The visible media pipeline evidence must include video-only scope, H.264,
   bounded stale-frame drop policy, and `host_audio_not_implemented`.
4. The verifier remains an evidence validator; self-test is not product
   evidence.
