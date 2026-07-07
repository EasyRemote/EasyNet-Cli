# Intent

Goal: converge desktop companion heartbeat observation on the SPEC-owned status-file contract.

Non-goals:
- Do not change the desktop companion SPEC direction.
- Do not move companion lifecycle into Axon SDK responsibilities.
- Do not keep adapter-local path construction when planner state can own it.

Acceptance criteria:
- Manifest-declared `status_file` becomes the runtime observation path in `DesktopCompanionPlan`.
- State-relative companion paths resolve under the local EasyNet state directory.
- Package-relative companion paths remain supported for package-owned heartbeat files.
- macOS and Windows supervisors consume the planned path instead of reconstructing it.
- The first-party desktop menubar package declares the section 16 status-file path.
