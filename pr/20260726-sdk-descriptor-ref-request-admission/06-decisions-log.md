# Decisions Log

2026-07-26:

- Keep generic descriptor catalogue lookup available, but make explicit provider-backed descriptor resolution identity-complete at the SDK client seam.
- Implement Go and Python together to avoid language-specific architecture drift.
- Treat missing `callee_ura`, `ability`, and `call_mode` as local SDK admission failures before provider transport.
- Reject all-zero principal placeholders only for explicit provider-backed descriptor requests at this lower RuntimeClient seam; higher-level invocation builders already reject all-zero caller/callee/subject for complete invocation tuples.
- Move `~/.easynet` aside instead of deleting it because the execution environment rejects `rm -rf`; the runtime still observes a clean state.
