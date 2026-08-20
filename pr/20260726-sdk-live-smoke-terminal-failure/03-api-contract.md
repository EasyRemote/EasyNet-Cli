# API Contract

- Public Go/Python invocation APIs stay unchanged.
- C ABI `runtime_invocation_invoke` continues returning JSON for complete invocation outcomes and typed SDK errors for invalid transport/projection failures.
- A receipt-free invocation result must carry:
  - terminal failed lifecycle state;
  - typed error code/stage/message;
  - no success flag;
  - no synthetic success receipt.
- Missing or ambiguous receipt fields remain invalid.
