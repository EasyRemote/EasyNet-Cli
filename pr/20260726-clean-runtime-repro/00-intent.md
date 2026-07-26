# Intent

## Goal

Reproduce and eliminate the product-visible invocation failures reported on a
clean runtime state:

- stale device read-model routes resolving to offline owners;
- descriptor reference lookup failures for meta/resource abilities;
- receipt-history admission failures caused by subject mismatch;
- caller signer lookup failures leaking local key-service details.

## Non-goals

- Preserve compatibility with stale local runtime data.
- Add product-specific abstractions to the SDK.
- Add fallback routing paths that bypass canonical runtime authority.

## Acceptance criteria

- A clean runtime state produces deterministic, typed failures before pairing.
- A clean paired Docker/product topology can invoke meta and receipt-history
  abilities without stale descriptor, signer, or subject mismatch failures.
- Any required fix preserves the SDK as a canonical runtime model rather than an
  EasyNet-specific facade.
