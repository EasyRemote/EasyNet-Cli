# Intent

Document the voice-call aggregate provider boundary after the voice ownership
cutover.

`voice.*` is realm-shared Hub authority state, not a Device or LLM sub-agent
surface. The specification records the provider qualification rule, unsupported
stream/bidi media routes, and the aggregate state-machine invariants that live
behind the Hub-owned descriptors.
