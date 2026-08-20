# API Contract

Public Java SDK names remain stable.

## Error contract

Missing proof facts raise `SDKError`:

- code: `RECEIPT_PROOF_FACTS_MISSING`
- stage: `receipt`
- retry: `never`

Invalid non-missing proof-fact values remain validation errors.
