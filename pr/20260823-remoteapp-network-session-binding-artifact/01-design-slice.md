# RemoteApp network session-binding artifact gate

## Product seam

The network fallback verifier required real route scenarios, selected ICE
candidate-pair state, fixture constraints, and media rendered after pair
selection. That still left a proof gap: a runner could attach candidate-pair
stats or rendered-media counters from a different WebRTC connection while the
RemoteApp scenario's selected Resource URA and session id remained unproven.

## Slice

- Require WebRTC evidence to bind the selected Resource URA, RemoteApp session
  id, caller device URA, callee device URA, and expected route kind.
- Require every selected candidate pair to expose a stable
  `candidate_pair_id`.
- Require rendered media evidence to bind the same selected Resource URA,
  session id, route kind, and candidate pair id.
- Keep credential redaction and direct/STUN/TURN/EasyNet route expectations
  unchanged.

## Expected impact

This does not prove live NAT/STUN/TURN/EasyNet relay reachability without a
real two-device, network-namespace, or deployment artifact. It closes the seam
where unrelated WebRTC stats could be reused as proof for a RemoteApp network
fallback scenario.
