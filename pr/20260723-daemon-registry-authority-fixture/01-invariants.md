# Invariants

1. Production daemon boot may still resolve authority from the local
   environment through the production constructor.
2. Tests that provide an authority fixture must not read ambient local
   credentials before installing that fixture.
3. No production fallback signer, fallback device, or synthetic credentials are
   introduced.
4. Real-invoke skill coverage remains real dispatcher execution; only authority
   assembly becomes hermetic.
5. Registry assembly continues to require an explicit `LocalRuntime`.
