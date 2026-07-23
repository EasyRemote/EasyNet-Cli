# Intent

Remove SDK-side compatibility parsing for daemon `control.json`.

The daemon discovery file is a local attach contract. Once present, it must be
strictly decoded: unknown fields are malformed, lifecycle/version facts are
mandatory, and listener ports are explicit positive values. SDKs must not
repair old discovery shapes into a runtime-ready projection.
