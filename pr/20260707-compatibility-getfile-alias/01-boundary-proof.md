# Boundary Proof

## Ownership

The Compatibility profile owns SDK-side carrier/projection methods for
OpenAI-compatible product APIs. The SPEC-required SDK method for file retrieval
is `GetFile`; product HTTP route names or external protocol verbs do not justify
a second Go SDK method alias.

## Invariants

1. `GetFile` remains the only public Go SDK compatibility file retrieval
   method.
2. Transport-level `BuildFileRetrieveInvocation` remains because it names the
   governed daemon compatibility operation, not a facade alias.
3. Compatibility methods still lower to complete Invocation carriers through
   daemon-owned transports.
4. No URI terminology or legacy input alias is introduced.
5. Scaffold checks reject reintroduction of `RetrieveFile`.

## Rejected Designs

- Keeping `RetrieveFile` for external API familiarity: rejected because the SDK
  is the canonical runtime model, not an OpenAI client library.
- Deprecating the alias without removing it: rejected because the goal requires
  latest-only surfaces with no compatibility aliases unless explicitly required
  by the SPEC.
