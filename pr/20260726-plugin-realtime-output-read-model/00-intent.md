# Intent

Remove the plugin realtime activation and surface read models from the set of accepted input DTOs.

The plugin runtime already has strict manifest and sidecar frame schemas. The realtime activation plan is different: it is daemon-produced state that tells UI/CLI consumers which declared realtime capabilities can be activated. Keeping `Deserialize` on that read model creates an unnecessary compatibility surface where product code could later treat daemon output as accepted input and silently preserve unknown legacy fields.

This iteration makes the boundary explicit: plugin realtime activation plans, plugin surface reports, and realtime activation reports are output-only projections.
