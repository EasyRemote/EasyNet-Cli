# Invariants

1. SDK control discovery parsers reject unknown fields.
2. `pid`, `daemon_version`, `supported_ipc_versions`, and `capability_flags`
   are required discovery facts.
3. `pages_port` is optional, but if present it must be a positive TCP port.
4. Missing `invocation_endpoint` remains a `CONTROL_ONLY` runtime connector
   error, not a discovery parse repair.
5. Go and Python SDKs enforce the same wire contract.
6. The Rust daemon discovery comments must not instruct readers to fall back to
   historical Pages defaults.
