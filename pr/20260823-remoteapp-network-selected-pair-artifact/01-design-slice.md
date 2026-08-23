# RemoteApp network selected-pair artifact contract

## Product gap

The network fallback verifier requires route classes and a
`selected_candidate_pair` object, but the artifact could still describe a route
model rather than the actual ICE pair selected by WebRTC. A real direct/STUN/
TURN/EasyNet relay proof needs to show that the candidate pair was nominated,
selected, succeeded, and used for byte transfer.

## Boundary decision

- The verifier validates evidence from a real two-device, network namespace, or
  deployment runner; it does not provision network infrastructure.
- The runner owns extracting selected candidate-pair stats from the browser or
  native WebRTC stack.
- The verifier requires nominated/selected/succeeded candidate-pair evidence
  with local and remote candidate IDs before route-class assertions can pass.

## Invariants

1. Every route scenario must report `selected_candidate_pair.selected=true`.
2. Every route scenario must report `selected_candidate_pair.nominated=true`.
3. Every route scenario must report `selected_candidate_pair.state=succeeded`.
4. Every route scenario must bind non-empty `local_candidate_id` and
   `remote_candidate_id`.
5. Route-class checks remain tied to candidate types and credentials must remain
   redacted.

## Verification checklist

- `bash -n tools/scripts/remoteapp-network-fallback-e2e.sh` — passed
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
  >/dev/null` — passed
- `bash tools/scripts/remoteapp-network-fallback-e2e.sh --self-test` —
  passed
- negative `--run --evidence-json` fixture without nominated selected-pair
  evidence — failed as expected
- negative `--run --evidence-json` fixture with non-succeeded selected pair
  state — failed as expected
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` —
  passed after correcting the mutation replacement to hit the verifier string
- `git diff --check` — passed
