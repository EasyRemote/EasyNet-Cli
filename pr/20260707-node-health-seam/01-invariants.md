# Invariants

1. Health is a generic daemon SDK profile, not an EasyNet backend or EasyRemote
   status model.
2. `apiReady && daemonReady` means API liveness; `runtimeReady` means runtime
   readiness. The seam must not collapse those states.
3. Diagnostics are typed SDK DTOs with `profile: "health"` and
   `kind: "diagnostics_report"`.
4. The Node facade accepts only an injected transport and must not discover,
   start, or supervise daemon processes.
5. Transport failures are typed SDK errors; malformed health payloads are decode
   errors. Consumers must not parse human error strings.
6. No legacy aliases are accepted in public input or decoded DTO fields.
