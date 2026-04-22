# Open Question — Does EasyNet need a worktree ability?

**Status:** Open · **No revisit trigger yet** · **Owner:** Silan Hu · **Date:** 2026-04-22

## Why this is an open question, not a plan item

"worktree" appeared in earlier drafts of the CLI plan under 不排期堆积, alongside `terminal` and `pairing`. Same evidentiary situation: I cannot now produce a grounded reason for it.

Plausible (but un-grounded) motivations:

- An agent that spawns multiple concurrent tasks wants each to have its own `git worktree` so the file-system snapshots don't stomp on each other.
- A mission that branches wants its branches to isolate disk state.
- The CLI grows a `easynet worktree create / list / remove` verb group to manage the above.

None of the above has a documented consumer today. The word "worktree" appears in the CLI plan purely as a bullet item.

## What would move this to a plan item

- A concrete mission that deadlocks on shared filesystem state, with `git worktree` as the evidence-based fix. The mission's trace would be the source document.
- A PR-10 EAL control-flow RFC follow-up specifying how `loop { body }` iterations interact with file-system state — where isolating per-iteration worktrees is the cleanest answer.
- An Alive or Frontend ask for parallel agent execution where each parallel branch needs its own rootfs view.

## If it becomes a plan item

Worktree creation is a `git worktree add` wrapper. The actual research is: where does the worktree live on disk, who cleans it up, what's the policy when a mission crashes mid-iteration and leaves a dirty worktree? None of that is derivable without the mission whose requirement motivates it.

## Log

| Date       | Event                                                                       |
|------------|-----------------------------------------------------------------------------|
| 2026-04-22 | Extracted from the CLI plan's "不排期堆积" bucket after a ground-first audit found no customer for the item. |
