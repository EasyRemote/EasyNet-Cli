# Decisions Log

## 2026-07-23

- Treat FFI runtime sizing as a host-side runtime policy. The existing behavior
  is correct; the architecture defect is naming that implies fallback/device
  ownership.
- Add a narrow unit test for the host-default minimum rather than mutating the
  process environment in concurrent Rust tests.
