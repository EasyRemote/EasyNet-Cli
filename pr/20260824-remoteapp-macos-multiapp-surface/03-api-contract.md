# API contract

- macOS application target model becomes `multi_surface_application_window_set`.
- Application resource metadata exposes exact `resolved_window_ids`,
  `display_ids`, `window_set_epoch`, ordered `front_to_back_surfaces`,
  `surface_layout_epoch`, and application-union geometry.
- Capture proof remains one `ResolvedCaptureTargetProof` with one committed
  `AppWindowSetProof` plus one committed `AppSurfaceLayoutProof`; raw per-window
  native filters are plugin implementation details and are not public abilities
  or Invocation fields.
- Pointer rejection exposes stable `pointer_outside_target_surface` and
  `pointer_occluded` reasons; it does not silently redirect into display-global
  input.
- Existing `AppSurface` capture scope and WebRTC video track contract remain compatible.
