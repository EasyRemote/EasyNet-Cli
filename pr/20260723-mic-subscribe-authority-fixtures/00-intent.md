# Intent

Remove ambient catalog authority construction from `mic.subscribe` tests.

`mic.subscribe` is a Device-hosted media stream ability. Its tests must bind
metadata and runtime-backed catalogs to an explicit Device authority root rather
than inheriting process-local daemon identity.
