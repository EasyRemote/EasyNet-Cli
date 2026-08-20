# Execution Checklist

- [x] Add Go SDK transport-backed resource subject facade and tests.
- [x] Add Python SDK projection function and tests.
- [x] Export Python public API.
- [x] Verify SDK import boundary remains clean.
- [ ] Migrate backend descriptor subject helper after backend can inject or
      construct an `IdentityClient` instead of calling a package-level string
      builder.
