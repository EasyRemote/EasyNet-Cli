# Boundary Proof

## Ownership

The added API belongs to the SDK Runtime Core facade. It decodes shared daemon
SDK health DTOs through injected transports. It does not own daemon startup,
daemon discovery, product routing, product health route shaping, keyring policy,
plugin policy, or backend behavior.

## Invariants

- API liveness and runtime readiness remain separate fields.
- Diagnostics are an optional transport capability, not a hidden fallback.
- Transport failures become typed SDK errors with retry hints.
- Malformed payloads become deterministic validation errors.
- Closed clients reject calls deterministically.
- Java and Swift expose the same DTO fields and state helpers as the mature
  SDKs.
- No product-specific address, directory, receipt, or lifecycle terminology is
  introduced.

## Compatibility

The public Java and Swift Runtime Core surfaces remain compatible. Health is an
additive seam over injected transports and does not alter existing invocation,
stream, bidi, or discovery contracts.

