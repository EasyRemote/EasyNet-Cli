# Intent

Remove ambient catalog authority construction from `agent.discover` tests.

`agent.discover` exercises Agent-owned discovery behavior through a
Device-hosted catalog. Tests must declare the hosting Device authority
explicitly instead of inheriting process-local daemon identity.
