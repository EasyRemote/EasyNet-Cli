# Architecture

`src/daemon/plugins/realtime.rs` owns the projection from package manifest + daemon ability catalogue into activation readiness.

The correct dependency direction is:

1. Plugin package manifest input is parsed by `manifest.rs`.
2. Daemon runtime state is read from the canonical ability catalogue.
3. `realtime.rs` computes output-only activation plans.
4. `surface.rs` and `broker.rs` compose output-only product reports.
5. CLI/UI renders or serializes those reports.

There is no reverse path where a product supplies an activation plan or plugin surface report to the daemon.
