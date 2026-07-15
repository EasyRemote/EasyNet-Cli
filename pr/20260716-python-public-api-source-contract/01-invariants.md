# Invariants

- Public Python SDK classes must not declare the same field or method name more
  than once.
- The AST inventory is the owner for Python public source-shape validation.
- The root `easynet_sdk` namespace must expose only canonical `__all__` symbols
  plus explicit package metadata.
- The strict Python type contract must be part of the same CI gate as the SDK
  package it verifies.
- The canonical public API gate must exercise the duplicate-member negative
  fixture in self-test mode.
- Runtime behavior and exported public symbols remain unchanged.
