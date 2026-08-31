# Verification

## Required matrix

- Bundle inspection finds the exact `Info.plist` identifier and executable.
- Signing verification proves stable identifier, Team ID, hardened runtime,
  and no cdhash-only designated requirement.
- Release package contract ships `.app` on macOS and flat binaries elsewhere.
- Physical permission probe returns `granted=true` after user grant.
- Browser lifecycle proves create, connected media with frame growth, end, and
  terminal receipt.

## Baseline evidence

- Flat helper request: `requested=true`, `granted=false`.
- System Settings file picker: exact Unix executable selected, Open disabled.
- Developer-ID signing produced a stable DR but still no application entry,
  disproving signing alone.
- Direct execution of the signed app's inner binary still returned
  `granted=false`, proving LaunchServices application context is required.
- LaunchServices permission probe returned `granted=true` for the exact target
  bundle.
- Browser evidence `rdp-7c940cc581c5fd65d4721d77` reached connected ICE and
  WebRTC, presented 37 frames during the sustained window, and ended with
  terminal reason `caller_ended`.
