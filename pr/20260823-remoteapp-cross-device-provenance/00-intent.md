# Intent — Cross-device smoke provenance

The RemoteApp cross-device smoke can reuse a prebuilt runtime image. Without provenance, a failed report may come from an image that predates the current source tree and should not be used as evidence against the current implementation.

Add source/runtime provenance to the smoke report so product-readiness evidence can distinguish current-source runs from stale-image runs.
