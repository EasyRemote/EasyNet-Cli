# Intent

Harden the daemon-owned MCP edge boundary without changing public behavior.

MCP is an EasyNet daemon product adapter: it projects daemon AbilityDescriptors
as MCP tools and routes MCP tool calls back through daemon invocation. It must
not become a second transport owner for stream or bidi capability lifecycles.

This slice pins two use-case boundaries:

- MCP tool listing and calls expose only unary RPC descriptors.
- MCP stdio frame ingestion rejects oversized EOF-delimited frames while keeping
  retained bytes bounded.
