# Boundary Proof

The root abstraction is not remote-desktop decoding. The defect is gate
ownership: a product-visible retired wire alias was protected by a standalone
script and a Cargo script-check wrapper, but the canonical SPEC v2 gate did not
own that boundary directly.

The converged boundary is:

1. Remote desktop exposes exactly one canonical transport wire spelling:
   `webrtc`.
2. The retired `web_rtc` spelling is not a serde alias, fallback, or alternate
   product contract.
3. SPEC v2 owns the regression proof by invoking the same product contract
   boundary script and by carrying a negative self-test fixture.

This does not alter runtime behavior. It removes a verification gap that could
let compatibility vocabulary return outside the main convergence gate.
