# Intent

Remove ambient catalog construction from `*.chat` registration tests.

Agent chat is a product-facing ability. Its route registration tests should
declare the Device authority under test instead of inheriting authority from
process-local daemon identity.
