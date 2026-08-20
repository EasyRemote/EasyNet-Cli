# Intent

## Goal

Align SDK public documentation with the canonical runtime model by replacing
daemon/product compatibility vocabulary with runtime-host and downstream-product
boundary language.

## Non-goals

- Do not change public code symbols.
- Do not change runtime behavior.
- Do not hide explicitly retired downstream product surfaces.

## Acceptance criteria

- Go/Python/Node/Java/Swift SDK docs describe runtime-host/provider-neutral
  concepts.
- Java and Swift docs no longer call themselves daemon SDKs.
- Product helper lists use generic downstream-surface wording instead of
  compatibility/client taxonomy.
- Existing conformance gates remain green.
