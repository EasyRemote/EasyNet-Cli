# RemoteApp selected candidate-pair evidence slice

## Product gap

The network fallback verifier requires real selected WebRTC candidate-pair
evidence for direct, STUN server-reflexive, TURN relay, and EasyNet relay
paths. The native media stats projection already exposes the selected pair id,
bytes, and RTT, but not the selected local/remote candidate types or protocol.
Without those fields, a real network artifact cannot prove which route class
was actually selected.

## Boundary decision

- ICE selection stays owned by WebRTC.
- RemoteApp does not infer or force a route.
- The plugin only projects the selected pair's candidate types and protocol from
  the authoritative WebRTC stats report.
- Candidate addresses and credentials remain excluded from the product stats
  projection.

## Invariants

1. `selected_candidate_pair` includes `local_candidate_type`,
   `remote_candidate_type`, and `protocol` when WebRTC exposes the referenced
   candidate stats.
2. Missing candidate stats are represented as null evidence, not guessed.
3. The projection does not leak candidate addresses, TURN usernames,
   credentials, or secret-bearing URLs.
4. This is evidence plumbing only; it does not change transport routing,
   lifecycle, or readiness decisions.

## Verification

- macOS native media unit tests prove candidate type/protocol projection from
  typed WebRTC candidate stats.
- Product closure gates require the native stats projection to expose the
  selected candidate-pair route fields needed by the network fallback verifier.
