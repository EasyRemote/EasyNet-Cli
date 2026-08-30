# Invariants

1. Frontend checks must require a host-local `permission_status` call with
   `args: {}` and no `subjectURA`.
2. Product-flow checks must require a visible `Check permissions` action in the
   share picker.
3. Store tests must prove permission_status output includes input/accessibility
   permission state.
4. The product audit must keep RemoteApp incomplete until real E2E evidence
   exists.
