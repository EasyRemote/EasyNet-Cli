# RemoteApp network route-class artifact contract

## Product gap

RemoteApp native WebRTC stats now expose `selected_route_class`, but the live
network fallback artifact verifier still accepts evidence that omits that field.
That leaves direct/STUN/relay scenario reports dependent on per-runner candidate
type interpretation instead of the Runtime-owned projection.

## Boundary decision

- WebRTC still selects the ICE candidate pair.
- Runtime stats project a route-class evidence label from selected candidate
  types.
- The live artifact verifier checks that each route scenario carries and matches
  this Runtime-projected label.
- The verifier does not infer TURN versus EasyNet relay from
  `selected_route_class`; deployment-specific fields still prove those subtypes.

## Invariants

1. Every real network fallback scenario must include
   `webrtc.selected_candidate_pair.selected_route_class`.
2. `direct` scenarios require `selected_route_class = "direct"`.
3. `stun_srflx` scenarios require `selected_route_class = "stun_srflx"`.
4. `turn_relay` and `easynet_relay` scenarios require
   `selected_route_class = "relay"`.
5. Candidate type checks remain as defense-in-depth and to catch inconsistent
   artifacts.
6. Missing or unknown route classes fail the artifact verifier.

## Verification checklist

- `tools/scripts/remoteapp-network-fallback-e2e.sh --self-test`
- negative evidence fixture without `selected_route_class` must fail
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Verified commands

- `bash -n tools/scripts/remoteapp-network-fallback-e2e.sh`
- `bash tools/scripts/remoteapp-network-fallback-e2e.sh --self-test`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json >/dev/null`
- negative `--run --evidence-json` fixture without
  `selected_route_class` rejected with `selected_route_class must be ...`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`
