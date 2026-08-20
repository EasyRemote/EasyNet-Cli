# Decisions Log

- Keep this slice Go-only at the language facade layer because Rust/C ABI and
  Python already expose the same lifecycle functions.
- Do not add daemon lifecycle compatibility shims; missing C ABI symbols should
  fail at bind time, matching other C ABI profile transports.
- Add explicit complete lifecycle request methods instead of changing the
  existing `AbilityImplID` shorthand. The shorthand remains useful for in-memory
  tests and facade-local transports, but complete daemon carriers are required
  for Runtime Core and C ABI execution.
