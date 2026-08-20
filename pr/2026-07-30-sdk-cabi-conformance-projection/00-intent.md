Intent
======

Close the SDK conformance drift where the Go C ABI runtime provider emits
runtime projection JSON that the canonical Go SDK parsers reject.

The fix must preserve the SDK as the canonical runtime model. The provider
must adapt to the canonical SDK stream/bidi DTOs; the SDK parsers must not be
weakened to accept provider-only or legacy fields.

