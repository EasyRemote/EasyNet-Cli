# Architecture

Layering:

1. Frontend product-flow harness runs frontend, browser lifecycle, cross-device smoke, and host verifier steps.
2. The harness emits `frontend_flow_summary` as a compact product journey summary.
3. Product-completion gate validates the summary before considering the frontend product-flow report complete.

This keeps frontend product journey evidence visible without letting it replace lower-layer RemoteApp verifier ownership.
