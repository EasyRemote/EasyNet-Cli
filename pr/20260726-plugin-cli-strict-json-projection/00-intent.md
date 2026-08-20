# Intent

Remove fallback rendering from the CLI plugin table projection.

After daemon plugin realtime/surface reports became output-only read models, the CLI correctly stopped deserializing daemon JSON into internal daemon types. The remaining risk is that table rendering silently substitutes `-`, `false`, `0`, or `unknown` when a daemon response is malformed. That is a compatibility layer in a different form: product output appears usable while canonical daemon report shape has drifted.

This iteration makes the CLI-owned JSON projection fail closed for required table fields.

