# API Contract

## Public CLI

Public CLI calls continue to use short agent selectors such as:

```text
easynet agent publish alice --dry-run
```

## Durable registry

The persisted registry must use canonical keys:

```json
{
  "agents": {
    "default/alice": {
      "schema_version": 2
    }
  }
}
```

Bare keys such as `"alice"` are invalid at persistence boundaries.
