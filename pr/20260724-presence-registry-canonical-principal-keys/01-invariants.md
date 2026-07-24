# Invariants

1. Presence liveness state is keyed only by canonical principal URAs.
2. Invalid or retired identity strings fail before mutating the presence map.
3. Read models consume already-valid liveness state; they do not repair malformed presence rows.
4. Device directory projection may filter canonical non-device principals, but must never be responsible for rejecting malformed keys.
5. Session displacement and offline/online ordering remain unchanged for valid keys.
