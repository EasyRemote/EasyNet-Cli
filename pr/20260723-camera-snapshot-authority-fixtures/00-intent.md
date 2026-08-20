# Intent

Remove ambient catalog authority construction from camera media tests.

`camera.snapshot`, `camera.subscribe`, and camera recording abilities are
Device-hosted resource abilities. Tests must bind metadata and runtime-backed
catalogs to an explicit Device authority root rather than inheriting
process-local daemon identity.
