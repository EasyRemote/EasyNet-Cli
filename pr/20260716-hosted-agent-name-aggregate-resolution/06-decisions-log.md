# Decisions Log

## 2026-07-16

- Selected hosted-Agent display-name resolution because multiple surfaces repeat local identity file loading and ambiguity handling.
- Deferred full `meta.teach` migration because that path combines Agent registry rows, hosted identity authorization, teach grant persistence, and workspace mutation; it should converge as a transaction/state-machine slice.
- Kept public command and Invocation shapes unchanged; only the internal read owner changes from local file helpers to the Agent aggregate projection.
- Preserved surface-specific error wording while centralizing ambiguity, malformed URA, and non-Agent URA classification in `HostedAgentNameLookupError`.
