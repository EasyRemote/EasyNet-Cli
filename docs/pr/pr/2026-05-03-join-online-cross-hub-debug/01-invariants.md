# Invariants

1. Canonical device identity is `easynet:///r/<realm>/device/<node>`.
2. Legacy `easynet:///r/<realm>/agent/<bare-node>` is accepted only as a migration input and is normalized exactly once at a boundary.
3. Real hosted-agent URIs (`/agent/<user>.<agent>`) must never be rewritten into device URIs.
4. `[daemon].hub_endpoint` is the device-to-hub `<self>.session` dial target, never the hub-to-hub TLS endpoint.
5. Presence lookup, directory projection, and direct probe surfaces must all key the same device URI for the same node.
