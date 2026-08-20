# Decisions Log

- 2026-07-07: Treat `status_file` path resolution as planner-owned runtime state, not platform adapter logic.
- 2026-07-07: Resolve manifest paths under `state/` and `companions/` against the local EasyNet state root; resolve other relative paths against the installed package root.
- 2026-07-07: Align the first-party desktop menubar manifest with the companion process contract path `companions/easynet.desktop.menubar/status.json`.
