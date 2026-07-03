# Runtime Lifecycle Invariants

1. Process facts are authoritative for local daemon lifecycle.
2. Projection absence never implies daemon absence.
3. Projection presence never implies daemon liveness.
4. Attach requires matching requested mode, realm, and device node id.
5. Control-ready without Invocation-ready is degraded and non-attachable.
6. Projection commit failure after a fresh spawn rolls back that spawned
   daemon; attach paths do not kill a pre-existing daemon.
7. PID-based signaling must reject PID reuse before sending a signal.
8. Stop must not remove projection as a success claim when daemon stop timed
   out and facts remain live.
9. Legacy Axon bridge and retired heartbeat pid cleanup are compatibility
   janitors, not current product daemon lifecycle states.
10. Product presence status is a separate observer and is not derived from
    local daemon process facts.
