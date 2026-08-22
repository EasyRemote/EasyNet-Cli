# Intent — RemoteApp Platform Capability Claims

RemoteApp product readiness must not be inferred from broad manifest resources
or unavailable native backend descriptors.

This batch closes one product seam: device capability views must distinguish
runtime-available production target subjects from package-declared or future
platform capability. A Linux or non-permissioned macOS host must not project
`window` or `application` as production-ready simply because the package
contains the macOS ScreenCaptureKit descriptor.

The work remains bounded to the RemoteApp plugin capability projection and
product-closure gates. It does not implement Windows/Linux native capture.
