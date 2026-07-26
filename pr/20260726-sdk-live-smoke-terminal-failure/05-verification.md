# Verification

Completed:

- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/python-sdk-live-smoke.sh`
- `tools/scripts/go-sdk-live-smoke.sh`
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo test admission_facade::tests::runtime_ --lib`
- `cargo test daemon::invocation::admission::policy_gate::tests:: --lib`
- `cargo test invocation_stream_close --lib`
- codegraph sync/status.

Not run:

- Full `tools/scripts/check-sdk-cutover-readiness.sh`; earlier failures include downstream EasyRemote/backend cutover issues outside this feature slice.
