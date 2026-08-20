# Architecture

The consumer boundary auditor is part of the SDK architecture contract. If its
rules are daemon-named, downstream products inherit a product daemon mental
model even when the SDK runtime model is generic. This iteration preserves the
actual boundary but renames the model and emitted violation rule to
runtime-host terminology.
