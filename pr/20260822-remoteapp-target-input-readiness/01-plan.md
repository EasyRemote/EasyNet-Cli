# RemoteApp target-tracker input readiness

## Invariant

`input_readiness` is the daemon-owned product truth for whether RemoteApp
pointer/keyboard control is usable. It must not report interactive readiness
when the session target tracker has disabled input because the target is lost,
rebinding, hidden, minimized, permission-revoked, or otherwise not safe.

## Change

- Make session view `input_readiness.blocked_reason` report
  `target_input_not_ready` when `target_snapshot.input_enabled=false`.
- Keep target tracker loss ahead of OS accessibility checks in the readiness
  decision, because a missing/unsafe target must fail before local injection
  capability is considered.
- Gate the contract in `check-remoteapp-input-consent-boundary.sh` and its
  mutation self-test.

## Product effect

Frontend and operator-visible session state now matches the daemon input
execution path: target loss disables input readiness before any WebRTC
data-channel frame can be applied. This closes a state-projection seam; it does
not prove successful pointer/keyboard injection E2E.
