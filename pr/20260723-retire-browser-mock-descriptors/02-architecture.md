# Architecture

## Boundary

The active ability descriptor inventory is part of the daemon's public runtime
surface. It must match executable LocalRuntime capabilities.

## Removed Legacy Surface

The retired browser mock surface consisted of descriptor files for:

- `browser.open_session`
- `browser.capture_viewport`
- `browser.send_input`
- `browser.close_session`
- `browser.attach_session`

These descriptors described placeholder/mock behavior and claimed
`cutover_ready` capability state without a production handler.

## Clean Target

Browser/WebView functionality can return later only as a provider-backed or
cutover-ready capability with:

- an explicit session lifecycle state machine;
- real executable handlers for each published call mode;
- deterministic terminal receipt semantics; and
- product-level e2e coverage.
