# Invariants

1. Adapter reports must reference files inside the repository root.
2. Each evidence `sha256` must equal the current bytes of its `ref_path`.
3. A report refresh may not invent or move evidence; it only updates derived
   digests for existing evidence paths.
4. The failing `daemon/permission_denied` case must continue to point at the
   Rust daemon invocation service test that asserts permission-denied behavior.
5. Generated live-result artifacts under `target/` are not committed.
