# Access resource bootstrap completion

## Objective

Close the remaining Access seam where first entry can show no mic/camera resources until another action causes resource projection to refresh.

## Boundary proof

- `meta.list_resources` remains a read-only cache projection.
- Boot-time media seeding must use the daemon's verified runtime identity, not a second credentials read that can diverge from the already-loaded daemon config/profile.
- `resource.refresh_remote_targets` remains the explicit mutable live target path for display/window/application inventory.

## Verification

- Add a daemon unit test pinning media resource owner derivation to `Ready` identity.
- Re-run focused Rust tests plus build.
- Re-run browser first-entry Access verification when runtime/frontend are available.
