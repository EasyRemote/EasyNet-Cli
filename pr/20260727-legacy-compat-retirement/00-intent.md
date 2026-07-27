# Intent

## Goal

Continue canonical runtime convergence by finding and retiring active legacy or compatibility paths that preserve a second runtime, admission, receipt, or public ingress model.

## Non-goals

- Do not remove negative tests that prove legacy inputs fail closed.
- Do not remove product HTTP compatibility surfaces such as OpenAI-compatible endpoints when they are explicitly product-owned and outside the canonical SDK/runtime model.
- Do not change user-owned documentation currently modified in the worktree.

## Acceptance Criteria

- The selected slice removes or narrows an active legacy path rather than adding another adapter.
- Public behavior remains compatible at the versioned edge, but canonical runtime internals have one owner.
- The canonical runtime convergence gate and focused tests pass.
- Any stable implementation is committed with `Silan.Hu <silan.hu@u.nus.edu>`.
