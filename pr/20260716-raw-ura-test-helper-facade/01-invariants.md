# Invariants

1. CLI modules must not hand-build URAs with `format!("easynet:///r/...")`
   outside `src/core/ura/mod.rs`.
2. Inline test modules inside production files are still scanned by the guard;
   they must use the same facade builders as production code.
3. Test fixture address shapes must remain equivalent after migration.
4. No exemption is added to the guard for fixable call sites.
