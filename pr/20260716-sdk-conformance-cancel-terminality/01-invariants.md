## Invariants

1. The conformance wrapper owns multi-language gate lifecycle.
2. A report validation failure is recoverable within the language loop; timeout,
   interrupt, or terminated child status is a terminal wrapper state.
3. Cancellation and timeout must not advance to later languages after a bounded
   child exits with terminal status.
4. The runner remains the owner of adapter execution semantics; this slice only
   changes wrapper lifecycle control.
5. Public SDK behavior and adapter report wire format remain unchanged.
