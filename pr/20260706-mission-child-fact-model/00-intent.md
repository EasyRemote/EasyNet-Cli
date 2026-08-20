Mission child Invocation fact model

Intent:
- Converge the Go and Python Mission facades on one SDK-owned projection model for daemon child Invocation facts.
- Preserve the SPEC boundary: SDKs submit and observe daemon-owned Mission/EAL, but do not execute Mission DSL, scheduler policy, retry policy, or receipt construction.
- Make child Invocation conformance stricter without changing `docs/spec/daemon-sdk-requirements-v1.md`.

Target SPEC clauses:
- Mission/EAL helpers create child Invocations for ability calls rather than redefining transport.
- Mission status exposes parent receipt URA, child receipt refs, and output refs.
- Product DSLs compile to EAL but call the Mission profile for daemon submission and observation.
