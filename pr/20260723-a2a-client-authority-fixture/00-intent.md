# Intent

Remove ambient catalog authority construction from A2A client tests.

The A2A client integration surface is a Device-hosted outbound adapter. Its
registration tests must bind metadata registration to an explicit Device
authority root rather than inheriting process-local daemon identity.
