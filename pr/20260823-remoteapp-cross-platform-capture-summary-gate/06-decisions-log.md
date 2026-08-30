# Decisions Log

- 2026-08-23: Add per-target capture summaries to the verifier report instead of making the aggregate parse full raw evidence. This preserves verifier ownership while closing the product-completion weak-report seam.
- 2026-08-23: Keep unsupported platform handling in the capture verifier for verifier contract testing, but product completion still requires all macOS/Windows/Linux display/window/application targets to pass.
