# Execution Checklist

- [x] Inspect current worktree state.
- [x] Use codegraph to identify Node receipt proof-fact seam.
- [x] Implement canonical authority-binding projection bytes.
- [x] Enforce authority proof expected hash.
- [x] Enforce signer/issuer proof topology.
- [x] Add targeted Node runtime tests.
- [x] Run Node tests and canonical gates.
- [x] Commit stable changes with required author.

## Execution notes

- Extended Node receipt validation from field-shape checking to canonical proof-fact semantics.
- Normalized authority binding objects before comparing top-level `authority_binding` and `authority_proof.binding`.
- Added canonical authority binding byte encoding matching Rust/Go discriminators and length-prefix rules.
- Strengthened the v2 gate so Node proof-fact semantics cannot regress to shape-only validation.
