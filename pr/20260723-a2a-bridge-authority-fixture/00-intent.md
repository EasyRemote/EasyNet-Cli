# Intent

Remove ambient catalog authority construction from A2A bridge tests.

The A2A bridge integration surface is the inbound counterpart to outbound A2A.
Its registration fixture must bind metadata registration to an explicit Device
authority root rather than inheriting process-local daemon identity.
