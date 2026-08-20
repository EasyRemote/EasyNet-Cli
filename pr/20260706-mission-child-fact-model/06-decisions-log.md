Decisions:
- Keep schema shape stable; enforce projection semantics in SDK parsers/conformance rather than editing the normative SPEC.
- Keep public error reason stable as `mission_child_invocation_mismatch`.
- Do not introduce a product-specific EasyRemote Pipeline abstraction in SDK code.
- Do not implement daemon scheduler/retry/live stream cutover in this slice.
