Verification checklist:

- `cargo test daemon::invocation::admission::target_gate::tests --lib`
- `go test ./sdk/go -run 'TestDirectRuntimeGRPCErrorProjects.*Descriptor'`
- Python direct-runtime focused test.
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`
