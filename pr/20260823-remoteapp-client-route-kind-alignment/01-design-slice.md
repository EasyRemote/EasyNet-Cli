# RemoteApp client route-kind alignment

## Product seam

The network fallback verifier distinguishes product route kinds
`direct`, `stun_srflx`, `turn_relay`, and `easynet_relay`. Browser RTCStats
selected candidate-pair evidence can only prove the selected route class:
`direct`, `stun_srflx`, or `relay`.

The frontend boundary must therefore preserve two separate fields:

- `client_transport.route_kind`: product route kind from daemon route state.
- `selected_candidate_pair.selected_route_class`: selected ICE pair class from
  browser RTCStats.

## Invariants

- Browser stats must not infer TURN-vs-EasyNet relay authority from candidate
  type alone.
- Runtime route state remains the authority for product route kind.
- Selected candidate-pair evidence remains useful for selected/nominated/
  succeeded ICE proof and media-after-route binding.

## Verification

- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh`
