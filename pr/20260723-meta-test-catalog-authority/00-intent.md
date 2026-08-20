# Intent

Remove ambient Device-authority construction from governance metadata tests.

`governance/meta.rs` contains a dense cluster of catalogue tests that construct
`AxonAbilityCatalog::new()`. In `cfg(test)`, that default constructor still
resolves local Device authority from ambient credentials. Metadata tests should
instead install an explicit authority fixture because they are asserting
catalogue semantics, not host pairing state.
