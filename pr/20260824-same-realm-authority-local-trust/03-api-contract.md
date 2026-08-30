# API Contract

No public API changes.

For an inbound canonical invocation whose caller is
`easynet:///r/<local-realm>/authority`, admission uses
`caller_signature.key_id_hint` as the exact public-key selector against the
local realm trust anchor. Absence or mismatch produces a typed fail-closed
diagnostic and performs no Hub request.
