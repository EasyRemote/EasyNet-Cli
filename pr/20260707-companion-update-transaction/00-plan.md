# Companion Update Transaction Plan

## Goal

Close the desktop companion plugin update gap by making `plugin update` use the same package and supervisor transaction boundary as install.

## Scope

- Add companion-aware update entrypoint to `PluginInstaller`.
- Preserve package tree and plugin lock rollback when companion supervisor update fails.
- Restore prior companion supervisor state after failed update rollback.
- Route CLI `plugin update` through the companion-aware installer path.

## Non-goals

- No SDK surface changes.
- No Axon protocol changes.
- No shell fallback path.
- No alternate lifecycle model outside `DesktopCompanionManager`.
