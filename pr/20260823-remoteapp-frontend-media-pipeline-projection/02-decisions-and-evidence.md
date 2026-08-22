# Decisions and evidence

## Decision

Add a typed `RemoteDesktopMediaPipelineSupport` projection to the frontend
protocol model and render it in RemoteApp session details.

## Evidence target

- frontend protocol test pins the daemon field mapping;
- frontend UI test pins the visible session-details label;
- EasyNet-Cli frontend product-flow checker requires both.
