# Invariants

1. Local transport must reach a deterministic terminal state for every request.
2. Session attach / bidi lifecycle must preserve exactly one terminal closure per session id.
3. Windows transport selection must be explicit and auditable through discovery/config, never heuristic at call sites.
4. A Windows daemon restart must not leave an ambiguous half-live IPC endpoint behind.
5. Unsupported paths must fail typed and early; supported paths must not silently downgrade to a different semantic transport.
