# Decisions Log

## 2026-07-05

- Selected Events device/invocation/history because the Python facade already exposes these operations but the C ABI contract only backs directory/session streams.
- Decided to model history as a bounded daemon read-model carrier/projection, not as local polling or backend fanout.
- Kept stream execution in Runtime Core and avoided adding product-specific event loops to Python.
