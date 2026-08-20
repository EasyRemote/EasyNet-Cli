# Execution Checklist

- [x] Add `RuntimeAdmissionPlane` value object.
- [x] Replace `DaemonInvocationService.admission` with `admission_plane`.
- [x] Migrate service callsites and tests.
- [x] Update architecture gates to reject the retired legacy wording/raw field.
- [x] Run targeted tests and convergence gates.
- [ ] Commit with the required author.
