# Intent

Remove ambient catalog authority construction from OpenAI compatibility tests.

The OpenAI-compatible integration surface is a daemon-hosted compatibility
adapter. Its tests must bind metadata registration to an explicit authority root
rather than inheriting process-local daemon identity.
