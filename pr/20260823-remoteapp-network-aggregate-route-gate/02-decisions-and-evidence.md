# Decisions and evidence

## Decision

Add `requires_network_route_scenarios` to the RemoteApp product-completion gate.
This keeps the aggregate gate small while closing the seam where a report with
the correct script name, `status=passed`, and `coverage=true` could still omit
the per-route proof summary needed for a product-complete network claim.

The aggregate gate checks only the summary fields emitted by the network
verifier:

- `route_kind`
- `selected_route_class`
- `ice_connection_state`
- `candidate_pair_id`
- `session_id`
- `frames_rendered`
- `candidate_types`
- `allowed_route_classes`
- `blocked_route_classes`

The deeper evidence contract remains in
`tools/scripts/remoteapp-network-fallback-e2e.sh`.

## Verification plan

- `bash -n tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/remoteapp-network-fallback-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_network_fallback_e2e.sh`
- `git diff --check -- tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-network-aggregate-route-gate/00-intent.md pr/20260823-remoteapp-network-aggregate-route-gate/01-invariants.md pr/20260823-remoteapp-network-aggregate-route-gate/02-decisions-and-evidence.md`

## Verification results

- `bash -n tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh tools/scripts/remoteapp-network-fallback-e2e.sh tests/scripts/test_remoteapp_network_fallback_e2e.sh`
- `git diff --check -- tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-network-aggregate-route-gate/00-intent.md pr/20260823-remoteapp-network-aggregate-route-gate/01-invariants.md pr/20260823-remoteapp-network-aggregate-route-gate/02-decisions-and-evidence.md`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tools/scripts/remoteapp-network-fallback-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tests/scripts/test_remoteapp_network_fallback_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
