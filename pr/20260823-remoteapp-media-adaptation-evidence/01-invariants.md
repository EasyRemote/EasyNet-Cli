# RemoteApp media adaptation evidence invariants

1. A live pass must use `proof_mode=real_media_adaptation_matrix`.
2. `component_mock=false` and `real_backend_runtime=true` are required.
3. The artifact must keep `product_complete_claim=false`.
4. Every scenario must bind `remote_desktop.create_session`,
   `remote_desktop.attach`, `remote_desktop.watch_events`, and
   `remote_desktop.end_session` to the same selected Resource URA and session.
5. Video evidence must include negotiated codec, content type, transport,
   rendered frames, encoded frames, requested FPS, effective FPS, measured FPS,
   target bitrate, observed bitrate, keyframe cadence, and latency.
6. Audio evidence must prove a real host audio path with negotiated codec,
   rendered packets or samples, sample rate, channel count, and non-muted
   playback/capture evidence.
7. Degraded-network evidence must prove bitrate/FPS adaptation and rendered
   media after adaptation.
8. Backpressure evidence must prove bounded queue depth and an exercised stale
   frame drop policy.
9. Drop policy must be explicit; unbounded queues or source-only stats are not
   acceptable.
10. Every scenario must expose a terminal receipt with a deterministic session
    terminal reason.
