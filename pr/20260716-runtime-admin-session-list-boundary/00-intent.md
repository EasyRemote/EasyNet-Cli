## Intent

Close the runtime-admin session-list response fork where SDK facades accepted a
legacy `items` array or silently projected malformed/missing daemon output as an
empty `RuntimeSessionPage`.

Expected effect: architecture convergence. The runtime-admin ability owns the
session-list response schema, and SDK facades must consume the canonical
`sessions` array rather than manufacturing a valid page from absent or retired
fields.
