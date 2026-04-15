# AAL Contracts (Producer Side)

These files are intentionally contract-like, not executable source yet.

- They define agent class boundaries from `docs/easynet_ontology.tex`.
- Public `ability` methods are the only cross-agent surface.
- Private `skill` fields are internal and never externally invokable.
- Internal evolution happens in memory/workflow graph sections while public signatures stay stable.

When BCC lands, this folder can be used as migration input for real AAL compilation.
