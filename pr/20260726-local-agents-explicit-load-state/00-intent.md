# Intent

Remove hidden missing-file defaulting from hosted-agent identity projection
persistence.

`local-agents.json` is identity authority for hosted Agents. Missing storage is
a first-boot state, not an empty identity registry produced by the storage
reader.
