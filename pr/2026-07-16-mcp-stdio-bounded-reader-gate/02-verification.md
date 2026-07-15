# Verification: MCP stdio bounded-reader gate

## Planned checks

- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- Focused Rust tests for MCP stdio bounded reader behavior.
- Scoped formatting/checks for touched files.

## Evidence recorded

- `tools/scripts/check-architecture-convergence.sh` passed.
- `tests/scripts/test_check_architecture_convergence.sh` passed, including the
  R23 negative fixture that reintroduces direct `read_line`.
- `cargo test --features axon-pb mcp_child_stdio_bounded_reader_drains_oversized_line --lib`
  passed.
- `cargo test --features axon-pb read_mcp_frame_rejects_oversized_content_length_before_body_allocation --lib`
  passed.
- `cargo test --features axon-pb bounded_reader --lib` passed.
