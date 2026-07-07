# Boundary Proof

## Ownership

The language facade owns DTO validation, exact request JSON transfer, summary
projection, and client lifecycle. The daemon/Axon provider owns receipt
verification, chain continuity, ledger access, receipt URA construction, and
causal-reference projection.

## Invariants

- Receipt fetch requests carry the complete Invocation tuple fields.
- Fetch selectors have exactly one of `invocation_ura`, `request_id`, or
  `trace_id`.
- Fetch carriers use the descriptor ref supplied by the request and do not
  synthesize descriptor grammar.
- Summary projection preserves `verified` exactly and never upgrades
  summary-only data to cryptographic evidence.
- Receipt refs require an explicit receipt URA and 64-character lowercase hash.
- Causal-ref construction from a summary without receipt URA and hash fails.
- Missing transport capabilities fail explicitly.

## Compatibility

The change is additive for Java and Swift. Runtime Core, Health, and Directory
+ Identity APIs remain unchanged.
