# Invariants

1. Principal lifecycle remains product-neutral: no account, EasyNet, EasyRemote, directory, or private-key custody fields.
2. There is one public principal lifecycle interface per SDK language.
3. Provider-backed implementation is represented by `RuntimePrincipalProvider`, not by a second public lifecycle interface.
4. Public transition operations stay unchanged: create, bind/add/rotate/revoke key, recovery, suspend/reactivate/delete, enrollment, grant, get.
5. No compatibility alias is kept after migration.
