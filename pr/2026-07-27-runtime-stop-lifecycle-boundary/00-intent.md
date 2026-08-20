# Runtime stop lifecycle boundary

Move OS-facing daemon stop transitions out of the CLI command layer and into
`daemon::boot::lifecycle::stop`, leaving CLI stop as a renderer over typed
lifecycle outcomes.
