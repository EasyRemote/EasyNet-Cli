# Architecture convergence clean commit plan

## Scope

Repackage the verified runtime convergence work into reviewable semantic
commits without changing the already-validated behavior.

## Invariants

- User remains an accountability Principal, not an invocation callee.
- Service-owned public abilities use Service owner URAs as callees.
- Device remains execution host; device-native public abilities are owned by
  device-sponsored SystemAgents.
- Ability publication keeps descriptor, owner, authority, implementation, and
  route bindings distinct.
- Public invocation boundaries carry explicit subject, nonce, and causal
  context; daemon-system defaults remain named policies.
- RemoteApp target sessions insert only after live target proof commits.
- Browser plugin manifests, compiled bindings, descriptors, and real-invoke
  coverage must agree.

## Verification

- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh --root .`
- RemoteApp boundary scripts
- `cargo test -q --lib`
- `cargo test -q remote_desktop --lib`
- Rust pluginexec tests
- Go SDK tests
- host-gated RemoteApp decoded-frame E2E for window and application
