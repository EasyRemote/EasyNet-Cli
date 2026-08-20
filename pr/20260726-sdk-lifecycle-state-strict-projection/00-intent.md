# Intent

## Goal

Converge SDK receipt lifecycle-state parsing across Go, Python, Node, Java, and Swift onto one canonical runtime model.

The SDK must treat runtime receipt `state` as a canonical runtime fact, not as a product-friendly or legacy-normalized string. Receipt state parsing must fail closed on retired spellings while preserving existing public method names and return projections.

## Non-goals

- Do not add product-specific EasyNet or EasyRemote lifecycle concepts.
- Do not change receipt `receipt_type` semantics.
- Do not change public SDK method names.
- Do not introduce compatibility aliases for legacy state spellings.

## Acceptance criteria

- All SDKs accept canonical receipt lifecycle state spellings only.
- Legacy lowercase, screaming-case, whitespace-padded, or punctuation-normalized state values are rejected.
- Existing public lifecycle projection methods remain available.
- Failure-path tests exist in each touched SDK.
- SDK conformance manifests are regenerated if source attestation changes.
