# Invariants — RemoteApp Input Control Support Matrix

1. Input support is runtime/product metadata, not an invocation tuple field.
2. Pointer/keyboard control requires explicit input-control consent and platform
   permission.
3. macOS display input may be available only when the platform permission check
   is true.
4. macOS window/application input remains unsupported until target-scoped focus
   and dispatch proofs exist.
5. Linux and Windows input injection remain unsupported until native platform
   injection backends exist.
6. Unsupported rows must carry stable reasons for frontend and E2E assertions.
