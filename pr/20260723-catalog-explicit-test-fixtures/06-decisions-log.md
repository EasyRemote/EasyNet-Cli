# Decisions Log

- Decision: place explicit test fixture constructors on `AxonAbilityCatalog`.
  Rationale: the catalog owns authority context and runtime attachment; ability
  modules should only declare the Device authority they are testing under.
- Decision: do not add a runtime-backed helper in this slice.
  Rationale: no current caller needs it; adding unused fixture surface would be
  capability accumulation instead of convergence.
