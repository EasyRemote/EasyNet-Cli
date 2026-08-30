# Executable Recovery Delta

- Product-flow gate now requires the frontend to expose a `Refresh targets` CTA
  when daemon target diagnostics request `refresh_targets`.
- Gate regression coverage rejects a frontend that only renders target recovery
  text without the executable CTA.
- Readiness evidence records this as application/window recovery UX, not as
  cross-platform capture or multi-window churn completion.
