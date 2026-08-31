# Architecture

The release transaction composes three owners:

- Runtime version synchronizer: root `VERSION`, Runtime Cargo manifests/locks, and Runtime feature projection.
- Axon dependency synchronizer: Python/Go dependency declarations and canonical `axon.lock.json`.
- Release transaction: clean-tree preflight, temporary CLI/Axon worktrees, allowed-path admission, metadata commit, fast-forward, and optional push.

State progression is `Preflight -> UpstreamPinned -> Isolated -> Synchronized -> Verified -> Committed -> Pushed`.
