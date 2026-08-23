# RemoteApp frontend observed lifecycle artifact contract

## Product gap

The Browser/Tauri lifecycle verifier requires the correct lifecycle steps, but
an artifact could still be assembled from component snapshots or mocked state
without proving that a real UI automation runner observed each step in order.

For product readiness, the frontend evidence must show the user-visible flow was
driven through the actual browser/Tauri surface, not reconstructed after the
fact from store state.

## Boundary decision

- The verifier validates evidence from a real Browser/Tauri runner; it does not
  simulate UI actions.
- The runner owns observing each lifecycle step through browser/Tauri
  automation.
- The artifact must record a source and monotonic timestamp for every lifecycle
  step.

## Invariants

1. Every step must carry `evidence_source=browser_automation` or
   `evidence_source=tauri_automation`.
2. Every step must carry positive `observed_at_ms`.
3. `observed_at_ms` must be strictly increasing in lifecycle order.
4. `component_snapshot_only` must not be true for any step.
5. Existing ability, WebRTC, media, input policy, and terminal receipt checks
   remain required.

## Verification checklist

- `bash -n tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh` —
  passed
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
  >/dev/null` — passed
- `bash tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh
  --self-test` — passed
- negative `--run --evidence-json` fixture without step evidence source must
  fail — failed as expected
- negative `--run --evidence-json` fixture with non-monotonic step timestamps
  must fail — failed as expected
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` — passed
  after correcting the mutation replacement to remove the actual automation
  source token
- `git diff --check` — passed
