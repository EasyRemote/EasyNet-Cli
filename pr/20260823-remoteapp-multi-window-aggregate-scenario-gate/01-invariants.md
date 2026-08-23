# Invariants

1. Product completion requires all multi-window scenarios:
   `independent_window_streams`, `geometry_churn`,
   `application_window_set_churn`, `target_loss_rebind`, and
   `multi_display_application`.
2. Every scenario summary must bind a selected Resource URA and session id.
3. Every scenario must render frames.
4. Independent stream evidence must contain at least two streams with distinct
   Resource URAs, session ids, stream ids, frame source ids, media source
   epochs, and sentinel ids.
5. Independent streams must prove no frame interleaving and no cross-stream
   sentinel leakage.
6. Geometry churn must include move and resize events.
7. Application window-set churn must include pending media rebind, target
   rebound, frames after rebind, committed sentinel rendering, no uncommitted
   same-app sentinel, and no display fallback.
8. Target loss must include loss plus bounded rebound or explicit rebind
   failure with an actionable frontend recovery.
9. Multi-display application must be a passed `MultiAppSurface`, not an
   unsupported readiness state.
