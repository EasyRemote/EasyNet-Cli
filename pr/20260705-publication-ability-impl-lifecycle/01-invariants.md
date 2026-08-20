# Invariants

- Publication lifecycle mutations must be complete daemon system-ability Invocations.
- `ability_ura` must be validated by the daemon contract as an Ability URA.
- `impl_id` is an implementation binding identifier, not a descriptor ref, path, or product function name.
- Enable and disable return typed Publication mutation records only after daemon output confirms success.
- Python facades may serialize request DTOs and map errors, but must not decide lifecycle state from catalogue rows.
- Runtime errors must surface as SDK typed errors without falling back to legacy transports.
- All resources opened by the environment/profile transports must remain closeable and deterministic.
