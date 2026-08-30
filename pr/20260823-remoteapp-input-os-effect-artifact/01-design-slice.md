# RemoteApp input OS-effect artifact gate

## Product seam

The existing input-injection verifier required `observed_effect`, but that was
too weak for product evidence: a runner could report a symbolic effect string
without proving that the host OS actually applied the pointer/key event to the
selected target after the RemoteApp input frame was applied.

## Slice

- Require every applied pointer/keyboard input result to include an `os_effect`
  object.
- Bind the observed OS effect to the same platform, session, selected Resource
  URA, and target geometry revision.
- Require the OS-effect observation timestamp to be after `host_applied_at_ms`.
- Require pointer effects to report expected/observed display-global position
  and bounded pixel tolerance.
- Require keyboard effects to report the focused Resource URA and observed key.

## Expected impact

This does not claim input injection is product-complete. It closes the verifier
seam where policy/runtime telemetry could be mistaken for a real host-side OS
input effect.
