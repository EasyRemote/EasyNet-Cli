API contract
============

Public behavior
---------------

- Federation receipts keep accepting the same JSON field names.
- `meta.list_abilities(scope="realm")` continues to return the same descriptor
  rows.
- The public catalogue source remains `authority:broadcast`.

Internal contract
-----------------

- New Rust callers must use `authority_published_abilities::AuthorityPublishedAbilityStore`.
- `HubPublishedAbilityStore` is retired and must not be reintroduced.
