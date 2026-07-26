# Decisions Log

## 2026-07-26

- Preserve the existing raw `terminalReceipt()` accessor for Java source compatibility.
- Add `runtimeReceipt()` instead of changing the record component type; typed usage becomes available without forcing a product migration in this slice.
- Do not add Java-only self-hash canonicalization. Receipt canonical bytes remain an Axon runtime concern; Java validates mandatory proof facts and projections.
- Extend SPEC v2 to require the typed accessor and typed validation helper so the Java result DTO cannot regress to a map-only receipt surface.
