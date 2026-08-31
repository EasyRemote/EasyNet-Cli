# Invariants

1. Runtime, Python SDK, and private Node seam retain distinct version ownership.
2. Axon dependencies in Cargo, Python, and Go resolve to the same Axon contract version.
3. `axon.lock.json` is generated from an exact clean Axon commit, never handwritten independently.
4. The caller checkout is unchanged until an isolated metadata commit passes verification.
5. Push is non-forced and cannot target `main` or `master`.
