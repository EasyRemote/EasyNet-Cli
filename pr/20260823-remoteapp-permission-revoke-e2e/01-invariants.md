# RemoteApp permission revoke E2E invariants

## Boundary invariants

- Permission-revoke proof must be observed through public
  `remote_desktop.show_session`, not by calling internal Rust handlers.
- The target remains an Invocation `subject` Resource URA. Session args must not
  duplicate subject identity.
- RemoteDesktop plugin owns the product session lifecycle. Runtime core remains
  responsible only for canonical invocation/admission/receipt semantics.
- The live harness must require real platform/operator revoke and must not
  provide a fake revoke switch.

## Lifecycle invariants

- Revocation terminal reason is exactly `target_permission_revoked`.
- Session state must become `closed`.
- Session `consent_phase` and `consent.phase` must become `revoked`.
- Event ordering must preserve:
  `TARGET_PERMISSION_REVOKED` before `MEDIA_SOURCE_LOST` before
  `SESSION_CLOSED`.
- The terminal receipt must bind the created `session_id` and carry
  `reason_code=target_permission_revoked`.

## Evidence invariants

- Self-test validates only the harness evidence contract.
- Product evidence requires a live run where the platform actually revokes host
  screen/input permission and the daemon projects the terminal session view.
