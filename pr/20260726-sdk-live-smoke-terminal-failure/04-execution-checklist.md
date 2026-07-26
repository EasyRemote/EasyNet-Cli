# Execution Checklist

- [x] Locate C ABI/daemon conversion that emits or rejects receipt-free invoke responses.
- [x] Identify whether admission denial should be converted in daemon service or C ABI projection.
- [x] Implement one canonical conversion point.
- [x] Add focused regression test for admission-denied receipt-free response shape.
- [x] Bind paired Device owner facts at boot through RuntimeTrust instead of policy-gate credential fallback.
- [x] Make stream terminal-drained close owner-authorized and idempotent at the C ABI resource layer.
- [x] Run Go/Python live smoke or focused equivalent.
- [x] Run static/gate verification.
