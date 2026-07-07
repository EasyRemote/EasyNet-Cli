# Intent: Node Authority Metadata Seam

Add a Node/TypeScript authority metadata seam that mirrors the generic
Go/Python SDK authority model over injected transports.

The slice exposes typed delegated-authority and session-authority metadata
projection, mutually-exclusive Invocation metadata attachment, and transport
delegation for minting authority metadata. It does not add daemon/C ABI
provider support and does not declare Node for the provider-backed
`authority/mutual_exclusion` conformance case.
