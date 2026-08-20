# Intent

Remove ambient catalog authority construction from `screen.snapshot` and
`screen.subscribe` tests.

Screen media abilities are Device-hosted resource abilities. Tests must bind
metadata and runtime-backed catalogs to an explicit Device authority root rather
than inheriting process-local daemon identity.
