# Execution Checklist

- [x] Add Rust Admin + Gateway revoke carrier and device-admin projection.
- [x] Export the carrier/projection through `include/easynet_cli.h` and C ABI.
- [x] Bind Go C ABI transport to execute `RevokeDevice`.
- [x] Bind Python C ABI transport to execute `revoke_device`.
- [x] Update tests, parity notes, and run targeted verification.
