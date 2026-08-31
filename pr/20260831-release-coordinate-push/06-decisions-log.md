# Decisions log

## 2026-08-31 — Runtime version inventory is explicit

The private Node seam and separately released Python SDK are not Runtime manifests. Broad `package.json` discovery is removed from Runtime version mutation.

## 2026-08-31 — Go development replacement belongs to `go.work`

The published Go module remains release-shaped with a real Axon version. The repository workspace owns the sibling checkout replacement for local development.
