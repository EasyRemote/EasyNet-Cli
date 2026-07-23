# Intent

Remove product-shaped vocabulary from the Go SDK root resource namespace
projection.

The root SDK owns canonical runtime concepts. Its resource namespace allowlist
is a generic runtime model boundary, not an EasyNet product namespace adapter.
Provider-specific translation must remain under provider packages.
