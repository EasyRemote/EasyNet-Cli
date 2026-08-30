# Invariants

1. Every scenario is a real Browser/backend/runtime session, never a component fixture.
2. Degraded-network and backpressure runs require explicit apply and reset commands.
3. Reset runs after success and from the exit trap after failure.
4. Commands are redacted from artifacts; only SHA-256 fingerprints are retained.
5. The canonical aggregator rejects Resource, media-pipeline, codec, or transport drift.
6. Scenario terminal lifecycle remains owned by the normal RemoteApp abilities.
