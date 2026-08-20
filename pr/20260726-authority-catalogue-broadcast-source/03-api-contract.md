API contract
============

Runtime catalogue projection
----------------------------

When a realm-scope descriptor row lacks a source, `meta.list_abilities` emits:

```json
{"source": "authority:broadcast"}
```

Rejected vocabulary
-------------------

`hub:broadcast` is retired for catalogue projection metadata.

Public behavior
---------------

The descriptor rows, ability URAs, descriptor refs, and filtering behavior are
unchanged.
