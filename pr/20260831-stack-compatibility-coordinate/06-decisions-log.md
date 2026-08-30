# Decisions log

## 2026-08-31 — Axon contract lands before the CLI lock

The CLI lock must bind an immutable upstream contract, so Axon contract revision `bf94445547fe50cb76f891bb53c748b8af2c815d` and contract SHA-256 `600e29eb39a47f66eecfba4fa455e64ab4f2421740cc18a32a2a1abdf64254e6` are the first candidate inputs.

## 2026-08-31 — A manifest cannot certify an incompatible candidate

The initial CLI full lib suite had 117 failures against Axon `0.192.3`. The failures shared stale fixture assumptions about Device-owned abilities and untyped output. The Runtime now exercises registry-declared, device-sponsored SystemAgent ownership and typed output; the final suite has zero failures, so the lock may advance.

## 2026-08-31 — Node remains a private seam package

The Node SDK package is not an independently released coordinate. Its package version remains `0.0.0-seam`; Runtime release identity is carried by the compatibility lock and generated public API facts instead of overloading npm package metadata.
