# Intent

Remove ambient catalog authority construction from `agent.invoke` tests.

`agent.invoke` exercises local registry dispatch and admission decisions. Its
tests need explicit Device authority, not process-local daemon identity.
