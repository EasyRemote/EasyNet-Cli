# Verification

## Commands

```text
rustfmt --edition 2021 --check src/daemon/execution/mcp/mod.rs
cargo test --features axon-pb mcp_child_stdio --lib
cargo test --features axon-pb read_mcp_frame --lib
tools/scripts/check-architecture-convergence.sh
git diff --check
```

## Result

All commands passed.

## Notes

Focused tests exercise the child stdout decoder directly with in-memory async
readers so the failure path is deterministic and does not require spawning an
external MCP process.
