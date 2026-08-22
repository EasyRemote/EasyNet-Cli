# RemoteApp multi-window tracking evidence invariants

1. A live pass must use `proof_mode=real_multi_window_tracking_matrix`.
2. `component_mock=false` and `real_backend_runtime=true` are required.
3. The artifact must keep `product_complete_claim=false`.
4. Every passing scenario must bind `remote_desktop.create_session`,
   `remote_desktop.attach`, `remote_desktop.watch_events`, and
   `remote_desktop.end_session` to the same selected Resource URA and session.
5. Independent stream evidence must show at least two concurrent sessions whose
   selected Resource URAs, session ids, stream ids, media source epochs, and
   frame source ids are distinct and non-interleaved.
6. Geometry evidence must include ordered `TARGET_MOVED` and `TARGET_RESIZED`
   lifecycle events with increasing `target_geometry_revision`.
7. Application churn evidence must include same-display window-set expansion or
   contraction, pending media rebind, `TARGET_REBOUND`, updated binding epoch,
   rendered frames after rebind, and no first-display fallback.
8. Target loss evidence must include `TARGET_LOST` and either explicit
   rebind-required failure or successful bounded rebind.
9. Multi-display application evidence must either pass with `MultiAppSurface`
   support or expose explicit product unsupported state without starting a
   capture session.
10. Every passing scenario must expose a terminal receipt with a deterministic
    session terminal reason.
