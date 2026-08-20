# Decisions log

- 2026-07-27: Treat stale local state as an operator-controlled purge concern,
  not as an invocation resolver compatibility concern.
- 2026-07-27: Use one root purge boundary (`config::state_dir()`) instead of
  enumerating keyring/descriptor/read-model filenames; filename inventories
  become legacy cleanup tables as soon as state ownership changes.
- 2026-07-27: Keep `--force` as the explicit override for active runtime
  detection. Tests that validate destructive purge semantics use `--force`
  because a developer machine may already have a daemon running.
