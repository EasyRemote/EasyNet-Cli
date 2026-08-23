# RemoteApp Network Fixture Artifact Slice

## Intent

Close the network fallback evidence seam where a report could present a
selected WebRTC candidate pair and route-class label without proving that the
pair was selected under the intended direct/STUN/TURN/EasyNet relay network
constraints.

## Boundary

This slice updates the network fallback evidence verifier, closure audit, and
readiness documentation. It does not provision NAT/TURN infrastructure, change
the RemoteApp media implementation, or claim product completion.

## Product invariant

Fallback behavior is not proven by a route label alone. A passing artifact must
prove, per route scenario:

- a real runner kind: two-device, network namespace, or deployment;
- `network_fixture.route_constraints_applied=true`;
- the fixture expected route kind matches the scenario;
- allowed and blocked route classes are explicit;
- selected-pair observation happens after route constraints are applied;
- selected route class is allowed by the fixture and not blocked by it;
- first rendered media frame happens after selected-pair observation.

## Architecture decision

Keep route-constraint proof in the E2E artifact contract. Runtime stats can
project the selected candidate pair, but only the environment runner can prove
the topology constraints that made direct/STUN/TURN/relay fallback meaningful.

## Verification checklist

- `bash -n tools/scripts/remoteapp-network-fallback-e2e.sh`
- `bash tools/scripts/remoteapp-network-fallback-e2e.sh --self-test`
- negative artifact: missing route constraints fails
- negative artifact: media rendered before selected pair fails
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Non-claims

- This does not prove live NAT/STUN/TURN/EasyNet relay deployment without a real
  `--run` artifact.
- This does not prove OS capture, input injection, media adaptation, frontend
  lifecycle, or cross-device RemoteApp product readiness.
