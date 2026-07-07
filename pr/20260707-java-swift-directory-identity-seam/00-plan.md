# Java and Swift Directory Identity Seam Plan

## Goal

Converge Java and Swift one step closer to the shared SDK runtime model by
adding Directory + Identity seam DTOs and clients over injected transports.

## Scope

- Add Java Directory + Identity request/response DTOs, clients, and injected
  transports.
- Add Swift Directory + Identity request/response DTOs, clients, and injected
  transports.
- Exercise descriptor-ref projection, directory resolution, bounded list pages,
  transport failure mapping, malformed payload handling, and closed-client
  behavior in existing seam tests.
- Register Java and Swift action-adapter evidence for the shared
  Directory + Identity conformance cases.
- Update scaffold guards and status text.

## Non-Goals

- No Java JNI or daemon provider.
- No Swift C ABI or daemon provider.
- No product-specific directory model.
- No local URA grammar reimplementation beyond shape validation needed to keep
  requests non-empty and transport-owned.
- No backend, EasyRemote, or product cutover claim.
