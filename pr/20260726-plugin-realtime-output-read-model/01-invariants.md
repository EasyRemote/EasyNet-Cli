# Invariants

- Plugin manifest declarations remain the only plugin-authored realtime input schema.
- Sidecar frames remain the only plugin runtime transport input schema.
- Realtime activation plans and reports are daemon-owned output projections.
- No plugin realtime/surface output read model type may derive `Deserialize`.
- No public behavior changes for CLI/UI rendering; serialization remains unchanged.
