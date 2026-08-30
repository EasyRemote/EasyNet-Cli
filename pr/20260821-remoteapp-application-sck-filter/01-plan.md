# RemoteApp application ScreenCaptureKit filter

Date: 2026-08-21

## Problem

Host decoded-frame E2E passes for `window` targets but fails for `application`
targets: routing, subject binding, target binding, and scope audit are correct,
yet the decoded media frame does not contain the selected application sentinel.

## Boundary invariants

- Remote desktop owns session/media execution, not resource inventory.
- Resource inventory still supplies application/window subjects and stable target
  identity.
- Application capture must remain app-scoped: no full-display fallback, no scope
  widening, and no descriptor/authority bypass.
- ScreenCaptureKit filter construction is part of the native media implementation
  boundary; daemon/catalog architecture must not special-case it.

## Implementation

- Keep `WindowSurface` on `initWithDesktopIndependentWindow`.
- Change `AppSurface` from a display/window include filter to ScreenCaptureKit's
  application filter:
  `initWithDisplay_includingApplications_exceptingWindows`.
- Keep the committed display-scoped application window-set proof as the identity
  check before constructing the filter.
- Add a narrow source contract test so the application branch cannot regress to
  `initWithDisplay_includingWindows`.

## Expected effect

The application session should continue to report:

- `target_kind=application`
- `target_model=display_scoped_application_window_set`
- `scope_widened=false`
- `display_fallback_used=false`

and the decoded media frame should contain the selected application sentinel.
