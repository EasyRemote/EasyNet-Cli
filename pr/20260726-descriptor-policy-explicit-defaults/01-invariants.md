Invariants
==========

- Visibility is an access-control fact, not a fallback value.
- Scope rule is an access-control fact, not a fallback value.
- Missing policy facts must not be silently projected as `Private` or `None`.
- Authoring paths may still explicitly choose conservative policy states.
