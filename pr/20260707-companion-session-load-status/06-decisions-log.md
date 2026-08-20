# Decisions Log

- 2026-07-07: Treat Linux graphical-session detection as a shared companion runtime probe consumed by both load planning and the Linux supervisor.
- 2026-07-07: Keep macOS and Windows load planning session-available; their detailed runtime session checks remain in platform supervisors because those probes require OS-specific command/provider state.
