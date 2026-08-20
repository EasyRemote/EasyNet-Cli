# Decisions Log

## Warning Channel

The SPEC requires an operator warning when online and offline observations disagree, while also requiring identical JSON schemas. The warning therefore stays on stderr through `output::warn`; stdout remains the canonical `DesktopCompanionStatus` DTO.

## Local Observation Wins

When daemon-local control ability output differs from the freshly computed local manager status, the CLI returns the local status. This preserves the SPEC's source-of-truth rule for stale daemon plugin state without adding compatibility fields.
