# Identity C ABI transport completion intent

Make the Python C ABI identity transport match the IdentityTransport protocol without inventing identity semantics in Python.

This slice connects local ResourceRef projection to the existing C ABI Publication resource-ref projector and converts signing-key/signer methods that lack lower C ABI contracts into explicit typed SDK errors.
