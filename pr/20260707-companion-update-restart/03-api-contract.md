# API Contract

Internal contract:
- `commit_package_update` returns action `restart` when the previous observed companion state was running-like.
- Running-like states remain `running`, `starting`, and `stale`.
- Restart sequence is `stop` then `start` after install, supervisor enablement, and desired-state persistence.

Error behavior:
- Any stop/start error bubbles to `PluginInstaller`, which restores previous package and supervisor state through existing rollback.
