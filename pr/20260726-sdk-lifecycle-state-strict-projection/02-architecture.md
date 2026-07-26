# Architecture

## Layering

- Canonical runtime model owns lifecycle vocabulary.
- SDKs implement the same lifecycle parser independently by language, with identical accepted carrier values.
- Product repositories consume SDK projections; they do not contribute lifecycle aliases.

## Module boundaries

- Go: `sdk/go/invocation_state.go` owns lifecycle parsing for runtime receipts and invocation results.
- Python: `sdk/python/easynet_sdk/invocation_state.py` owns lifecycle parsing.
- Node: `sdk/node/index.js` owns receipt state projection.
- Java: `sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java` owns receipt state projection.
- Swift: `sdk/swift/Sources/RuntimeSDK/Runtime.swift` owns receipt state projection.

## Ownership rule

No SDK may accept product-specific or retired lifecycle spelling. Canonicalization is projection from a canonical carrier into language-native API values, not normalization of arbitrary carriers.
