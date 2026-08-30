# Intent

## Goal

Continue RemoteApp product closure from current source and external runtime
evidence without treating ABI v9, unit tests, source gates, or historical `/tmp`
artifacts as proof of the interactive product.

The completion scope remains:

1. exact application/window selection and stable native capture on macOS,
   Windows, and Linux;
2. permission-scoped, low-latency pointer and keyboard OS effects;
3. bounded H.264/Opus media with negotiated rate/adaptation/drop behavior;
4. real multi-window and multi-application tracking and rebind;
5. deterministic disconnect/resume/revoke/cancel/timeout closure;
6. direct, STUN, TURN, and EasyNet relay network paths;
7. complete browser discovery, consent, session, render, control, recovery, and
   end-session UX; and
8. cross-device Browser-to-Hub-to-daemon-to-native-host regression evidence.

## Current truth

`docs/design/remoteapp-product-readiness-matrix.json` reports all eight product
requirements as `partial` and `product_complete=false`. This plan does not
weaken those requirements.

## Non-goals

- Do not route RemoteApp media through generic FFI v9.
- Do not turn a Device, plugin, helper process, or user account into an Agent.
- Do not replace live platform/network evidence with cross-compilation or
  synthetic fixtures.
- Do not stage or commit unrelated concurrent worktree changes.

## 2026-08-30 macOS production-flow incident

Restore the concrete browser Remote Desktop flow after a live macOS session
reached connected ICE/PeerConnection state but the installed media-host failed
VideoToolbox initialization with `kVTPropertyNotSupportedErr` (`-12900`). The
source fix must be proven against the physical encoder, installed through the
canonical developer deployment, and exercised through the browser product
flow. A closed session whose media process never initialized is not a working
RemoteApp result.
