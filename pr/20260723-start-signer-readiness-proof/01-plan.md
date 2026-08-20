# Start signer readiness proof

## Goal

Make device-mode `easynet start` treat daemon Ready as insufficient unless the
daemon also advertises the `paired_user_runtime_signer` readiness capability.

## Root abstraction problem

Attach preflight already refuses a device daemon that lacks paired User signer
readiness, but the fresh-start path persists `runtime.json` and prints the
Welcome line after any daemon Ready event. That leaves a product state where
startup appears successful while the first canonical remote invocation fails
with `CALLER_SIGNER_UNAVAILABLE`.

## Invariants

1. Device-mode start must not save a runtime projection until daemon Ready also
   proves paired User caller signer custody.
2. Device-mode start must not print the success/Welcome surface when the proof
   is absent.
3. The proof must come from daemon control discovery capability flags emitted by
   invocation boot after signer registration, not from daemon mode inference.
4. If a freshly-spawned daemon reaches Ready without the proof, CLI start stops
   that child instead of leaving a broken daemon behind.
5. Hub-mode start remains unaffected because Hub callers do not need paired User
   signer custody.

## Boundary proof

- `InvocationTransportReady` is already the daemon-side object that owns runtime
  readiness capability projection.
- `control.json` already carries `capability_flags`, and lifecycle attach
  already checks `PAIRED_USER_RUNTIME_SIGNER`.
- Extending `BootProgressOutcome` to carry ready capability flags lets fresh
  start and attach converge on the same proof without adding another IPC path.

## Verification plan

1. Unit-test ready capability acceptance and rejection in `start.rs`.
2. Add a boundary script requiring fresh-start validation before projection
   persistence and requiring `start_boot_watcher` to capture ready capability
   flags.
3. Run targeted tests, formatting, SPEC v2 gate, architecture gate, and
   codegraph sync.
