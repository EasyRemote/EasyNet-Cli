# Invariants

- One application Resource represents one live application process/bundle window set, not one application-display pair.
- Every captured native window ID is present in the committed `AppWindowSetProof` and owner-matches the application identity.
- Window identity and ordered surface layout are separate proofs. Geometry or Z-order drift rebuilds media even when the window-set identity epoch is unchanged.
- No display capture, crop, mask, or unrelated same-app/uncommitted window is used as fallback.
- Multi-window composition is hard-bounded by window count, pixels, callback rate, and one retained frame per surface; an unchanged window's last frame remains valid and cannot freeze updates from other windows.
- `resolution=native` applies ScreenCaptureKit point-to-pixel scale instead of treating macOS logical points as physical pixels.
- Gaps are deterministic black and cannot expose host display pixels.
- Target-local pointer input may land only on the current topmost committed target window; black gaps and foreign-window occlusion fail closed before OS injection.
- The composed frame remains one canonical WebRTC video track; Invocation, receipt, sequence, consent, and terminal lifecycle are unchanged.
- Rebind prestarts a muted complete capture plan, pauses both generations around Runtime state commit, and selects only the committed generation; stale commit resumes the old generation without media drift.
