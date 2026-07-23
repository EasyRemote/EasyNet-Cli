# API Contract

No public method signatures change.

Valid current daemon `control.json` files continue to parse. Malformed or old
files with missing lifecycle/version facts, unknown fields, or `pages_port: 0`
now fail at discovery decode with SDK errors instead of being normalized to
empty defaults.
