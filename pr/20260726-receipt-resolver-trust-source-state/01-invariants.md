# Invariants

- Local key-service receipt signer resolution remains the first authority for local runtime receipts.
- Realm trust anchor resolution remains the second authority for non-local trusted signers.
- A malformed realm trust anchor must not be swallowed with `.ok()`.
- Empty and missing realm trust anchors must be represented as explicit trust-source states.
- The resolver must never report a generic `empty or unavailable` trust source.
