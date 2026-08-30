# Intent

Goal: prevent RemoteApp product completion from accepting cross-device RemoteApp reports that only expose target names, frame counts, and input policy mode.

Product completion must require each display/window/application cross-device scenario to summarize the same facts already validated by the child verifier: distinct caller/provider devices, remote target inventory, governed ability binding, provider capture binding, caller-rendered WebRTC media, input policy binding, and terminal receipt visibility.

Non-goals:
- Do not claim full RemoteApp product completion.
- Do not implement new native cross-device transport in this dirty checkout.
- Do not make the aggregate gate parse the full raw evidence payload.

Acceptance criteria:
- `remoteapp-cross-device-remoteapp-e2e.sh` passed reports include per-scenario `remoteapp_summary`.
- `remoteapp-product-completion-e2e.sh` validates `remoteapp_summary` for display, window, and application.
- Product-completion self-test rejects cross-device RemoteApp reports without summaries.
- Closure audit protects the aggregate requirement.
