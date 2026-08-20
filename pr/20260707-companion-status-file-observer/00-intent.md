# Companion Status File Observer

## Goal

Converge desktop companion heartbeat observation into one daemon-owned runtime abstraction and close the Windows supervisor stop gap required by the desktop companion plugin SPEC.

## Boundary

- EasyNet-Cli daemon owns desktop companion plugin lifecycle and local user-session supervisor behavior.
- Axon SDK/runtime is not involved because companion lifecycle does not define Invocation, admission, receipts, federation, or protocol state.
- Language SDKs and CLI consume the projected DTO; they must not reclassify heartbeat freshness.

## Invariants

- A status file for the wrong package or version is never treated as running.
- A stale heartbeat projects to `stale`, not `running`.
- An invalid status file projects to a health error instead of silently falling back to a process probe.
- Platform supervisors own process start/stop only; shared heartbeat classification lives in one runtime object.

## Verification

- Unit tests cover fresh, stale, invalid, and version-mismatch status files.
- Focused companion tests must pass after refactoring.
- A terminology audit over touched files must not introduce forbidden architecture terms.

## Results

- `cargo test -q daemon::plugins::companion` passed with 12 focused tests.
- `cargo test -q plugin_host_install` passed.
- `cargo test -q plugin_host_update` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.
- `cargo check -q --target x86_64-pc-windows-gnu` was attempted but blocked before crate checking by missing `x86_64-w64-mingw32-gcc` while building `ring`.
