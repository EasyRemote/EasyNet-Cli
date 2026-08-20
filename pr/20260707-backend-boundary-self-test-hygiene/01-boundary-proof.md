# Backend Boundary Self-Test Hygiene Boundary Proof

The backend SDK-only boundary scanner is part of the aggregate SDK cutover gate.
Its self-test must be deterministic and isolated because it asserts lower-layer
deletion boundaries for the EasyNet backend.

Moving the expected-failure capture file under the existing temporary fixture
directory removes a shared filesystem side effect without changing scanner
semantics. The gate still rejects raw Axon imports, generated protocol imports,
direct daemon transport packages, raw socket markers, and runtime subprocess
paths.
