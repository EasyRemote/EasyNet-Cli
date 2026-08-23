# Invariants

- Cross-device RemoteApp evidence must use distinct caller/provider Device URAs.
- Each target scenario must be bound to a canonical selected Resource URA and non-empty session id.
- Remote target inventory must be observed before treating capture as product evidence.
- Public RemoteApp session abilities must bind the selected Resource URA and session.
- Capture must happen on the provider device and media must render on the caller device.
- Input policy must be checked and session-bound even when view-only or blocked.
- Terminal receipt must be visible and session-bound.
- Product completion validates summaries only; raw evidence validation remains owned by the cross-device verifier.
