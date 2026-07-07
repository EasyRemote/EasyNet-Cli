# Invariants

1. Desktop companion lifecycle is EasyNet-Cli daemon/plugin product state, not Axon Invocation state.
2. A `desktop_companion` package must declare `[companion]`; other plugin kinds must not.
3. `abilities` and `ability_metadata` are optional only for `desktop_companion`.
4. Desired, supervisor, and observed state remain separate inputs to the projected state machine.
5. Observed status comes from heartbeat/process observation; LaunchAgent, Run key, or user unit state is not canonical runtime truth.
6. Daemon boot readiness must not depend on GUI session availability or companion start success.
7. User-level launchers only: macOS LaunchAgent, Windows HKCU Run, Linux user-session classification.
8. Package hashes must include every declared companion executable artifact through the hashed `bin/` or `dist/` package surface.
9. CLI, control ability, FFI, and SDK wrappers must consume shared DTOs rather than reclassifying state.
10. URA terminology remains the only addressing vocabulary.
