# Architecture

Layering:
- `DesktopCompanionManager` owns lifecycle sequencing.
- Platform supervisors only implement primitive stop/start operations.
- `PluginInstaller` remains the transaction owner and uses manager failure to trigger rollback.

Boundary proof:
- This is EasyNet-Cli daemon/plugin lifecycle behavior.
- No Axon SDK or invocation protocol semantics are changed.
