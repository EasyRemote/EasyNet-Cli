# RemoteApp Application Window-Set Freeze

## Problem

Live application decoded-frame E2E can select the correct application Resource
URA, create a valid AppSurface session, then decode black frames because the
ScreenCaptureKit application selector expands from the committed inventory
window set to every same-application window visible to SCK.

## Invariants

- `RemoteAppTargetBinding` remains the capture boundary.
- An application `AppSurface` captures the committed display-scoped
  `AppWindowSetProof`, not an open-ended application subscription.
- Extra same-process windows do not widen the active capture scope.
- Missing committed windows fail closed with a typed target-domain error.
- Window/application capture must not fall back to display baseline capture.

## Implementation

- Expose read-only `AppWindowSetProof` membership helpers.
- Make the macOS ScreenCaptureKit application selector include only committed
  `resolved_window_ids`.
- Reject missing committed window ids before constructing the content filter.
- Extend static RemoteApp E2E boundary checks so future changes cannot return
  to unbounded application window inclusion.

## Verification

- `cargo fmt -p easynet -- --check`
- RemoteApp static gates.
- RemoteApp script check integration.
- Targeted RemoteApp Rust tests.
- Host decoded-frame E2E for window and application when the local daemon/TCC
  environment is available.
