# Java Async Runtime Seam Plan

## Goal

Add Java/JVM idiomatic async adaptation to the Runtime Core seam while preserving the single injected-transport runtime model.

## Scope

- Add `AsyncRuntimeClient` over the existing `RuntimeClient` and injected `RuntimeTransport`.
- Add `RuntimeFuture<T>` as a `CompletableFuture<T>` subclass with observable cancellation.
- Make `StreamHandle` and `BidiSession` implement Java `Iterator` over retained lifecycle state without adding a second stream model.
- Extend Java seam tests and docs to cover async invocation and cancellation.

## Non-Scope

- No provider-backed daemon, JNI, or C ABI transport.
- No Java profile clients beyond Runtime Core.
- No reimplemented protocol signing, receipt, or admission logic.
