# Invariants

- Permission-free realtime capabilities remain `not_required`.
- Declared permissions with a status ability remain `status_ability_available`.
- Declared permissions with a request ability remain `request_ability_available`.
- Declared permissions without either action path must be explicit and actionable, not `unknown`.
- The activation report remains serialize-only daemon output; CLI projection must not deserialize internal Rust DTOs.
