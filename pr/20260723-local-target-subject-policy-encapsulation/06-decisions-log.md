# Decisions Log

1. `LocalAbilityTarget` remains a route value object. It no longer stores or exposes subject state.
2. Descriptor-derived daemon-system subject policy is centralized in runtime issuer helpers.
3. `root_context_for_target` delegates nonce minting and context construction to `root_context`; freshness remains owned by one named issuer.
4. MCP and A2A bridge adapters no longer assemble target subject facts; they request `SystemInvocationTargetIssuer::local_root_for_target`.
5. Architecture gates now reject production reintroduction of `default_subject_ura` and require the target-bound issuer helpers plus subject regression tests.
