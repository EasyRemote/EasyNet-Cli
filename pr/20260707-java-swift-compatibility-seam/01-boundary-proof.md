# Boundary Proof

## Ownership

The Compatibility profile belongs in the daemon SDK facade because it maps product-facing OpenAI-style requests onto generic daemon runtime carriers and projections. Backend HTTP authentication, API-key policy, quota, billing, multipart storage, HTTP response shaping, and SSE/WebSocket fanout remain product-owned.

## Invocation Completeness

Every Compatibility request carrier keeps `caller_ura`, `callee_ura`, `subject_ura`, `descriptor_version`, `nonce_base64`, and `causal_context` explicit before transport dispatch. Java and Swift clients only pass encoded request JSON to injected transports; they do not invent or hide Invocation fields.

## URA Discipline

The seam validates `easynet:///r/` URA fields for carrier identities, subjects, file/resource refs, and ability-backed model identifiers. The implementation introduces no alternate address spelling, obsolete input fields, endpoint identity, or Axon/protobuf dependency into Java or Swift packages.

## Product Boundary

The seam exposes Compatibility DTOs and client methods over injected transports. It does not own OpenAI HTTP routes, route auth, OpenAI schema as daemon protocol, product file storage, or streaming fanout. Those concerns remain downstream consumers of the SDK.

## Lifecycle

`CompatibilityClient` in both languages has a closed state. Operations after `close` fail deterministically with a typed SDK error. Transport failure is wrapped as retry-safe transport error unless already typed.
