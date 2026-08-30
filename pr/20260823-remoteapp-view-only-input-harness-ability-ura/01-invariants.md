# Invariants

1. Do not make the CLI accept short ability names where the command contract
   requires an Ability URA.
2. The E2E harness must call public `remote_desktop.attach` through the
   advertised full Ability URA.
3. The selected subject must remain the target Resource URA, not the callee or
   short ability name.
4. View-only app/window sessions must reject pointer/key diagnostic frames with
   explicit `input_scope_unsupported` evidence.
5. The fix is harness/interface correctness only; it does not claim OS input
   injection product completion.
