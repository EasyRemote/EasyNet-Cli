# Intent

Goal: make desktop companion platform artifacts version-addressable so update and remove cannot accidentally mutate another installed version's launcher state.

Non-goals:
- Do not change the desktop companion SPEC direction.
- Do not introduce legacy fallback install paths.
- Do not change public CLI commands.

Acceptance criteria:
- macOS app bundles install under a path containing package id and package version.
- Windows tray artifacts install under a path containing package id and package version.
- macOS LaunchAgent removal only removes a plist that points at the current plan executable.
- Windows startup entry removal only removes a Run entry that points at the current plan executable.
- Tests prove versioned artifact paths and launcher target checks.
