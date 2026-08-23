# Invariants

1. Product completion is a single aggregate claim; child verifiers must still set
   `product_complete_claim=false`.
2. Network fallback product completion requires all route kinds:
   `direct`, `stun_srflx`, `turn_relay`, and `easynet_relay`.
3. Each route scenario summary must include a non-empty session id and stable
   candidate-pair id.
4. Each route scenario must have connected or completed ICE state and rendered
   media frames.
5. `selected_route_class` must match the route expectation:
   `direct -> direct`, `stun_srflx -> stun_srflx`, relay routes -> `relay`.
6. The selected route class must be allowed by the reported network fixture and
   not appear in the blocked classes.
7. Candidate types are checked as defense-in-depth: direct must include host and
   exclude relay; STUN must include srflx/prflx; relay routes must include relay.
8. Missing or malformed route summaries fail closed; they cannot be interpreted
   as product-ready network fallback evidence.
