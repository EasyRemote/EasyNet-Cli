# Invariants

- Public exported Go API names remain unchanged for this slice.
- `IsResourceNamespace`, `ResourceURA`, and URA parsing behavior remain stable.
- Provider-specific EasyNet translation stays outside the SDK root.
- No new compatibility aliases are introduced.
- Go and Python SDKs continue to model resources as runtime concepts, not
  product directory concepts.
