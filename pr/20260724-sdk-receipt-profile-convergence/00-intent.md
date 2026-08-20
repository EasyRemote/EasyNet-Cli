# Intent

Close the cross-language SDK receipt proof-fact profile seam.

Node, Swift, and Java currently validate receipt entity profiles with a local
string whitelist that admits `axon-legacy-v1` and `opaque`. Go and Python route
the same receipt facts through Axon's canonical profile parser. The local
whitelists create a language-specific runtime model and let receipt
canonicalization accept retired proof-fact identity profiles.

This iteration converges Node, Swift, and Java onto the same canonical runtime
profile contract: receipt identity profiles are `axon-strict-v2` only.
