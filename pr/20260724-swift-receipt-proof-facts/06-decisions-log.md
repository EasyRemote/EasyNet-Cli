# Decisions Log

## 2026-07-24

- Treat Swift `receipt_ref` terminal receipt fixtures as a legacy compatibility seam.
- Preserve public dictionary projection while requiring canonical receipt validation before exposure.
- Keep Swift `InvocationResult.terminalReceipt` as `[String: String]` for public compatibility, but validate complete `[String: Any]` proof facts first.
- Use the same authority proof hash semantics as Rust/Go/Python/Java/Node: hash proof payload when present, otherwise hash canonical authority-binding projection bytes.
