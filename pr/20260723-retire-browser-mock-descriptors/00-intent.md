# Intent

## Goal

Remove the retired browser mock ability descriptors from the active system
descriptor inventory. The runtime must not publish `browser.*` as
`cutover_ready` when no executable LocalRuntime handler exists.

## Non-goals

- Do not implement WebView/browser runtime functionality in this slice.
- Do not preserve placeholder descriptors as compatibility surface.
- Do not add route fallbacks or synthetic handlers.
- Do not change the browser product roadmap; this only removes the retired mock
  surface from canonical runtime publication.

## Acceptance Criteria

- `ability-descriptors/system/device_control/browser.*.ability.toml` are removed.
- Active convergence gates reject browser mock vocabulary in descriptors and
  runtime source.
- No production source advertises `browser.open_session`,
  `browser.capture_viewport`, `browser.send_input`, `browser.close_session`, or
  `browser.attach_session`.
- Existing gates and formatting remain green.
