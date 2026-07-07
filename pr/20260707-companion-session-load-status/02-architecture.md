# Architecture

Layering:
- `companion/session.rs` owns platform session availability checks.
- `PluginLoadPlanner` uses the session probe only for load-state classification.
- `LinuxDesktopCompanionSupervisor` delegates graphical-session detection to the same probe.
- macOS and Windows keep their existing supervisor-specific runtime checks; load planning treats them as session-available because their first implementation requires OS-level probes.

Boundary proof:
- This is EasyNet-Cli daemon/plugin runtime state. It does not affect Axon SDKs or protocol invocation semantics.
