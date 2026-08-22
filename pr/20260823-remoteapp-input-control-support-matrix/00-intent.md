# Intent — RemoteApp Input Control Support Matrix

RemoteApp device capabilities expose a global `input_injection` bool and
session-level `input_readiness`, but the product view still lacks a stable
platform/target support matrix for pointer and keyboard control.

This batch adds that matrix so frontend UI and E2E harnesses can distinguish
display-global macOS input from unsupported window/application and non-macOS
input-control paths without inferring from broad plugin permissions.
