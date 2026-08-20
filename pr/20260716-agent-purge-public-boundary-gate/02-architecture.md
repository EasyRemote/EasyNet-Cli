# Architecture

The boundary owner is `src/daemon/ability/builtins/agents/lifecycle.rs`.
Catalog metadata projects that boundary to public descriptors through
`src/daemon/ability/catalog/catalog_metadata.rs`.

The gate belongs in `tools/scripts/check-architecture-convergence.sh` because
the risk is architectural drift: public descriptor semantics and lifecycle
handler semantics can regress independently while unit tests still cover only
one side.

CodeGraph inspection:

- `codegraph status .` reported an up-to-date index with 934 files, 32,472
  nodes, and 120,726 edges.
- `codegraph explore "agent.stop agent.purge destructive lifecycle root removal descriptor catalog metadata"`
  identified the purge lifecycle owner, the durable purge FSM in
  `src/daemon/persistence/agent_lifecycle.rs`, and descriptor placement through
  `src/daemon/ability/catalog/descriptor_paths.rs`.
