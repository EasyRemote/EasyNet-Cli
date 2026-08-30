# Intent

Goal: prevent RemoteApp product completion from accepting frontend product-flow reports that only expose generic step pass/fail data.

Product completion must require an explicit `frontend_flow_summary` showing the browser/frontend path covered runtime readiness, UI flow, Browser/Tauri lifecycle, distinct-device smoke, permission, target selection, window/application rendering, view-only input control policy, and end/cleanup lifecycle evidence.

Non-goals:
- Do not claim full RemoteApp product completion.
- Do not replace daemon/plugin/media/input verifiers with frontend evidence.
- Do not run live frontend/browser infrastructure in this checkout.

Acceptance criteria:
- `frontend-remoteapp-product-flow-e2e.sh` reports include `frontend_flow_summary`.
- `remoteapp-product-completion-e2e.sh` validates the summary for both window and application product flow.
- Product-completion self-test rejects frontend product-flow reports without summaries.
- Closure audit and frontend harness tests protect the summary contract.
