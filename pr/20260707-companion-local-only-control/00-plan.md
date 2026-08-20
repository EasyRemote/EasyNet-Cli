# Companion Local-Only Control Plan

## Goal

Close the SPEC requirement that `plugin.companion_status` and
`plugin.companion_reconcile` are daemon-local control surfaces, not remotely
advertised or remotely routable product abilities.

## Scope

- Keep the local daemon registry handlers intact.
- Exclude companion control names from public catalogue publication.
- Exclude companion control names from device-profile descriptor generation.
- Prevent LocalRuntime authority from proving a remote route for daemon-local
  companion control names.

## Non-Goals

- No companion DTO changes.
- No SDK surface changes.
- No remote AuthorityBinding for companion control.
