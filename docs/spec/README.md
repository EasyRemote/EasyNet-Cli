# `docs/spec/` — Index

Specs that bind future code. A reader entering this directory should start here.

## Active

- **[daemon-sdk-requirements-v1.md](daemon-sdk-requirements-v1.md)** — Product-facing Daemon SDK requirements for `easynet-daemon`: supported languages, Axon-style project structure, complete OOP object model, daemon/client/invocation/stream/bidi/directory/fan-out state machines, complete Invocation, listing complexity bounds, facade fan-out bans, typed errors, EasyRemote relationship, and EasyNet backend cutover constraints.
- **[node-roster-label-v2.md](node-roster-label-v2.md)** — Format of the `a2a.agents_json` node-level discovery hint attached to Axon node registration. Strictly a rendering hint for the EasyNet Frontend's agents page. **Not** an Agent-layer publish. Retirement path tracked in [`../open-questions/retire-a2a-agents-json-label.md`](../open-questions/retire-a2a-agents-json-label.md).

## Superseded — do not build on these

These files are retained (not `git rm`'d) so the decision history remains auditable. Each file carries a Status banner at the top explaining what replaced it and why. **A reader doing new work should not reference them.**

- **[agent-publish-mechanism.md](agent-publish-mechanism.md)** — Conflated three layers (node labels / capability-package publish / Axon Tier-2 discovery agent publish) into a single "hybrid A+B" decision. None of the three was actually an Agent-layer publish in the AXIOM §6.2 sense. Replaced by `node-roster-label-v2.md` (for the label part) + `../open-questions/retire-a2a-agents-json-label.md` (for the discovery-agent part, deferred pending Axon's `DEFAULT_PROFILE.md`).
- **[publish-json-format.md](publish-json-format.md)** — Designed a `publish.json` state machine (`pending | published | failed | partial`) for a dual-write publish operation that no longer exists under the corrected scope. Replaced by nothing — the node roster label is derivable from `~/.easynet/agents.json` on every register_node call and needs no local state.

## Why keep superseded specs readable

Deleting them would hide the reasoning that led to the correction. A reader two years from now should be able to answer "why didn't we do X" by reading the file that once said "we will do X" and its supersession banner explaining why we changed our mind. The git log records the *change*; the file records the *decision*.
