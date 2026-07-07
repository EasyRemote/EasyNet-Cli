# SDK Package Metadata Gate

## Intent

Add a machine-checked package metadata gate for shipped SDK language packages.
The gate validates package identity and current capability state without
claiming release stability, publishing readiness, provider-backed status for P1
languages, or product cutover readiness.

## Scope

- Validate Go, Python, Node, Java, and Swift SDK package manifests.
- Keep P1 Node, Java, and Swift packages marked as seam-only packages.
- Wire the gate into scaffold and aggregate cutover readiness checks.
- Update SDK status text so package metadata evidence is distinct from package
  stability and release evidence.

## Non-Scope

- No package publishing.
- No provider-backed Node, Java, or Swift transport claim.
- No backend or EasyRemote product cutover claim.
