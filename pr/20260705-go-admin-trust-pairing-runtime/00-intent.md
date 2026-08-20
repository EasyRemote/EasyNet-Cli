# Intent

Implement Go Admin + Gateway Runtime Core execution for Hub membership and
pairing/credential lifecycle operations.

## Goal

- Replace Go `AdminRuntimeTransport` hub join/leave and pairing/credential
  `NOT_IMPLEMENTED` seams with complete Invocation dispatch through
  `RuntimeClient`.
- Keep descriptor-ref construction delegated to `IdentityClient`.
- Project daemon results into the existing `JoinResult`, `LeaveResult`,
  `PairingPreflight`, `PairingToken`, `DeviceCredential`, and
  `DeviceCredentialVerification` DTOs.

## Non-Goals

- Do not implement certificate policy, token generation, trust-store writes, or
  backend account binding in Go.
- Do not export new C ABI admin symbols in this slice.
- Do not change the daemon SDK spec.
