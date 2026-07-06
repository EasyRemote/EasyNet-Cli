# Boundary Proof

Publication profile responsibilities:

- build and project ability package/catalogue records;
- ask Directory + Identity to validate or project URA values;
- filter catalogue rows using explicit metadata or projected identity
  components.

Directory + Identity responsibilities:

- parse URA values;
- expose owner kind and owner ids as projection components;
- remain the single grammar owner for URA layout.

The correct dependency flow is:

```text
PublicationCatalogRecord.owner_ura
  -> Publication addressing facade parse_ura()
  -> IdentityProjection.components["user_id"]
  -> catalogue user filter
```

This removes a hidden dependency on `/agent/<user>.<agent>` and `/user/<user>`
text layout from the Publication profile.
