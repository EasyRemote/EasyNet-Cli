# Authority Metadata Facade Plan

## Goal

Expose SDK-owned typed authority metadata for Go and Python without moving
Axon authority canonicalization, signing, or verification into language
facades.

## Boundary Proof

- Axon/daemon remains the authority for canonical payload bytes, signatures,
  trust-anchor lookup, scope/audience matching, and admission decisions.
- Go/Python SDKs may decode the daemon/Axon metadata envelope into typed DTOs
  and attach exactly one authority envelope to an Invocation draft.
- Product callers should stop hand-writing `x-easynet-delegation` and
  `x-easynet-session-authority`; they should use SDK metadata objects.
- The SDK must reject ambiguous authority metadata before dispatch.
- Public Go/Python surfaces must not import Axon SDKs or expose generated Axon
  protobufs.

## Invariants

1. One Invocation metadata map may carry either delegated authority or session
   authority, never both with non-empty values.
2. Authority DTOs must expose issuer/subject/audience-or-audiences/scopes,
   issued/expires timestamps, and signature bytes.
3. Authority metadata values remain opaque daemon/Axon wire strings; SDK
   projection does not recompute canonical bytes.
4. Builder attachment must preserve unrelated metadata and must fail
   deterministically on ambiguity.
5. Go and Python expose equivalent authority projection and builder semantics.

## Implementation Steps

1. Add Go authority DTOs and metadata helpers.
2. Add Go InvocationBuilder authority attachment and ambiguity validation.
3. Add Python authority DTOs and metadata helpers.
4. Add Python InvocationBuilder authority attachment and ambiguity validation.
5. Run Go/Python tests and SDK parity gates.

## Remaining After This Slice

- Authority minting through daemon/Axon-owned transport remains to be added.
- EasyNet backend still needs source cutover from raw Axon authority builders to
  the SDK facade.
- RFC-007 receipt URA construction and product repository cutover gates remain
  outside this slice.
