# Invariants

- Any live `voice.*` descriptor and receipt is owned by the canonical realm Hub
  URA.
- Device-owned microphone and speaker resources remain separate Invocations;
  they are not aliases for Hub-owned voice descriptors.
- Call signaling RPC routes register only when a qualified realm-shared provider
  is assembled.
- `voice.subscribe` and `voice.transcribe` remain unsupported in the production
  live catalog until their media providers exist.
- Admission action and call geometry are descriptor facts and must not be
  inferred from ability names.
- There is no local daemon-state fallback for production voice call storage.
