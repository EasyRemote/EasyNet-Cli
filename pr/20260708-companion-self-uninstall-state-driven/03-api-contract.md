# API Contract

- `cleanup_for_self_uninstall(packages)` reads all companion desired-state
  records.
- For a matching desktop companion package, it calls `remove(package)`.
- For a missing package, it removes `~/.easynet/companions/<id>/` and the state
  row, then returns a warning.
- The method returns warnings instead of aborting the rest of self uninstall.
