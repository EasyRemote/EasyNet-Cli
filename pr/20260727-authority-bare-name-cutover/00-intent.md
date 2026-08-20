# Authority bare-name cutover

## Goal

Remove the descriptor-wire path that accepts Authority-owned `hub.*` bare
ability names and projects them through legacy Hub-style owner-local naming.

## Non-goals

- Do not remove daemon-local bare dispatch names for Device-owned abilities.
- Do not remove Agent-owned local registry projection.
- Do not change public descriptor-ref validation for already canonical Ability
  URAs or descriptor refs.
- Do not stage unrelated `docs/spec/*` worktree changes.

## Acceptance criteria

- Authority callees no longer strip or accept `hub.*` as an alias for
  Authority-owned abilities.
- `hub.*` Authority bare-name projection is rejected before constructing an
  Ability URA.
- SPEC v2 gate prevents reintroducing Authority bare-name projection.
