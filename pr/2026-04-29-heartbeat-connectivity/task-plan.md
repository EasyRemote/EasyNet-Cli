# Task Plan: heartbeat-connectivity

## Scope
Fix EasyNet CLI/device heartbeat liveliness detection and device-to-device connectivity reporting.

## Invariants
1. Heartbeat probe must terminate with a deterministic success/failure result.
2. Offline/unreachable nodes must surface as explicit status, not silent success.
3. Connectivity diagnosis must distinguish local daemon absence, roster absence, and remote probe timeout.
4. Existing unrelated user edits must remain intact.

## Boundaries
- CLI facade may translate runtime outcomes into user-facing status.
- Runtime/control plane may provide probe semantics and timeout handling.
- No unrelated protocol or install flow changes.

## Plan
1. Inspect existing heartbeat/status/device connectivity flows.
2. Reproduce or infer failure path from code/tests.
3. Patch the minimal layer that restores bounded liveness/connectivity semantics.
4. Verify with targeted tests or CLI-level checks.
5. Record outcome here.

## Decision Log
- Root cause is semantic, not transport absence: operator-facing status surfaces were
  reading local state files and hard-coding HEALTHY/local_only instead of using the
  live federation path.
- Local liveness is now defined by a real `observe.health` probe over control.sock,
  not by the presence of `runtime.json`.
- Cross-device visibility now uses `federation.resolve`; direct peer reachability is
  classified by `federation.forward_invoke(..., "observe.health")`.
- Remote mutating fleet operations remain unchanged; this patch fixes diagnosis and
  node visibility first.

## Verification
1. Toolchain was present but missing from this Codex process `PATH`. Verified with absolute path:
   `~/.cargo/bin/cargo`, `~/.cargo/bin/rustc`, `~/.cargo/bin/rustfmt`.
2. `cargo fmt --all` completed successfully.
3. `cargo check` completed successfully on the patched tree.
4. `cargo build --bin easynet --bin easynet-daemon` completed successfully.
5. Live verification:
   - `target/debug/easynet runtime start --foreground --no-mcp` succeeded when
     `PATH` included `/usr/local/bin` so `axon-runtime` could be found.
   - `target/debug/easynet runtime status` now refuses false-positive "alive"
     output when the local daemon/control socket is unreachable.
   - `target/debug/easynet doctor` now fails the runtime check when
     `observe.health` cannot be reached.
   - `target/debug/easynet device list --state all --format json` returned:
     `federation_view = "federated"` with `resolve_latency_ms`, plus an explicit
     reason when only the local device was advertised.
6. Residual runtime findings discovered during live debug:
   - Background-start verification is sensitive to the current exec harness because
     child processes may be reaped when the launching command exits; foreground mode
     provided the stable end-to-end verification path.
   - `runtime stop` still leaves stale `control.sock`, `runtime-dispatch.sock`,
     `control.json`, and pid files behind after process exit.
   - Historical `heartbeat.log` entries still show older
     `AXON_EASYNET_SUBJECT_KEY_UNREGISTERED` failures from prior runs; the new
     foreground verification path used direct in-process heartbeat and validated the
     CLI liveness surfaces independently of that stale file.
