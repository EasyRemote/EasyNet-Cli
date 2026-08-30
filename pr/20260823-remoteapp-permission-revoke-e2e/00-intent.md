# RemoteApp permission revoke E2E intent

Date: 2026-08-23

## Problem

RemoteApp already has daemon/session logic for `target_permission_revoked`, but
product evidence was still mostly source-level: lifecycle tests and frontend
terminal-sync tests proved the intended state transition, not a host-level
public ability path under real platform permission revocation.

## Intent

Add a host-side RemoteApp permission-revoke E2E harness that:

- creates a real RemoteApp session through public CLI/daemon abilities;
- requires a live Resource URA selected through `resource.refresh_remote_targets`;
- waits for a real platform/operator permission revoke instead of simulating
  revoke through a debug path;
- observes terminal state through public `remote_desktop.show_session`;
- validates `target_permission_revoked`, revoked consent projection,
  `TARGET_PERMISSION_REVOKED` event ordering, and terminal receipt binding.

## Non-goals

- Do not add a production debug ability that can fake OS permission revocation.
- Do not claim a completed real OS revoke report until the live harness has
  actually passed on a host.
- Do not prove reconnect, crash/restart recovery, cross-device revoke, input
  injection, or NAT/WebRTC fallback.
