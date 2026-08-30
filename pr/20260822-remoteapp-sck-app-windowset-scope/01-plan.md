# RemoteApp ScreenCaptureKit application window-set scope

## Invariant

An application RemoteApp session captures the committed display-scoped
`AppWindowSetProof`. It must not widen to every same-application window visible
to ScreenCaptureKit after session creation.

## Change

- Keep application capture on ScreenCaptureKit's application filter.
- Build `exceptingWindows` from same-application, same-display windows that are
  not in the committed `AppWindowSetProof`.
- Continue failing closed when committed windows disappear or span unsupported
  displays.
- Extend the target-binding boundary gate and source test to reject an empty
  `exceptingWindows` application filter.

## Product effect

This directly reduces the risk that selecting one application leaks unrelated
new windows from the same app. It does not complete cross-platform capture or
multi-display `MultiAppSurface` support.
