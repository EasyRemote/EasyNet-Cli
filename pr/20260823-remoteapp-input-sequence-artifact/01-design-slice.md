# RemoteApp input sequence artifact contract

## Product gap

The input injection verifier requires pointer/keyboard applied events, latency,
focus, permission, and observed OS effects. That proves successful input only
for the happy path. It does not prove that replayed or out-of-order data-channel
frames are rejected before host input injection.

For an interactive remote desktop product, input correctness is not just
whether one pointer and one key event can be applied. The execution path must
also prove client ordering and replay safety, otherwise a stale frame can
duplicate or reorder control on the host.

## Boundary decision

- The verifier validates evidence from a real host input runner; it does not
  synthesize OS input or replace daemon input policy.
- The daemon/plugin execution path owns sequence rejection.
- The artifact must report both accepted applied frames and a stale/replayed
  frame rejection observed through the same public RemoteApp session channel.

## Invariants

1. Passed platforms must list applied `input_results` in strictly increasing
   `client_sequence` order.
2. Applied frames must carry `host_received_at_ms`, `host_applied_at_ms`, and
   `latency_ms` so receive/apply timing is auditable.
3. Passed platforms must include at least one rejected frame with
   `event_type=INPUT_FRAME_REJECTED`, `reason=stale_client_sequence`, and a
   `client_sequence` not greater than the max applied sequence.
4. Rejected stale frames must not carry `host_applied_at_ms`.
5. Unsupported Windows/Linux states remain explicit product unsupported states
   and must not report applied or rejected input effects.

## Verification checklist

- `bash -n tools/scripts/remoteapp-input-injection-e2e.sh` — passed
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
  >/dev/null` — passed
- `bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test` — passed
- negative `--run --evidence-json` fixture without stale rejection evidence
  — failed as expected
- negative `--run --evidence-json` fixture with non-monotonic applied
  `client_sequence` — failed as expected
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` — passed
- `git diff --check` — passed
