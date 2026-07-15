Invariants

1. `build_system_registry()` remains the complete system descriptor inventory
   and may retain the `session.open` Device carrier template.
2. Admission test fixtures that need mixed Hub and `session.open` descriptors
   may use `build_system_registry()`, but must not install `session.open`
   execution rows.
3. No production dispatcher, admission policy, descriptor binding, authority
   scope, or session carrier behavior changes in this slice.
4. `control_plane_authority_root` must continue to fail closed when a caller
   asks for a name-only key across multiple authority roots.
5. The repaired tests must prove PrincipalLifecycle suspended/deleted users are
   denied even when their key remains trusted.
