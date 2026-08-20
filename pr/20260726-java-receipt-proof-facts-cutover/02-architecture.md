# Architecture

## Boundary

`RuntimeReceiptProofFacts` owns Java proof-fact validation. Receipt projection must depend on this validator rather than duplicating local checks or accepting unvalidated maps.

## Shared runtime model

The Java SDK is a canonical runtime SDK implementation. Its proof-fact acceptance rules must match the Go and Python SDKs:

- mandatory descriptor facts
- mandatory runtime identity facts
- mandatory authority binding facts
- strict profile only

## Result

Java receipt projection becomes provider-backed/fail-closed instead of a shape-only seam.
