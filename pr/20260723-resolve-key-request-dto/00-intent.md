# Intent

Move outbound `federation.resolve_key` request construction out of the
admission resolver and into the typed federation wrapper DTO.

The root defect is not the field name itself; it is that admission builds a
protocol request with ad-hoc JSON, so legacy shape vocabulary can remain in the
signing path without a single owner.
