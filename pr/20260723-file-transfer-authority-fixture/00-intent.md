# Intent

Remove ambient catalog authority construction from file transfer registration
tests.

`file_transfer.*` abilities are Device-hosted control/data plane surfaces. Their
registration fixture must bind metadata registration to an explicit Device
authority root rather than inheriting process-local daemon identity.
