# API contract

No new Ability or Invocation shape is introduced.

The local CLI adds `create-remote-desktop-session --input-control`; omission
continues to mint a consent ticket with `input_control=false`. The flag changes
the `grant_consent` request only and is never copied into `create_session`
arguments.

For an interactive macOS window/application session with `input_control=true`:

- `target_binding.input_scope = "target_local"`
- `input_readiness.effective_mode = "interactive"` only while target and
  Accessibility gates are ready
- pointer frames require the current `target_geometry_revision`
- pointer and key frames require the current `target_focus_epoch`

Host validation failures are emitted through the existing
`INPUT_FRAME_REJECTED` path with a specific `target_input_guard_*` reason and
must never be reported as applied input.

Sampled successful `INPUT_FRAME_APPLIED` events carry
`target_guard_validation` for target-local input, including selected Resource
URA, session id, target kind, snapshot-start and validation timestamps,
geometry/focus epochs, and exact window or application window-set proof. This
is execution evidence, not new input authority.
