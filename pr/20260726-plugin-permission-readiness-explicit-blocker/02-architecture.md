# Architecture

`PluginPolicyBroker` owns permission readiness interpretation for plugin realtime activation.

The broker maps manifest-declared permission requirements plus available daemon/plugin abilities into a small output-only state machine:

1. No required permissions: `not_required`.
2. Status path present: `status_ability_available`.
3. Request path present: `request_ability_available`.
4. Required permissions with no action path: `action_unavailable`.

No product surface should infer this state from absent arrays or a generic unknown label.
