# Decisions Log

## 2026-07-16

- Selected `agents.chat` hot-added discover/invoke providers and peer-skill enumeration because they are invocation-facing reads still bypassing the Agent aggregate owner.
- Kept discover and invoke provider signatures unchanged in this slice. The root fork is the persistence read owner, not the public per-agent ability contract.
- Preserved chat peer-skill degraded behavior because the hint list is advisory and should not fail an otherwise valid chat turn.
- Added a narrow convergence rule for `agents/chat.rs` instead of globally banning direct registry reads, because catalog/bootstrap and other inventory surfaces are separate migration slices.
