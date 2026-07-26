# API Contract

Public serialized fields are unchanged.

Removed internal capability:

- Deserializing `PluginRealtimeActivationPlan`
- Deserializing nested readiness/status projection structs/enums from `realtime.rs`
- Deserializing plugin surface report rows from `surface.rs`
- Deserializing realtime activation report/readiness rows from `broker.rs`

Retained capability:

- Serializing activation plans for CLI/UI/plugin host reports
- Serializing plugin surface and activation reports
- Constructing plans through `activation_plans_for_manifest`
