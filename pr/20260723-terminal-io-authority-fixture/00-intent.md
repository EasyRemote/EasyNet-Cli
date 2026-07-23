# Intent

Remove ambient catalog authority construction from terminal I/O tests.

`terminal.input`, `terminal.read`, and `terminal.resize` are Device-hosted
control-plane abilities. Their registration tests must bind metadata catalogs
to an explicit Device authority root rather than inheriting process-local daemon
identity.
