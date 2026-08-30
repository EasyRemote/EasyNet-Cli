# Architecture

The plugin capture adapter owns native window capture and composition. The
session-owned `RemoteAppTargetBinding` remains the single identity/lifecycle
source. Runtime Core is unchanged.

The xcap adapter composes a committed process window set in virtual-desktop
coordinates. ScreenCaptureKit continues to use one display-relative app filter;
cross-display macOS support requires a future plugin-owned `MultiAppSurface`
source that produces one canonical video surface without widening to display
capture.
