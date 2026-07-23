# Intent

Remove ambient catalog authority construction from MCP client tests.

MCP client integration abilities are Device-hosted integration surfaces. Their
tests must bind metadata registration to an explicit Device authority root
rather than inheriting process-local daemon identity.
