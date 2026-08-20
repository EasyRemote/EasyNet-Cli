# Architecture

## Boundary

`src/daemon/invocation/bidi/state/presence.rs` owns session liveness state. It is the only layer allowed to mutate the presence map.

## Refactoring direction

Previously, `PresenceRegistry` accepted arbitrary strings and downstream projections decided what to ignore. That creates multiple liveness interpretations and lets legacy rows survive until a product path happens to filter them.

The converged model validates the principal key at the mutation boundary and keeps read models pure.
