# Invariants

- SDK internals use generic runtime names and generic lifecycle concepts.
- Public invocation boundaries preserve the full seven-field tuple.
- Product-specific daemon, EasyNet, EasyRemote, device, Hub, plugin, or receipt
  ownership does not become part of canonical SDK concepts.
- Legacy input shapes are either rejected deterministically or translated at a
  versioned edge into the canonical descriptor-bound request.
- Agent registry key projections are authoritative inputs to admission,
  Mission target conflict detection, and skill ownership discovery. A malformed
  registry key is corrupt aggregate state and must fail closed; product paths
  must not silently drop the bad row and continue with a partial projection.
