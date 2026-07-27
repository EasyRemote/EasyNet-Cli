# Intent

## Goal

Continue canonical runtime convergence by deleting an active legacy or
compatibility surface that keeps product-specific or duplicate runtime semantics
inside the SDK/runtime implementation.

## Non-goals

- Do not remove product-owned HTTP compatibility APIs.
- Do not remove negative tests that prove retired inputs fail closed.
- Do not alter user-owned `docs/spec` worktree changes.

## Acceptance Criteria

- The selected change removes or narrows an active compatibility surface.
- Callers migrate to the canonical runtime abstraction without adding a new
  fallback.
- Focused tests and architecture gates pass.
- Stable work is committed with `Silan.Hu <silan.hu@u.nus.edu>`.
