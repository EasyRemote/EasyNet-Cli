# Companion Update Transaction Invariants

1. Package tree movement and plugin state files remain owned by `PluginInstaller`.
2. Companion supervisor lifecycle remains owned by `DesktopCompanionManager`.
3. A failed companion supervisor update cannot leave the new package lock active.
4. A failed companion supervisor update attempts to restore the previous package's companion artifacts and desired state.
5. Update preserves the previous companion desired state instead of forcing enablement.
6. A companion that was running before update is restarted after a successful update.
7. No remote control-plane exposure is added.
