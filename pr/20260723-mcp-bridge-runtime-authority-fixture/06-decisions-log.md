# Decisions Log

- Decision: preserve runtime-backed coverage and replace only the ambient
  authority constructor.
  Rationale: MCP bridge tests validate dynamic runtime registration and handler
  lookup; metadata-only coverage would be weaker and misaligned.
