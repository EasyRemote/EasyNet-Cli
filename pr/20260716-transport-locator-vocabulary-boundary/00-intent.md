# Intent

Close the transport-locator vocabulary fork inside SDK gate scripts.

Retired identifier fixtures still need tokens such as `AgentURI` or
`target_node_uri` so the guards can prove they fail. The gate narratives,
temporary variable names, and fixture descriptions must not describe the
architecture as a retired address-token migration because the canonical model
has URA for semantic identities and separate transport locators for HTTP, gRPC,
or local endpoints.

Expected effect: architecture convergence. Negative tests continue to exercise
retired address-token identifiers, while human-facing gate output uses the
canonical vocabulary.
