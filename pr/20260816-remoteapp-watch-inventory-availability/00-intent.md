# Remoteapp Watch Inventory Availability Intent

## Goal

Close the target picker inventory seam where `resource.watch_remote_targets` can represent a host-local target discovery outage as if all previously visible targets were removed.

## Root problem

`resource.watch_remote_targets` is an inventory stream, not a session lifecycle stream. A transient inventory source outage and a real target removal have different frontend recovery semantics. Collapsing both into `removed_resource_uras` makes the picker treat stale targets as definitively gone and obscures the SPEC requirement that stale rows are either unavailable or withheld, not definitely capturable.

## Scope

- Keep inventory ownership in the daemon resource layer.
- Preserve `resource.refresh_remote_targets` as the mutable live refresh ability.
- Add typed watch-stream evidence for unavailable inventory observations.
- Ensure unavailable observations do not fabricate target removal events.
- Keep remote desktop plugin/session code out of inventory refresh semantics.

## Non-goals

- No frontend file edits in this turn because the EasyNet frontend/backend worktree is already dirty outside this task.
- No remote desktop session lifecycle changes.
- No change to `meta.list_resources` read-only cache behavior.
