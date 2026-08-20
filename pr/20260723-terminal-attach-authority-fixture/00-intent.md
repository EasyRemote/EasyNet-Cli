# Intent

Remove ambient catalog authority construction from terminal attach tests.

`terminal.attach` is a Device-hosted BIDI data-plane ability. Its registration
test must bind the metadata catalog to an explicit Device authority root rather
than inheriting process-local daemon identity.
