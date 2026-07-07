# Compatibility GetFile Alias Removal Plan

## Objective

Remove the Go Compatibility profile's duplicate `RetrieveFile` public facade
method so the SDK exposes only the SPEC-required latest method name:
`GetFile`.

## Current Defect

Section 20.7 defines the Go Compatibility file retrieval method as
`GetFile(ctx, CompatibilityFileRequest)`. The current Go facade also exposes
`RetrieveFile` as a direct alias. That creates semantic drift in the public
surface and weakens the "latest-only, no legacy aliases" SDK rule.

## Steps

1. Remove the `CompatibilityClient.RetrieveFile` alias.
2. Update compatibility tests to assert only the normative `GetFile` method.
3. Update Go conformance method/action maps to remove `RetrieveFile`.
4. Add scaffold rejection for the retired public method name.
5. Run Go tests, scaffold, and aggregate SDK gates.
