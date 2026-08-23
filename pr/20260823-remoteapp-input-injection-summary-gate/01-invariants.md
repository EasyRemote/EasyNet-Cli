# Invariants

- Input injection must be permission-backed, not policy-only.
- Pointer and keyboard inputs must both be applied.
- Applied input sequence numbers must be monotonic and stale sequences must be rejected.
- Latency summaries must stay inside the verifier-declared threshold.
- OS effects must be observed independently from the injector and bound to selected Resource URA, session id, geometry revision, and focus epoch.
- Terminal receipt visibility remains required.
- Product-completion aggregation validates summaries only; deep raw evidence remains owned by the input verifier.
