# Decisions Log

## 2026-08-21

- Decided that the root fix is descriptor-owner callee resolution, not frontend/UI-specific fallback behavior.
- Kept `RuntimeDescriptorRefRequest.callee_ura` public field name for compatibility; added internal projection semantics instead of a breaking rename.
- Rejected merging the detached `refresh_remote_targets` operator exposure diff in this slice because it lacks full authority/frontend proof.
- Treated the first `check-sdk-cutover-readiness.sh` backend failure as a real downstream architecture seam, then fixed backend authority fixtures on a separate branch.
- Added Python catalogue descriptor provider projection parity after codegraph/rg showed Python still leaked Device target into descriptor-resolution transport where Go projected to SystemAgent.
