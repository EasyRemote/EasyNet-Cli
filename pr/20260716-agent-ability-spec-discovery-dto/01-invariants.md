# Invariants

1. Ability manifests remain the canonical source for `input_schema`.
2. `AgentAbilitySpec` may not become a second schema carrier beside manifests.
3. Manifest-backed abilities with non-object input schemas are still rejected
   before they become visible through `abilities_for`.
4. Prompt/hint formatting remains name/description-only.
5. Public daemon behavior and wire descriptors remain unchanged.
