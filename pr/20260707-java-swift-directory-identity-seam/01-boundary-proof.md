# Boundary Proof

## Ownership

Directory + Identity belongs to the EasyNet-Cli daemon SDK facade as a generic
runtime profile. Java and Swift may expose public DTOs and clients, but the
transport owns descriptor-ref projection, directory resolution, and daemon
read-model facts.

## Invariants

- Java and Swift Directory + Identity clients consume injected transports only.
- DescriptorRef and URA projection are delegated to transport responses; the
  language seams do not parse or synthesize Axon canonical grammar.
- Directory list page bounds are enforced before transport dispatch.
- Directory/Identity DTOs use generic runtime names only.
- Transport failures become typed SDK transport errors.
- Malformed payloads become deterministic validation errors.
- Closed clients reject calls deterministically.
- The seam exposes no raw Axon, protobuf, daemon provider, backend, or
  EasyRemote-specific types.

## Compatibility

The change is additive for P1 packages. Existing Runtime Core, Health, stream,
bidi, and invocation APIs are unchanged.
