# Decisions log

## 2026-08-24

- Extend the existing decoded-frame receiver so media and input share one real
  WebRTC transport instead of building a second signaling implementation.
- Keep input proof opt-in so view-only decoded-frame E2E behavior remains
  unchanged.
- Observe AppKit callbacks in the target process rather than reading cursor
  position or generating events from the fixture; the observer must be
  independent from the daemon injector.
- Use unique, bounded pointer and keyboard actions and verify the unrelated
  process did not receive them.
- Treat `remote_desktop.set_description`, not diagnostic
  `remote_desktop.attach`, as the signaling/attachment proof for the production
  WebRTC input path.
- Preserve a permission-gated live run as a blocker artifact and keep readiness
  partial; reaching the real permission boundary is not an input-effect pass.
