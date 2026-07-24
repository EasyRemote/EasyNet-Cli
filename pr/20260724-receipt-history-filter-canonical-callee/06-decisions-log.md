# Decisions Log

## 2026-07-24

- Treat `filter.agent_ura` in receipt history as a compatibility alias because the canonical invocation ledger stores `callee_ura`, not product directory agent state.
- Keep any CLI operator spelling as facade-only lowering to avoid breaking CLI muscle memory while removing the daemon/SDK protocol alias.
- Update the SPEC v2 gate with the new invariant so it rejects daemon-side `agent_ura` receipt-history filter support instead of preserving the previous compatibility rule.
- Regenerate SDK public API inventory and parity matrix after removing the Node-only `ReceiptFilter.agentURA` surface.
