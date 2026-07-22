# Architecture

`src/daemon/ability/dispatch.rs` owns the in-process catalogue projection.

- Control plane: committed ability publication records.
- Execution index: authority-scoped handler dispatch rows.
- Diagnostics: narrow predicates that answer explicit questions without creating an alternate read model.

The removed helper crossed these boundaries by listing execution-index names in a way that could be reused as a discovery source. The retained diagnostic predicate keeps cohesion with hot-reload collision checks without exposing a second publication path.
