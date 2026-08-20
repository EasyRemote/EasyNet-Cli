# API Contract

No public JSON response shape changes.

Behavioral tightening:

- `plugin list --format table`, `plugin status`, and `plugin activate-realtime --format table` now fail on malformed daemon plugin-control response fields instead of rendering placeholder defaults.
- Optional companion state still renders `-` when the package has no companion projection.

