# Intent

Close the hosted Agent authority lease generation fork.

Hot enroll, rollback, and revoke share one mutable authority inventory. The
generation and incarnation counters are lifecycle facts for that inventory, so
they must be advanced by the inventory state owner and fail closed on overflow
instead of wrapping into a stale lease or rollback token.
