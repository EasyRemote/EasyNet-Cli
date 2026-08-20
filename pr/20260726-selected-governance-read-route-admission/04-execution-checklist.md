Execution checklist
===================

- [x] Rename the remote-only module to selected-route governance-read admission.
- [x] Update unary/stream/bidi dispatch imports and call sites.
- [x] Add LocalRuntime selected-route rejection before runtime admission.
- [x] Add regression coverage for local receipt-history public-action subject.
- [x] Update SPEC v2 gate checks to enforce selected-route, not remote-only,
      ownership.
- [x] Run targeted Rust tests.
- [x] Run fmt, diff check, and architecture gates.
