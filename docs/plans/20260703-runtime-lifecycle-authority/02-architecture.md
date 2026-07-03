# Runtime Lifecycle Architecture

Layering:

- `daemon::boot::lifecycle`: lifecycle authority objects, pure state
  classification, preflight decisions, stop planning, projection transaction.
- `daemon::boot::process`: daemon process launch/attach/stop primitive.
- `daemon::control::discovery`: `control.json` discovery document.
- `daemon::persistence::config`: storage for existing `runtime.json` and pid
  paths.
- `cli::commands::{start,status,stop}`: parse arguments and render reports.

Object model:

- `DaemonDiscoverySnapshot`: process facts from pidfile, `control.json`, and
  endpoint probes.
- `RuntimeSessionProjection`: explicit wrapper for `runtime.json`.
- `RuntimeLifecycleStatus`: deterministic local lifecycle state machine.
- `RuntimeStartRequest`: requested mode/realm/node identity for attach checks.
- `RuntimeStartPreflightReport`: selected action before daemon launch.
- `RuntimeStopPlan`: side-effect-free shutdown plan.
- `RuntimeLifecycleService`: facade that sequences observe, preflight, stop
  planning, and projection commit rollback.

The CLI keeps the public command behavior, but lifecycle policy moves out of
command files.
