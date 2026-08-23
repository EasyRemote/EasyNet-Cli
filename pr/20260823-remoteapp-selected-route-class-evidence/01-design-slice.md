# RemoteApp selected route-class evidence slice

## Product gap

The native media stats projection now exposes selected candidate types and
protocol, but downstream E2E reports still need a stable route-class field to
compare with direct/STUN/relay scenario expectations. Without a canonical route
class, every runner would have to reimplement candidate-type precedence.

## Boundary decision

- WebRTC still owns ICE nomination and selected-pair choice.
- RemoteApp only derives an evidence label from selected local/remote candidate
  stats.
- Relay evidence is classified as `relay`; this slice does not claim whether
  the relay was generic TURN or EasyNet relay. That subtype remains runner or
  deployment evidence.

## Invariants

1. Any relay candidate in the selected pair yields `selected_route_class =
   "relay"`.
2. Server-reflexive or peer-reflexive candidates yield
   `selected_route_class = "stun_srflx"` unless relay is present.
3. Host-only selected pairs yield `selected_route_class = "direct"`.
4. Missing candidate stats yield null, not a guessed route class.
5. The route-class projection does not include addresses, TURN usernames,
   credentials, or relay URLs.

## Verification

- Native media unit tests cover direct, STUN/reflexive, relay, and missing
  candidate stats.
- Product closure gates require the selected route-class field in native
  WebRTC stats.

## Verified commands

- `rustfmt --edition 2021 plugins/remote-desktop/src/media/native.rs`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json >/dev/null`
- `cargo test -q selected_candidate_pair_route_class_distinguishes_direct_and_stun`
- `cargo test -q selected_candidate_pair_route_evidence_uses_typed_candidate_stats`
- `cargo test -q selected_candidate_pair_route_evidence_does_not_guess_missing_stats`
- `git diff --check`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
