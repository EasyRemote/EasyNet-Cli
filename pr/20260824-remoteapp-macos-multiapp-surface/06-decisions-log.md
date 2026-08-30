# Decisions log

- Use one desktop-independent ScreenCaptureKit stream per committed window instead of display capture plus crop.
- Keep one composed video track so the frontend and WebRTC session contract do not become a second multi-track lifecycle protocol.
- Capture application audio from one designated application surface to avoid duplicate audio packets across window streams.
- Replace a full capture plan during rebind; do not mutate window filters piecemeal and risk mixed identity epochs.
- Keep `AppWindowSetProof` for identity and add `AppSurfaceLayoutProof` for ordered geometry; conflating them would rotate identity on every move and still fail to detect pure Z-order changes.
- Validate target-local pointer hit-testing against the fresh global front-to-back host list rather than only checking union bounds, because union gaps and foreign occlusion are not safe injection targets.
