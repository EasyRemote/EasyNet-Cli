# Execution Checklist

- [x] Inspect current worktree and SPEC v2.
- [x] Use codegraph and targeted search to identify active legacy compatibility surfaces.
- [x] Select one high-value path that can be removed safely.
- [x] Refactor the root abstraction and migrate callers.
- [x] Delete obsolete implementation.
- [x] Run focused tests and convergence gates.
- [ ] Commit a stable slice if verification passes.
