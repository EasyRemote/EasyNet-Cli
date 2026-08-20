# Intent

Add a generic SDK facade for descriptor-bound resource subjects.

Backend descriptor-bound signing currently reaches into the Axon Go SDK only to
build a resource-shaped subject URA. The daemon SDK should own this reusable
runtime projection so downstream products do not import raw Axon helpers. The
facade delegates to Identity `build_ura(kind=resource)` rather than reimplementing
URA grammar in language bindings.
