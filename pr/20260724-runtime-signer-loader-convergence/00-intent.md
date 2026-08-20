# Intent

Converge runtime signer loading on the canonical caller-signer resolver.

The codebase now has a generic runtime caller signer port that distinguishes managed User custody from runtime-owner custody. Some boot/product helper paths still bind `RuntimeSigningIdentity::load_default` directly, which preserves a second signer-loading idiom outside the canonical resolver.

This iteration removes those direct product/boot calls and routes them through `load_runtime_caller_signer`.

