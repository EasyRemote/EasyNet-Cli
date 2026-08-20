# API Contract

Unchanged:

- `sdk/node/index.js` exports.
- `sdk/node/index.d.ts` declarations.
- Node runtime behavior and tests.
- Existing provider source files.

Changed:

- `sdk/node/package.json.name` becomes `@runtime/sdk`.
- `sdk/node/package-lock.json` root package names are updated to match.
- `tools/scripts/check-sdk-package-metadata.sh` now enforces the neutral package
  name and self-test mutation.

Because the package is private, this is metadata convergence, not a published
package rename.
