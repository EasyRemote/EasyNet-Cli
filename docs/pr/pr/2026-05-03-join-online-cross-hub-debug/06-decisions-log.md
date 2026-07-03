# Decisions Log

- 2026-05-03: Treat the main failure as a URI-shape consistency bug across join, presence lookup, and monitor surfaces, not as an independent liveness-protocol failure.
- 2026-05-03: Keep `agent_uri` as the JSON field name on compatibility surfaces; change only the carried URI value.
- 2026-05-03: Normalize legacy `/agent/<bare-node>` only at ingress boundaries and keep real hosted-agent URIs strict.
- 2026-05-03: Preserve compatibility for legacy callers in production code, but move test fixtures and assertions to canonical `/device/<node>` so regressions are detected against the current contract, not the migration shim.
