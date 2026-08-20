# Invariants

- The project has URA only; Hub naming must not survive as a public Authority
  ability selector.
- Descriptor-wire helpers must not translate product-era Authority aliases into
  canonical runtime facts.
- Device and Agent local registry keys remain implementation dispatch concerns,
  not canonical public selectors.
- Descriptor refs remain the preferred public wire form.
