# Verification

## Required checks

1. A registry-only projection succeeds with a valid registry and malformed
   hosted identity file.
2. Target production callers contain no direct `agent_registry::load_agents`
   access.
3. The architecture convergence gate rejects reintroduced direct reads.
4. Focused Rust tests and the convergence script pass.
