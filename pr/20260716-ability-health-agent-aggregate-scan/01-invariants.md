# Invariants

- A health scan must derive registry rows and hosted Agent URAs from one aggregate snapshot.
- Health metadata is advisory only and must not change invocation acceptance.
- Missing or ambiguous hosted LLM identity must not produce a canonical ability URA for health records.
- Registry-load and identity-load failure context must remain source-classified for operator diagnostics.
- Existing health scheduling, backoff, boot cooldown, and record retention semantics remain unchanged.
