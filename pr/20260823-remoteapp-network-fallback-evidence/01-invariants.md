# RemoteApp network fallback evidence invariants

## Evidence invariants

- `proof_mode` must be `real_network_fallback_matrix`.
- `component_mock` must be false.
- `real_backend_runtime` must be true.
- `product_complete_claim` must be false.
- The runner kind must be `two_device`, `network_namespace`, or `deployment`.
- The artifact must not contain raw credential, token, password, secret, or
  private-key fields.

## Scenario invariants

The verifier requires one passing scenario for each route kind:

1. `direct`
2. `stun_srflx`
3. `turn_relay`
4. `easynet_relay`

Each scenario must include:

- canonical caller/callee device URAs;
- a selected Resource URA and session id;
- public `remote_desktop.create_session`, `remote_desktop.attach`,
  `remote_desktop.watch_events`, and `remote_desktop.end_session` ability
  evidence;
- selected Resource URA subject binding for those abilities;
- connected/completed WebRTC ICE state;
- selected candidate-pair details;
- rendered media frames and positive media duration;
- visible terminal receipt bound to the same session id.

## Route-specific invariants

- `direct` must use host candidates without relay candidates.
- `stun_srflx` must include a server-reflexive candidate.
- `turn_relay` must include a relay candidate and redacted credential evidence.
- `easynet_relay` must identify `route_provider=easynet_relay`, prove relay
  reachability, and include a relay session id without exposing secrets.

## Product boundary

This verifier defines the network fallback artifact required before product
completion. It does not itself create real network evidence.
