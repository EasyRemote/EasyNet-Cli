# Invariants

- Opaque `InvocationResult` receipt projection remains a non-verifying provider output view.
- Required receipt validation must include invocation identity, receipt hashes, caller/callee/subject bindings, nonce, causal binding, descriptor version, schema hash, impl hash, runtime environment, authority binding, authority proof, input/output hashes, signature, and parent receipt binding.
- The Go and Python SDKs expose the same validation semantics.
- No product-specific receipt model or EasyNet-specific receipt naming is introduced.
