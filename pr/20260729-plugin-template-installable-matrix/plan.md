# Plugin template installable matrix

## Intent

Keep `easynet plugin init` product-safe as the template surface expands beyond
Python. Every generated exec template that claims provider-backed helper support
must also produce a package root whose manifest and ability descriptors are
accepted by the daemon plugin package parser.

## Invariants

- Template availability is derived from the provider sidecar helper capability
  matrix, not from language-specific ad hoc branches.
- Generated plugin code must use the SDK provider helper for its language and
  must not hand-write sidecar JSON frames.
- Compiled-language templates may require an explicit build before invocation,
  but their generated package metadata must still parse before build.
- Daemon install/reload remains the authority for package collisions and
  executable readiness; templates must not add compatibility bypasses.

## Verification

- Add a Rust unit test that generates every template-open language and parses
  each generated root with `PluginPackage::from_installed`.
- Add a SPEC v2 gate assertion requiring that test so template/package coverage
  cannot silently regress.
- Run targeted plugin template tests, `cargo fmt --check`, SPEC v2 gate, and the
  architecture convergence gate.
