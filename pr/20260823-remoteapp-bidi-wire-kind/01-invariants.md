# Invariants

- Raw bytes are a transport representation, not a new Invocation semantic.
- Invocation caller/callee/ability/subject/nonce/causal_context/args remain unchanged.
- Session lifecycle, terminal closure, receipts, and errors remain canonical metadata, not payload-side conventions.
- `metadata_json_plus_binary` is allowed for RemoteApp attach because the implementation emits JSON metadata frames plus binary JPEG/H.264 media frames.
- Browser/CDP `json_frames` remains JSON-only and is not changed by this work.
- The existing local binary-capable adapter remains the execution adapter until the in-flight dispatcher branch is ready to expose a distinct runtime wire kind.
