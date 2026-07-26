Execution checklist
===================

- [x] Add provider-specific subject validation in the FFI descriptor resolver.
- [x] Reuse canonical URA parsing and all-zero principal guards.
- [x] Update receipt-history descriptor resolver tests to use runtime-state read
      subject.
- [x] Add negative tests for Device subject and missing subject.
- [x] Add SPEC v2 gate coverage for the FFI receipt-history subject guard.
- [x] Run targeted FFI tests.
- [x] Run fmt, diff check, and architecture gates.
