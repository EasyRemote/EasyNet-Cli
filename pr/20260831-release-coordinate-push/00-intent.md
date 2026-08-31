# Intent

## Goal

Provide one Runtime release-coordinate command that synchronizes the Runtime version, locks one exact released Axon coordinate, commits only generated release metadata, and optionally pushes the current release branch.

## Non-goals

- Do not change the independently versioned Python SDK or the private Node seam to the Runtime version.
- Do not publish Axon, Runtime, or SDK artifacts.
- Do not accept a dirty or unverified Axon coordinate.
