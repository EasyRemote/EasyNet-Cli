# Java/Swift Receipt Provider Seam Plan

## Goal

Close the Java and Swift P1 Receipt seams for shared receipt profile operations:
projection, verification, and chain verification over injected transports. The
clean scaffold pass also requires completing the Java/Swift authority metadata
seam that Invocation builders already reference.

## Scope

- Add injected transport methods for receipt projection and verification.
- Add `ReceiptChain` request validation and chain verification delegation.
- Add Java/Swift authority metadata DTOs, request/projection parsing, transport
  clients, and ambiguous metadata guardrails required by clean package builds.
- Preserve opaque `receipt_ura` semantics. The SDK must require daemon/Axon
  returned `receipt_ura` plus `receipt_hash_hex` facts and must not construct a
  receipt URA locally.
- Extend Java and Swift seam tests with summary-only verification guardrails,
  explicit receipt ref validation, transport delegation, and chain verification.

## Out Of Scope

- Provider transports for Java or Swift.
- RFC-007 receipt URA builder behavior.
- Local cryptographic receipt verification in language facades.
- Authority signing, verification, or trust-anchor admission.
