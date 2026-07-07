# Boundary Proof

The report files are language-owned evidence over the shared SDK conformance
case catalog. They do not define new runtime semantics.

The Java and Swift reports are restricted to:

- ABI compatibility checks
- typed SDK error projection
- complete Invocation tuple construction
- builder lifecycle state
- bounded stream/bidi backpressure
- MEMC seam exclusivity assertions

Provider-backed daemon transports, profile-specific DTOs, and product cutover
remain outside these reports.
