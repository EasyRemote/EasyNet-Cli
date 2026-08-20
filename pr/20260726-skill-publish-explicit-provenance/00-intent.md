# Intent

## Goal

Remove the hidden `mission.think` provenance fallback from `skill.publish`.

`skill.publish` must preserve its public request contract, including optional
`mission_run_id`, while writing an explicit install provenance state:

- curator publication when `mission_run_id` is supplied;
- direct runtime publication when no curator run is supplied.

## Non-goals

- Do not change the public `skill.publish` request or response shape.
- Do not change `skill.install` GitHub/source installation semantics.
- Do not migrate unrelated Mission, browser, or invocation-history behavior in
  this task.

## Acceptance criteria

- A missing `mission_run_id` no longer serializes as `source.kind=curator` with
  `source.identifier=mission.think`.
- Curator-authored calls with `mission_run_id` still serialize as curator
  provenance bound to the supplied run id.
- Direct publish provenance is generic runtime provenance, not EasyNet,
  EasyRemote, or Mission lifecycle naming.
- Tests prove both provenance states.
- Architecture gates reject restoration of the hidden fallback.
