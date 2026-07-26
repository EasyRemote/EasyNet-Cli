# Intent

## Goal

Cut test and smoke fixtures over to canonical agent registry keys so publish
coverage no longer depends on retired bare agent-name registry rows.

## Non-goals

- Do not relax `save_agents` validation.
- Do not add load-time migration for bare agent names.
- Do not change public CLI arguments; users may still pass the short agent name
  where command APIs accept an agent selector.
- Do not modify product documentation files already dirty in the worktree.

## Acceptance criteria

- `ability.publish` and real invoke publish fixtures persist registry rows under
  canonical keys such as `default/alice`.
- CLI agent tests read and assert registry rows through canonical keys.
- Broad publish test filtering no longer fails because fixtures use retired
  registry keys.
- Architecture gates keep `save_agents` as the canonical persistence boundary.
