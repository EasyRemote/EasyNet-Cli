# Node/TypeScript Runtime SDK

The Node package is a generic Runtime Core seam for desktop tools and extension
hosts. Its JavaScript and TypeScript public model is limited to:

- feature discovery and explicit client lifecycle;
- typed SDK errors and stable error classes;
- runtime health and diagnostics;
- complete Invocation tuple construction;
- prepare, caller-sign, submit, result, cancellation, event, and handle state;
- delegated and session authority metadata;
- bounded stream and bidirectional-session state machines with async iteration
  and `AbortSignal` cancellation.

Invocation results preserve runtime-provided receipt facts as opaque objects. The
package does not expose receipt history or interpret product receipt policy.

Downstream workflow profiles are deliberately absent. Product administration,
gateway, application lifecycle, compatibility adapters, directory views,
identity projections, event feeds, host binding, orchestration, publication,
receipt-history pages, page/model/file helpers, and wrapper behavior belong to
downstream products. `index.js` and `index.d.ts` provide no aliases or empty
transport placeholders for those surfaces.

The package currently has no bundled runtime-host transport or C ABI provider.
`tools/scripts/check-node-sdk-seam.sh` runs the Runtime Core tests, validates the
JavaScript module, and checks the source boundary. The test suite also enforces
exact JavaScript exports and matching TypeScript declaration symbols.
