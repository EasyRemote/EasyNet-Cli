# Intent

Remove ambient catalog authority construction from media ability metadata tests.

The media ability table describes Device-hosted physical media capabilities and
Hub voice seams. Tests must bind the metadata catalog to an explicit Device
authority root rather than inheriting process-local daemon identity.
