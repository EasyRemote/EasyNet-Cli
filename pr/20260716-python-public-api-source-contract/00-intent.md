# Intent

Converge the Python SDK public API contract around one source owner:
`sdk/conformance/public_api_inventory.py`.

The immediate defect class is duplicate public DTO members. Python accepts the
last declaration, while the inventory's set-based member graph can hide the
source duplication. The public API inventory must reject that state before the
canonical manifest hashes are trusted.

