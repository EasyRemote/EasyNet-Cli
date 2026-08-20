# Intent

Make `easynet runtime status --json` render desktop companion DTOs from
explicit lifecycle observations instead of performing companion package I/O
inside the JSON renderer.

This keeps runtime status aligned with the SPEC unified control-plane contract:
the lifecycle report carries `DesktopCompanionStatus` DTO values and rendering
does not reclassify companion state.
