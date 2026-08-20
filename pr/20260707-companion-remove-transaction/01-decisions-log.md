# Decisions Log

## Installer-Owned Removal

The previous CLI path called the companion manager before `PluginInstaller::remove`. That split ownership across two layers and could disable supervisor state before package state had a rollback owner. Removal now enters through `PluginInstaller::remove_with_companion_manager` so package and companion cleanup are sequenced by one transaction boundary.

## Rollback Allocation Before State Mutation

The package removal transaction now allocates its rollback path before editing `state/plugins.toml`. This closes a root transaction bug where rollback-path allocation failure could leave the active package directory present but the lock row removed.
