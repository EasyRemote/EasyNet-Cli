# Companion Windows Provider

## Goal

Move the Windows desktop companion supervisor from seam/no-op behavior to provider-backed install, enable, start, stop, and process observation semantics required by the desktop companion plugin SPEC.

## Boundary

- The Windows supervisor owns Windows user-session integration and process control.
- The companion manager owns lifecycle orchestration and DTO projection.
- SDKs and CLI must consume projected status/action DTOs; they must not shell out to Windows tools.

## Invariants

- Windows enablement points at the installed app path under the user's EasyNet app directory, not the mutable package source path.
- Windows status prefers a status-file heartbeat and falls back to process observation.
- Windows stop uses heartbeat pid when available and image-name termination otherwise.
- Non-Windows builds keep unsupported-platform behavior and do not report false success.

## Verification

- Unit tests cover Windows task-list parsing and installed app path calculation.
- Focused companion tests must pass on the host platform.
- Static checks and touched-file terminology audit must pass.

## Results

- `cargo test -q daemon::plugins::companion` passed with 15 focused tests.
- `cargo test -q plugin_host_install` passed.
- `cargo test -q plugin_host_update` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.
