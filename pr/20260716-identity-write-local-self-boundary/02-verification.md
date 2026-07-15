# Verification

## Evidence

- `rustfmt --edition 2021 src/daemon/invocation/admission/admission_facade.rs src/daemon/invocation/admission/identity_write_gate.rs src/daemon/invocation/dispatch/unary_dispatcher.rs`
- `rustfmt --edition 2021 --check src/daemon/invocation/admission/admission_facade.rs src/daemon/invocation/admission/identity_write_gate.rs src/daemon/invocation/dispatch/unary_dispatcher.rs`
- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- `cargo test --features axon-pb identity_write_gate --lib`
- `cargo test --features axon-pb local_self --lib`
- `git diff --check -- src/daemon/invocation/admission/admission_facade.rs src/daemon/invocation/admission/identity_write_gate.rs src/daemon/invocation/dispatch/unary_dispatcher.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh`

## Result

- Identity trust-row writers no longer own a separate loopback predicate or caller flag.
- `AdmissionTransportBoundary` owns the local-self caller predicate and `IdentityWriteGate` consumes that state as a read-only policy input.
- `OffBoxStrict` rejects daemon-URA spoofing for identity trust-row writes without an anchor entry.
