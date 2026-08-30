# Architecture

Layering:

1. Plugin manifest owns the package-declared bidi wire representation.
2. RemoteApp compiled registration must match the package declaration.
3. Ability wire registry projects plugin declarations into daemon dispatch adapters.
4. The dispatcher continues to execute through the existing adapter that already handles JSON control and binary chunks.
5. Product closure audit pins the RemoteApp-specific declaration so future work cannot regress the public contract to JSON-only.

This is a contract/metadata correction. It aligns the advertised data-plane shape with the existing implementation without touching the concurrent dispatcher refactor.
