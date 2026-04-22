# `docs/open-questions/` — Index

Questions we've named but deliberately not decided. Each file states what would move the question into a decision, and logs the revisit trigger (trigger-based, not calendar-based, unless otherwise noted).

## Active

- **[retire-a2a-agents-json-label.md](retire-a2a-agents-json-label.md)** — when Axon's AXIOM §6.2 discovery agent becomes implementable, the `a2a.agents_json` node roster label retires in favor of protocol-level publish. Two trigger conditions; retirement PR shape sketched.
- **[axon-invocation-receipt-link.md](axon-invocation-receipt-link.md)** — whether CLI-side run artefacts (mission run dir, AgentSession timeline) should link to Axon `invocation::Receipt.id`. Revisit at PR-7 merge + 30 days.
- **[does-easynet-need-a-terminal-ability.md](does-easynet-need-a-terminal-ability.md)** — a terminal/shell ability was listed in earlier plans without a documented consumer. No current customer → no plan item.
- **[does-easynet-need-a-worktree-ability.md](does-easynet-need-a-worktree-ability.md)** — same situation for git worktrees.
- **[does-easynet-need-a-local-ws-control-plane.md](does-easynet-need-a-local-ws-control-plane.md)** — former PR-6's WS server / bearer auth / permit interactive flow. No local client exists; Tier A (tokio + session ownership) absorbed into PR-7.
- **[cli-dispatch-as-first-class-invocation.md](cli-dispatch-as-first-class-invocation.md)** — whether CLI dispatch migrates from "RPC with audit trail" to AXIOM §5 signed `InvocationEnvelope` + receipts. Blocked on three upstream AXIOM artefacts (URA namespace, DEFAULT_PROFILE.md, discovery agent) AND a concrete consumer need.
- **[skill-marketplace-integration.md](skill-marketplace-integration.md)** — searching / browsing / installing skills from OpenSkill or other upstream marketplaces via Frontend + CLI. Customer has surfaced; four open design decisions (marketplace protocol, manifest format, install-target semantics, Frontend↔CLI wire) block coding.

## Discipline

Open questions are **not** the same as 不排期堆积. An item on this list has a named question and a rule for what would resolve it. A bullet on a planning wishlist does not. The distinction is the discipline: a question on this list has been audited for ground; a wishlist bullet survives on list inertia.
