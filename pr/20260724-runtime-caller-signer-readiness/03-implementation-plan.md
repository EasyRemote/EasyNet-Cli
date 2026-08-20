# Implementation Plan

1. Add a reusable runtime caller signer custody proof helper in `self_identity`.
2. Domain-separate the proof challenge and verify the returned signature with the signer public key.
3. Replace `validate_device_ready_capabilities` with a stricter readiness validator that also receives credentials and invokes the identity proof.
4. Add unit tests for success, missing capability, missing credential User URA, and signer proof failure.
5. Run targeted Rust tests, formatting, and convergence gates.

