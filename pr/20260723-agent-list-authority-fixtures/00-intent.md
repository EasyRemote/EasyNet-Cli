# Intent

Remove ambient catalog construction from `agent.list` tests.

`agent.list` is a product-facing operational projection. Its tests should not
obtain catalog authority from local daemon identity when they only need to
assert registration and response projection.
