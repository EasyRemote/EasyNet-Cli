# Decisions Log

## 2026-07-24

- Decision: migrate pluginexec helpers to `provider/runtime` rather than adding aliases.
- Reason: compatibility aliases under `provider/easynet` would preserve the product-specific SDK architecture defect.
- Decision: migrate Python plugin_exec with the other language helpers in the same iteration.
- Reason: leaving Python at `providers.easynet.plugin_exec` would keep the generated Python template product-specific while the other languages converged.
- Decision: keep Go lifecycle/identity provider paths out of this commit.
- Reason: lifecycle/identity have separate public provider semantics and need their own cutover plan; mixing them with pluginexec would create a broad, less auditable change.
- Decision: update generated templates to use canonical sidecar tuple fields (`caller_ura`, `callee_ura`, `ability_ura`, `subject_ura`) from helper DTOs.
- Reason: helper-backed templates must compile/run against the SDK helper API instead of relying on retired short tuple aliases.
