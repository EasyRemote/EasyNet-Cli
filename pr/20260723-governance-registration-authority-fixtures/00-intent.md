# Intent

Remove ambient catalog construction from small governance registration tests.

`access_control`, `network_health`, and `admin_status` registration tests only
assert ability registration. They should not depend on process-local daemon
identity state through `AxonAbilityCatalog::new()`.
