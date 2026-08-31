# API contract

- `--check`, `--commit`, and `--push` have the same semantics as Axon's release-coordinate command.
- `--version VERSION` freezes the Runtime coordinate.
- `--axon-root PATH` selects the Axon Git repository; its selected HEAD is copied into a clean isolated worktree.
- The synchronizer updates released dependency versions while retaining local development through repository-owned workspace/source seams.
