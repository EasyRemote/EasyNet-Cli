# Intent

## Goal

Implement `docs/design/desktop-companion-plugin-spec.md` as production EasyNet-Cli desktop companion plugin infrastructure.

## Non-Goals

- Do not move desktop companion lifecycle into Axon.
- Do not make desktop companions fake ability providers.
- Do not require graphical desktop availability for daemon boot.
- Do not preserve script-owned install behavior as the canonical path.

## Acceptance Criteria

- `desktop_companion` packages parse with zero abilities and typed companion metadata.
- Plugin list, plugin status, runtime status, daemon-local control abilities, FFI, and SDK projections share one companion DTO contract.
- Package install/update/remove preserve package lock consistency with supervisor install state.
- Daemon post-Ready companion startup is best-effort and non-fatal.
- Runtime stop honors `stop_policy`, with `keep_running` as the EasyNet Menu Bar default.
- Declared companion artifacts are covered by package hashing.
