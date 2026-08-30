# Invariants

1. A selected application/window Resource remains the session subject.
2. Capture must not silently fall back to a display for unsupported targets.
3. Platform capture lives behind the RemoteApp plugin provider boundary.
4. Capture queues, retries, and target observation are bounded.
5. Loss, permission denial, and unsupported platforms produce typed terminal or recoverable state.
6. Product completion requires live OS evidence; source gates are not substitutes.
