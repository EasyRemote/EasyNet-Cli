# Invariants

1. Input is admitted only after explicit input-control consent and the current session policy.
2. Window/application input requires a fresh host target proof before every OS call.
3. Pointer coordinates consume the committed target geometry revision and are clamped to the selected surface.
4. OS injection is synchronous and bounded; one input frame cannot expand into an unbounded event batch.
5. Platform denial is typed and projected through the existing session input-block state.
6. Windows uses `SendInput`; Linux X11 uses XTest; unsupported Wayland environments fail closed.
7. Source/compile proof is baseline evidence, not live cross-platform product certification.
