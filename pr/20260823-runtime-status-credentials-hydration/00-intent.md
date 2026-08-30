# Intent — Runtime Status Credential Context Hydration

RemoteApp product-flow evidence depends on an upstream Hub API readiness gate.
That gate reads `easynet runtime status --json`; if an old
`connection-state.json` snapshot lacks `hub_api_endpoint`, the product-flow
harness cannot verify Hub API health even when current credentials still carry
the API base.

This change keeps the connection-state object authoritative for lifecycle
state/failure, but hydrates missing endpoint context from current credentials
when the persisted snapshot belongs to the same device and realm.

The scope is runtime status diagnosability and product-flow readiness only. It
does not start the Hub, repair credentials, or claim RemoteApp product
completion.
