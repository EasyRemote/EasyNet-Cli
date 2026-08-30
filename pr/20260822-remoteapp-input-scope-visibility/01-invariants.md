# Invariants

1. Product-flow checks must fail if the UI stops rendering daemon-projected
   input scope.
2. Product-flow checks must fail if component coverage no longer asserts
   pointer/keyboard enablement visibility.
3. The readiness matrix and audit must keep `input_injection` incomplete until
   real OS input injection, focus safety, latency, and E2E evidence exist.
4. The frontend must not synthesize input authority from UI-only state; it must
   consume the daemon `inputReadiness` projection.
