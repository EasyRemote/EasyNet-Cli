# Intent — RemoteApp Platform Support Matrix

RemoteApp needs a product-visible platform support matrix for
macOS/Linux/Windows capture targets. Current runtime capability projection
already separates production and diagnostic subjects, but it does not explicitly
state which platform/target combinations are product-ready, diagnostic-only, or
unsupported.

This batch adds that matrix to the daemon device capability view and pins it
with boundary tests. It does not implement new Windows/Linux capture.
