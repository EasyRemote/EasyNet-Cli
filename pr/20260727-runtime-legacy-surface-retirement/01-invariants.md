# Invariants

- SDK internals use generic runtime names and generic lifecycle concepts.
- Public invocation boundaries preserve the full seven-field tuple.
- Product-specific daemon, EasyNet, EasyRemote, device, Hub, plugin, or receipt
  ownership does not become part of canonical SDK concepts.
- Legacy input shapes are either rejected deterministically or translated at a
  versioned edge into the canonical descriptor-bound request.
