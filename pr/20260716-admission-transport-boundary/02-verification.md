# Verification: admission transport boundary state

## Evidence

- PASS: `rustfmt --edition 2021 --check src/daemon/invocation/admission/admission_facade.rs src/daemon/invocation/dispatch/daemon_invocation_service.rs src/daemon/boot/invocation/mod.rs src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/invocation/streams/stream_dispatcher.rs src/daemon/invocation/bidi/bidi_dispatcher.rs`
- PASS: `bash -n tools/scripts/check-architecture-convergence.sh && bash -n tests/scripts/test_check_architecture_convergence.sh`
- PASS: `cargo test --features axon-pb off_box_facade --lib`
- PASS: `cargo test --features axon-pb local_self --lib`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `tests/scripts/test_check_architecture_convergence.sh`

## Final scoped check

- PASS: `git diff --check -- src/daemon/boot/invocation/mod.rs src/daemon/invocation/admission/admission_facade.rs src/daemon/invocation/bidi/bidi_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service.rs src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/invocation/streams/stream_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh`
