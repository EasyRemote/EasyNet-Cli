# Invariants

1. Cross-device smoke reports must record the EasyNet-Cli source revision.
2. Cross-device smoke reports must record whether the working tree was dirty at run time.
3. Cross-device smoke reports must record the runtime image name, image id, created timestamp, and whether `--build` was requested.
4. Provenance does not turn a failed or skipped smoke into product evidence.
