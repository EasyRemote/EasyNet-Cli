# CLI compatibility coordinate

`axon.lock.json` is the sole pinned Axon coordinate consumed by CLI admission.
It binds an exact Axon commit and contract digest to Runtime and SDK dependency
projections.

Use `python3 tools/scripts/release-coordinate.py --check|--commit|--push` to
derive the lock from a clean exact Axon checkout. The transaction operates in
isolated worktrees and admits only Runtime manifests, dependency declarations,
generated locks, and the compatibility lock into the metadata commit.

The Runtime, Python SDK, and private Node seam remain independently versioned.
Local Go development uses root `go.work`; release-shaped `sdk/go/go.mod` keeps
the registry version and contains no sibling replacement.
