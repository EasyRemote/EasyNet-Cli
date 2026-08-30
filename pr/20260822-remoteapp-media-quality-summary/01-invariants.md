# Invariants — RemoteApp Media Quality Summary Gate

- The frontend quality summary must consume daemon/browser stats, not invent
  another media state machine.
- Audio/video adaptation remains partial until real codec negotiation,
  audio-path, soak, and degraded-network E2E reports exist.
- The product matrix must remain incomplete.
