# RemoteApp input evidence contract hardening

## Intent

Close a verifier seam in the RemoteApp input-injection product gate. The daemon
now emits stable `input_event_id` values for `INPUT_FRAME_APPLIED`, but the live
input verifier still accepted any non-empty string. That allowed evidence to
look product-complete without proving that the OS-effect observation was bound
to a daemon-applied input event.

## Architecture boundary

- This is an E2E evidence contract change, not a protocol change.
- `input_event_id` remains daemon-local RemoteApp execution evidence; it is not
  an Axon Invocation id or receipt id.
- The verifier consumes public session events emitted by the RemoteApp plugin
  and rejects evidence that does not preserve their execution identity fields.

## Invariants

1. Applied input results must carry `event_type=INPUT_FRAME_APPLIED`.
2. Applied input results must carry a daemon-shaped `input_event_id`:
   `rdinp1_<32 lowercase hex>`.
3. Applied input results must expose the session transport generation
   (`transport_epoch`) and daemon applied counter (`accepted_count`).
4. OS-effect observations must bind the same `input_event_id`, selected
   Resource URA, session id, geometry revision, and focus epoch.
5. The verifier must still keep `product_complete_claim=false`; this hardens
   evidence quality and does not replace a live runner.

## Verification

Commands to run before commit:

```bash
bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test
bash tests/scripts/test_remoteapp_input_injection_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
git diff --check
```

## Results

- `bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test` — passed.
- `bash tests/scripts/test_remoteapp_input_injection_e2e.sh` — passed.
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed.
- `git diff --check` / `git diff --cached --check` — passed.

## Non-claims

This does not create the live host input runner and does not prove successful
OS input effects. It hardens the evidence contract so that a future live
artifact must be bound to daemon-applied `INPUT_FRAME_APPLIED` events.
