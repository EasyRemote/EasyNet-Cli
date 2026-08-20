# Intent

Remove ambient catalog authority construction from terminal lifecycle tests.

`terminal.create`, `terminal.list`, and `terminal.close` are Device-hosted
control-plane abilities. Their registration tests must bind metadata catalogs
to an explicit Device authority root rather than inheriting process-local daemon
identity.
