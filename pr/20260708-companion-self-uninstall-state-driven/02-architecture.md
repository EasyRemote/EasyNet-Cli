# Architecture

`DesktopCompanionManager` owns self-uninstall companion cleanup because it owns
the desired-state store and the supervisor boundary. `selfcmd` delegates to the
manager instead of open-coding package iteration.

The manager treats `PluginPackageIndex` as a plan lookup table, not as the
enumeration source. This preserves the SPEC distinction between indexed package
state and companion desired state.
