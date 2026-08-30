# Decisions and evidence

## Decision

Add `requires_multi_window_scenarios` to
`tools/scripts/remoteapp-product-completion-e2e.sh`.

Extend `tools/scripts/remoteapp-multi-window-tracking-e2e.sh` report summaries
with the minimum product-completion fields needed by the aggregate gate:

- `scenario`
- `status`
- `session_id`
- `selected_resource_ura`
- `frames_rendered`
- `events`
- independent stream distinct counts and sentinel-leak flags
- geometry revision counts
- application rebind and display-fallback flags
- target-loss rebind outcome
- multi-display `MultiAppSurface` state

The full artifact validation remains in the dedicated multi-window verifier.

## Verification plan

- `bash -n tools/scripts/remoteapp-multi-window-tracking-e2e.sh tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/remoteapp-multi-window-tracking-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check -- tools/scripts/remoteapp-multi-window-tracking-e2e.sh tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/00-intent.md pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/01-invariants.md pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/02-decisions-and-evidence.md`

## Verification results

- `bash -n tools/scripts/remoteapp-multi-window-tracking-e2e.sh tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `git diff --check -- tools/scripts/remoteapp-multi-window-tracking-e2e.sh tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/00-intent.md pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/01-invariants.md pr/20260823-remoteapp-multi-window-aggregate-scenario-gate/02-decisions-and-evidence.md`
- `bash tools/scripts/remoteapp-multi-window-tracking-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_multi_window_tracking_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
