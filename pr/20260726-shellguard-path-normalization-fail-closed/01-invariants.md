Invariants
==========

- A write redirect target must either normalize to a concrete absolute path or
  fail closed.
- The path constraint stage must not fabricate a replacement target.
- Rejection remains deterministic and inspectable by downstream shell.run
  diagnostics.
- No filesystem access is performed during string-level normalization.
