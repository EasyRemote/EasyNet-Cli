# Java pluginexec JSON array projection parity

## Goal

Converge the Java pluginexec sidecar helper with the canonical runtime JSON value model used by the other SDK provider helpers. A Java handler returning a native Java array must project to a canonical JSON array without forcing product templates to hand-shape results as `List`.

## Invariants

- SDK helpers expose generic runtime/provider concepts only.
- The helper owns canonical frame projection; product templates must not manually serialize sidecar response JSON.
- URA terminology remains canonical.
- Java native arrays are deterministic ordered values and may lower to canonical JSON arrays.
- Generic `Iterable`/`Set` support is intentionally excluded because it can introduce non-deterministic ordering or side-effecting iteration into receipt-visible response projection.
- Existing public helper behavior remains compatible.

## Boundary proof

- Scope is limited to `run.runtime.sdk.provider.runtime.pluginexec`.
- No EasyNet/EasyRemote product naming or lifecycle enters the SDK helper.
- The conformance gate owns the evidence that Java has the same provider-helper capability as the other language helpers.

## Refactoring plan

1. Add native Java array serialization inside the Java JSON frame codec.
2. Cover primitive and object arrays through `java.lang.reflect.Array`.
3. Add a provider-helper test proving arrays are emitted as JSON arrays from `SidecarRuntime.serve`.
4. Add SPEC v2 gate tokens so future Java helper regressions fail before product tests.

## Verification

- Compile/run the Java helper test directly.
- Run SDK/product-neutrality and canonical-runtime-convergence gates.
- Run repository formatting/diff checks.
- Use codegraph after implementation to verify the new parity surface.
