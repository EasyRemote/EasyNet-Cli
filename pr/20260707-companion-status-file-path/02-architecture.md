# Architecture

Layering:
- Manifest validation proves the declared path is relative and required for `status_file` health.
- Companion path resolution is a planner concern because it combines package metadata, package root, and local state root.
- `DesktopCompanionPlan` carries the resolved path as the contract consumed by platform supervisors.
- macOS and Windows adapters own LaunchAgent/startup/process mechanics only.

Boundary proof:
- This remains inside the EasyNet-Cli daemon/plugin boundary.
- SDK and FFI consumers see the same DTO state; they do not classify status-file health.
