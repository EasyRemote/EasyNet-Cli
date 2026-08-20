# Invariants

1. `AbilityDescriptor` is the sole owner of descriptor identity derivation.
2. FFI catalog projection may serialize descriptor facts, but must not recompute canonical descriptor refs independently.
3. Descriptor hash validation remains strict: descriptor hashes are still sha256-prefixed canonical hex values.
4. Runtime descriptor resolution remains bounded to local/realm catalog lookup and must not introduce remote probe fallback.
5. Public behavior is unchanged: descriptor catalog entries still include `name`, `owner_ura`, `ability_ura`, `descriptor_ref`, `version`, `descriptor_hash`, `call_mode`, and `admission_action`.
