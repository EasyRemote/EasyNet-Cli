# Invariants

- Device online/offline state is determined by the signed `session.open`
  carrier and its typed preludes.
- Credential/trust recovery must be represented as canonical invocation/prelude
  behavior, not as an advisory REST side effect.
- A failed trust or descriptor prelude must surface as a session error rather
  than being masked by a best-effort warmup.
- The session lifecycle has one phase tracker; no parallel warmup lifecycle may
  influence admission.
