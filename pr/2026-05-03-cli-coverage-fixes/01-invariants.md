# Invariants

1. `device join` must remain non-destructive: join succeeds even if no runtime
   is running, and any post-join bootstrap is best-effort only.
2. `fleet.describe_node` must keep the local fast path for unpaired devices,
   but when paired it may search cross-tenant federation state for a concrete
   `node_id`.
3. Cross-tenant lookup must stay read-only: resolve directory entries, then
   probe `observe.health`; no mutation side effects during `device show` or
   `auth abilities`.
4. `ability exec` must preserve argv semantics. The command remains structured
   execution, not shell interpolation.
5. Failure degradation must stay actionable:
   - backend 404 on `auth abilities` should fall back when federation data is
     available;
   - if fallback also fails, the user must still see the original HTTP failure
     plus the fallback miss.
