# Intent

Remove tracked Python bytecode from the canonical SDK tree.

Generated interpreter artifacts are not runtime model source, provider source,
or conformance evidence. Keeping them tracked makes the SDK public surface and
provider boundaries depend on local Python execution state.
