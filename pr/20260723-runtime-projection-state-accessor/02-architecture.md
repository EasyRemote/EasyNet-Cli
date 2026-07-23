# Architecture

## Layering

- `daemon::boot::lifecycle::projection` owns typed access to the persisted
  runtime session projection.
- CLI renderers consume lifecycle reports and may borrow projection state, but
  they do not own projection schema semantics.
- `RuntimeLifecycleService` remains the mutation boundary for projection
  persistence and rollback behavior.

## Boundary proof

The previous `as_runtime_state` name coupled callers to the storage type and
described the consumers as legacy CLI renderers. Replacing it with a
projection-state accessor preserves the internal data shape while removing the
legacy ownership implication.
