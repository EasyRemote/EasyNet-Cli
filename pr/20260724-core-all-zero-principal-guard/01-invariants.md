# Invariants

1. `00000000-0000-0000-0000-000000000000` is never a valid runtime principal fact.
2. Exact user-id checks and embedded URA placeholder checks are distinct APIs.
3. Product and transport layers may choose their error wording, but not their own sentinel semantics.
4. The guard is pure core identity logic and has no daemon, CLI, SDK, or product dependency.
