# Intent

Remove ambient metadata catalog authority construction from the Agent lifecycle
registration smoke test.

The registration test only needs catalog metadata to prove that lifecycle
abilities are wired. It must use an explicit Device authority root instead of
the broad metadata-only constructor.
