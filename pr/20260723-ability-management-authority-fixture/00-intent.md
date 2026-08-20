# Intent

Remove ambient catalog authority construction from ability management tests.

`ability_management.*` abilities are Device-hosted catalog/control-plane
surfaces. Their test fixtures must declare whether they need metadata-only
registration or executable LocalRuntime behavior, and must bind either fixture
to an explicit Device authority root.
