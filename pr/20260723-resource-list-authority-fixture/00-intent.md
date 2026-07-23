# Intent

Remove ambient catalog construction from `meta.list_resources` registration
tests.

Resource discovery is a product-facing ability. Its registration test should
declare Device authority explicitly instead of relying on process-local daemon
identity.
