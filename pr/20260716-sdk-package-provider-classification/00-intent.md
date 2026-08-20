# Intent

Close the SDK package-manifest ownership fork.

The canonical SDK model must stay product-neutral. Existing Java, Python, and
Swift package names are public compatibility surfaces, not proof that
EasyNet/daemon-branded package roots are canonical public facades. This slice
keeps public behavior unchanged and moves that classification into the
conformance source of truth.
