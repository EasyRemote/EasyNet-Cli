# Verification

Planned checks:

- Add focused tests proving `has_rpc` / `has_stream` / `has_bidi` do not stay
  true solely because `LocalRuntime` still has ability options.
- Add focused tests proving `has_rpc` / `has_stream` / `has_bidi` do not stay
  true solely because handler rows still exist after control-plane removal.
- Add focused tests proving `list_rpc_names` requires a committed RPC
  control-plane record.
- `cargo test --features axon-pb --lib <focused ability dispatch tests>`
- `cargo check --features axon-pb`
- `tools/scripts/check-architecture-convergence.sh`
- Targeted `rustfmt --check`
- `git diff --check`

## Delta

- `has_rpc`, `has_stream`, and `has_bidi` now require a committed
  control-plane mode record plus an execution-index handler for that same mode.
- `list_rpc_names` is projected from committed RPC control-plane records rather
  than the handler-map union.
- Removed the obsolete execution-index RPC-name union helper after the
  control-plane projection became the only publication path.
